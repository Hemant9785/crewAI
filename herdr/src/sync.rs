//! Bounded, event-based synchronization over an authenticated peer channel.
//!
//! This module only handles team records. It never reads repository files and
//! it requires the storage receive path to verify the event signature and
//! active membership before projection.

use crate::team::Event;
use crate::team_storage::{StorageError, TeamStore};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt;

pub(crate) const MAX_SYNC_EVENTS: usize = 256;
pub(crate) const MAX_SYNC_MESSAGE_BYTES: usize = 512 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) enum SyncMessage {
    Inventory {
        team_id: String,
        event_ids: Vec<String>,
    },
    Events {
        team_id: String,
        events: Vec<Event>,
    },
}

impl SyncMessage {
    pub(crate) fn encode(&self) -> Result<Vec<u8>, SyncError> {
        self.validate()?;
        let encoded = serde_json::to_vec(self).map_err(|_| SyncError::MalformedMessage)?;
        if encoded.len() > MAX_SYNC_MESSAGE_BYTES {
            return Err(SyncError::MessageTooLarge);
        }
        Ok(encoded)
    }

    pub(crate) fn decode(encoded: &[u8]) -> Result<Self, SyncError> {
        if encoded.len() > MAX_SYNC_MESSAGE_BYTES {
            return Err(SyncError::MessageTooLarge);
        }
        let message: Self =
            serde_json::from_slice(encoded).map_err(|_| SyncError::MalformedMessage)?;
        message.validate()?;
        Ok(message)
    }

    fn validate(&self) -> Result<(), SyncError> {
        match self {
            Self::Inventory { team_id, event_ids } => {
                validate_team_id(team_id)?;
                if event_ids.len() > MAX_SYNC_EVENTS {
                    return Err(SyncError::TooManyEvents(event_ids.len()));
                }
                if event_ids.iter().any(|event_id| event_id.trim().is_empty()) {
                    return Err(SyncError::MalformedMessage);
                }
            }
            Self::Events { team_id, events } => {
                validate_team_id(team_id)?;
                if events.len() > MAX_SYNC_EVENTS {
                    return Err(SyncError::TooManyEvents(events.len()));
                }
                for event in events {
                    event
                        .validate()
                        .map_err(|_| SyncError::InvalidEvent(event.event_id.clone()))?;
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct SyncApplyReport {
    pub inserted: usize,
    pub duplicates: usize,
}

pub(crate) fn inventory(store: &TeamStore, team_id: &str) -> Result<SyncMessage, SyncError> {
    let event_ids = store.event_ids(MAX_SYNC_EVENTS)?;
    let message = SyncMessage::Inventory {
        team_id: team_id.to_string(),
        event_ids,
    };
    message.validate()?;
    Ok(message)
}

pub(crate) fn missing_events_for_inventory(
    store: &TeamStore,
    team_id: &str,
    remote: &SyncMessage,
) -> Result<SyncMessage, SyncError> {
    let SyncMessage::Inventory {
        team_id: remote_team_id,
        event_ids: remote_event_ids,
    } = remote
    else {
        return Err(SyncError::UnexpectedMessage);
    };
    if remote_team_id != team_id {
        return Err(SyncError::WrongTeam);
    }
    let local_event_ids = store.event_ids(MAX_SYNC_EVENTS)?;
    let remote_set = remote_event_ids.iter().collect::<HashSet<_>>();
    let missing_ids = local_event_ids
        .into_iter()
        .filter(|event_id| !remote_set.contains(event_id))
        .take(MAX_SYNC_EVENTS)
        .collect::<Vec<_>>();
    let events = store.events_by_ids(&missing_ids)?;
    let message = SyncMessage::Events {
        team_id: team_id.to_string(),
        events,
    };
    message.validate()?;
    Ok(message)
}

pub(crate) fn apply_remote_events(
    store: &mut TeamStore,
    team_id: &str,
    message: &SyncMessage,
    received_at: u64,
) -> Result<SyncApplyReport, SyncError> {
    let SyncMessage::Events {
        team_id: remote_team_id,
        events,
    } = message
    else {
        return Err(SyncError::UnexpectedMessage);
    };
    if remote_team_id != team_id {
        return Err(SyncError::WrongTeam);
    }
    message.validate()?;
    if events.iter().any(|event| event.team_id != team_id) {
        return Err(SyncError::WrongTeam);
    }

    // Stable logical ordering lets a status event arrive in the same batch as
    // its creation event without depending on wall-clock arrival order.
    let mut pending = events.clone();
    pending.sort_by(|left, right| {
        left.logical_time
            .cmp(&right.logical_time)
            .then_with(|| left.event_id.cmp(&right.event_id))
    });
    for event in &pending {
        store.validate_verified_event(event)?;
    }

    let mut report = SyncApplyReport::default();
    let mut deferred = Vec::new();
    while !pending.is_empty() {
        let mut progressed = false;
        deferred.clear();
        for event in pending.drain(..) {
            match store.insert_verified_event(&event, received_at) {
                Ok(true) => {
                    report.inserted += 1;
                    progressed = true;
                }
                Ok(false) => {
                    report.duplicates += 1;
                    progressed = true;
                }
                Err(StorageError::Sql(rusqlite::Error::QueryReturnedNoRows)) => {
                    deferred.push(event);
                }
                Err(error) => return Err(SyncError::Storage(error)),
            }
        }
        if !deferred.is_empty() && !progressed {
            return Err(SyncError::DependencyMissing(deferred[0].event_id.clone()));
        }
        pending = deferred.clone();
    }
    Ok(report)
}

fn validate_team_id(team_id: &str) -> Result<(), SyncError> {
    if team_id.trim().is_empty() {
        return Err(SyncError::MalformedMessage);
    }
    Ok(())
}

#[derive(Debug)]
pub(crate) enum SyncError {
    Storage(StorageError),
    MalformedMessage,
    MessageTooLarge,
    TooManyEvents(usize),
    InvalidEvent(String),
    UnexpectedMessage,
    WrongTeam,
    DependencyMissing(String),
}

impl fmt::Display for SyncError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Storage(error) => write!(formatter, "sync storage error: {error}"),
            Self::MalformedMessage => formatter.write_str("sync message is malformed"),
            Self::MessageTooLarge => formatter.write_str("sync message is too large"),
            Self::TooManyEvents(count) => {
                write!(formatter, "sync message contains too many events: {count}")
            }
            Self::InvalidEvent(event_id) => write!(formatter, "sync event is invalid: {event_id}"),
            Self::UnexpectedMessage => formatter.write_str("unexpected sync message type"),
            Self::WrongTeam => formatter.write_str("sync message belongs to another team"),
            Self::DependencyMissing(event_id) => {
                write!(formatter, "sync event dependency is missing: {event_id}")
            }
        }
    }
}

impl std::error::Error for SyncError {}

impl From<StorageError> for SyncError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::DeviceIdentity;
    use crate::team::{Device, EventType, Member, Role, TaskStatus};
    use serde_json::json;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn path(label: &str, extension: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "team-assistant-sync-{label}-{}-{nanos}.{extension}",
            std::process::id()
        ))
    }

    fn seed_store(database: &PathBuf, identity: &DeviceIdentity, member_id: &str) -> TeamStore {
        let store = TeamStore::open(database).unwrap();
        store.save_team(&crate::team::Team { team_id: "team-1".into(), name: "Sync test".into(), authority_member_id: "member-1".into(), created_at: 1 }).unwrap();
        for id in ["member-1", "member-2"] {
            store.save_member("team-1", &Member { member_id: id.into(), display_name: id.into(), role: Role::Implementer, active: true }, 1).unwrap();
        }
        let member = Member {
            member_id: member_id.into(),
            display_name: member_id.into(),
            role: Role::Implementer,
            active: true,
        };
        store.save_member("team-1", &member, 1).unwrap();
        store
            .save_device(
                &Device {
                    device_id: identity.device_id().into(),
                    member_id: member_id.into(),
                    public_key: identity.public_key(),
                    active: true,
                },
                1,
            )
            .unwrap();
        store
    }

    fn signed_event(
        identity: &DeviceIdentity,
        event_id: &str,
        event_type: EventType,
        payload: serde_json::Value,
        logical_time: u64,
    ) -> Event {
        let mut event = Event {
            event_id: event_id.into(),
            team_id: "team-1".into(),
            actor_member_id: "member-1".into(),
            device_id: identity.device_id().into(),
            event_type,
            payload,
            schema_version: crate::team::SUPPORTED_EVENT_SCHEMA_VERSION,
            logical_time,
            signature: None,
        };
        event.signature = Some(identity.sign(&event.signing_bytes()));
        event
    }

    #[test]
    fn two_profiles_converge_after_out_of_order_delivery_and_duplicate_retry() {
        let first_identity_path = path("first-identity", "json");
        let second_identity_path = path("second-identity", "json");
        let first_database = path("first", "sqlite");
        let second_database = path("second", "sqlite");
        let first_identity = DeviceIdentity::load_or_create(&first_identity_path).unwrap();
        let second_identity = DeviceIdentity::load_or_create(&second_identity_path).unwrap();
        let mut first_store = seed_store(&first_database, &first_identity, "member-1");
        let mut second_store = seed_store(&second_database, &second_identity, "member-2");
        second_store
            .save_member(
                "team-1",
                &Member {
                    member_id: "member-1".into(),
                    display_name: "member-1".into(),
                    role: Role::Implementer,
                    active: true,
                },
                1,
            )
            .unwrap();
        second_store
            .save_device(
                &Device {
                    device_id: first_identity.device_id().into(),
                    member_id: "member-1".into(),
                    public_key: first_identity.public_key(),
                    active: true,
                },
                1,
            )
            .unwrap();
        let created = signed_event(
            &first_identity,
            "event-created",
            EventType::TaskCreated,
            json!({
                "task_id": "task-1",
                "assigner_member_id": "member-1",
                "assignee_member_id": "member-2",
                "description": "sync task",
                "created_at": 1
            }),
            1,
        );
        let delivered = signed_event(
            &first_identity,
            "event-delivered",
            EventType::TaskStatusChanged,
            json!({
                "task_id": "task-1",
                "status": TaskStatus::Delivered,
                "changed_at": 2
            }),
            2,
        );
        let in_progress = signed_event(
            &first_identity,
            "event-in-progress",
            EventType::TaskStatusChanged,
            json!({
                "task_id": "task-1",
                "status": TaskStatus::InProgress,
                "changed_at": 3
            }),
            3,
        );
        let done = signed_event(
            &first_identity,
            "event-done",
            EventType::TaskStatusChanged,
            json!({
                "task_id": "task-1",
                "status": TaskStatus::Done,
                "changed_at": 4
            }),
            4,
        );
        first_store.insert_event(&created, 1).unwrap();
        first_store.insert_event(&delivered, 2).unwrap();
        first_store.insert_event(&in_progress, 3).unwrap();
        first_store.insert_event(&done, 4).unwrap();
        let source_events = first_store
            .events_by_ids(&[
                done.event_id.clone(),
                in_progress.event_id.clone(),
                delivered.event_id.clone(),
                created.event_id.clone(),
            ])
            .unwrap();
        let message = SyncMessage::Events {
            team_id: "team-1".into(),
            events: source_events.into_iter().rev().collect(),
        };
        let report = apply_remote_events(&mut second_store, "team-1", &message, 3).unwrap();
        assert_eq!(
            report,
            SyncApplyReport {
                inserted: 4,
                duplicates: 0
            }
        );
        assert_eq!(
            second_store.task("task-1").unwrap().unwrap().status,
            TaskStatus::Done
        );
        let retry = apply_remote_events(&mut second_store, "team-1", &message, 4).unwrap();
        assert_eq!(
            retry,
            SyncApplyReport {
                inserted: 0,
                duplicates: 4
            }
        );

        for path in [
            first_identity_path,
            second_identity_path,
            first_database,
            second_database,
        ] {
            let _ = fs::remove_file(path);
        }
    }

    #[test]
    fn invalid_signature_is_rejected_before_projection() {
        let identity_path = path("invalid-identity", "json");
        let database = path("invalid", "sqlite");
        let identity = DeviceIdentity::load_or_create(&identity_path).unwrap();
        let mut store = seed_store(&database, &identity, "member-1");
        let mut event = signed_event(
            &identity,
            "event-invalid",
            EventType::TaskCreated,
            json!({
                "task_id": "task-invalid",
                "assigner_member_id": "member-1",
                "assignee_member_id": "member-1",
                "description": "reject me",
                "created_at": 1
            }),
            1,
        );
        event.signature = Some("tampered".into());
        let result = apply_remote_events(
            &mut store,
            "team-1",
            &SyncMessage::Events {
                team_id: "team-1".into(),
                events: vec![event],
            },
            1,
        );
        assert!(matches!(
            result,
            Err(SyncError::Storage(StorageError::InvalidEventSignature))
        ));
        assert_eq!(store.task_count().unwrap(), 0);
        let _ = fs::remove_file(identity_path);
        let _ = fs::remove_file(database);
    }

    #[test]
    fn inventories_and_batches_are_bounded_and_team_scoped() {
        let message = SyncMessage::Inventory {
            team_id: "team-1".into(),
            event_ids: vec!["event-1".into()],
        };
        let decoded = SyncMessage::decode(&message.encode().unwrap()).unwrap();
        assert_eq!(decoded, message);
        let wrong = SyncMessage::Inventory {
            team_id: "team-2".into(),
            event_ids: Vec::new(),
        };
        assert!(matches!(
            missing_events_for_inventory(&TeamStore::open(":memory:").unwrap(), "team-1", &wrong,),
            Err(SyncError::WrongTeam)
        ));
    }
}
