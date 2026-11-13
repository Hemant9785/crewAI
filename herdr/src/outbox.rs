//! Persistent offline delivery queue and execution-time authorization.

use crate::team::{
    DeliveryState, OutboxRecord, OutboxState, PermissionAction, PermissionResource, RequestKind,
};
use crate::team_storage::{StorageError, TeamStore, MAX_OUTBOX_ATTEMPTS};
use std::fmt;

pub(crate) fn queue_event(
    store: &TeamStore,
    event_id: &str,
    next_attempt_at: u64,
) -> Result<(), QueueError> {
    if event_id.trim().is_empty() {
        return Err(QueueError::InvalidEventId);
    }
    store.enqueue_outbox(event_id, next_attempt_at)?;
    Ok(())
}

pub(crate) fn claim_ready(
    store: &mut TeamStore,
    now: u64,
    limit: usize,
) -> Result<Vec<OutboxRecord>, QueueError> {
    Ok(store.claim_outbox(now, limit)?)
}

pub(crate) fn acknowledge(store: &TeamStore, event_id: &str) -> Result<(), QueueError> {
    store.acknowledge_outbox(event_id)?;
    Ok(())
}

pub(crate) fn fail(
    store: &TeamStore,
    event_id: &str,
    retry_at: u64,
) -> Result<OutboxState, QueueError> {
    Ok(store.fail_outbox(event_id, retry_at)?)
}

pub(crate) fn cancel(store: &TeamStore, event_id: &str) -> Result<(), QueueError> {
    store.cancel_outbox(event_id)?;
    Ok(())
}

/// Re-check the permission needed by a queued request immediately before the
/// target process is started. Creation-time authorization is intentionally not
/// reused because grants may have expired or been revoked while offline.
pub(crate) fn authorize_request_execution(
    store: &TeamStore,
    team_id: &str,
    request_id: &str,
    resource_id: &str,
    now: u64,
) -> Result<(), QueueError> {
    let request = store
        .request(request_id)?
        .ok_or_else(|| QueueError::RequestNotFound(request_id.into()))?;
    if matches!(
        request.delivery,
        DeliveryState::Completed
            | DeliveryState::Cancelled
            | DeliveryState::Expired
            | DeliveryState::Failed
            | DeliveryState::Unauthorized
    ) {
        return Err(QueueError::NotRunnable(request.delivery));
    }
    let (resource, action) = match request.kind {
        RequestKind::Task => (
            PermissionResource::Task(resource_id.into()),
            PermissionAction::UpdateTask,
        ),
        RequestKind::Check | RequestKind::Read => (
            PermissionResource::Repository(resource_id.into()),
            PermissionAction::ReadRepository,
        ),
    };
    let decision = store.permission_decision(
        team_id,
        &request.requester_member_id,
        &resource,
        &action,
        now,
    )?;
    if !decision.allowed {
        return Err(QueueError::Unauthorized(request_id.into()));
    }
    Ok(())
}

pub(crate) const fn max_attempts() -> u32 {
    MAX_OUTBOX_ATTEMPTS
}

#[derive(Debug)]
pub(crate) enum QueueError {
    Storage(StorageError),
    InvalidEventId,
    RequestNotFound(String),
    Unauthorized(String),
    NotRunnable(DeliveryState),
}

impl fmt::Display for QueueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Storage(error) => write!(formatter, "outbox storage error: {error}"),
            Self::InvalidEventId => formatter.write_str("outbox event ID must not be empty"),
            Self::RequestNotFound(request_id) => {
                write!(formatter, "request not found: {request_id}")
            }
            Self::Unauthorized(request_id) => {
                write!(
                    formatter,
                    "request is unauthorized at execution time: {request_id}"
                )
            }
            Self::NotRunnable(state) => {
                write!(formatter, "request is not runnable in state {state:?}")
            }
        }
    }
}

impl std::error::Error for QueueError {}

impl From<StorageError> for QueueError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::DeviceIdentity;
    use crate::sync::{apply_remote_events, SyncMessage};
    use crate::team::{Device, Event, EventType, Member, PermissionGrant, Role};
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
            "team-assistant-outbox-{label}-{}-{nanos}.{extension}",
            std::process::id()
        ))
    }

    fn member(member_id: &str) -> Member {
        Member {
            member_id: member_id.into(),
            display_name: member_id.into(),
            role: Role::Implementer,
            active: true,
        }
    }

    #[test]
    fn offline_event_is_claimed_retried_and_acknowledged_without_duplicate_queue_rows() {
        let database = path("queue", "sqlite");
        let mut store = TeamStore::open(&database).unwrap();
        queue_event(&store, "event-1", 10).unwrap();
        queue_event(&store, "event-1", 20).unwrap();
        assert_eq!(store.outbox().unwrap().len(), 1);
        let claimed = claim_ready(&mut store, 10, 10).unwrap();
        assert_eq!(claimed[0].attempts, 1);
        assert_eq!(fail(&store, "event-1", 20).unwrap(), OutboxState::Queued);
        let claimed = claim_ready(&mut store, 20, 10).unwrap();
        assert_eq!(claimed[0].attempts, 2);
        acknowledge(&store, "event-1").unwrap();
        assert_eq!(store.outbox().unwrap()[0].state, OutboxState::Delivered);
        assert!(matches!(
            claim_ready(&mut store, 30, 10).unwrap().as_slice(),
            []
        ));
        let _ = fs::remove_file(database);
    }

    #[test]
    fn revoked_permission_blocks_a_queued_request_at_execution_time() {
        let identity_path = path("identity", "json");
        let database = path("revoked", "sqlite");
        let identity = DeviceIdentity::load_or_create(&identity_path).unwrap();
        let mut store = TeamStore::open(&database).unwrap();
        let team = crate::team::Team {
            team_id: "team-1".into(),
            name: "Team".into(),
            authority_member_id: "manager".into(),
            created_at: 1,
        };
        store.save_team(&team).unwrap();
        for member_id in ["manager", "requester", "owner"] {
            store.save_member("team-1", &member(member_id), 1).unwrap();
        }
        store
            .save_device(
                &Device {
                    device_id: identity.device_id().into(),
                    member_id: "requester".into(),
                    public_key: identity.public_key(),
                    active: true,
                },
                1,
            )
            .unwrap();
        let request_event = Event {
            event_id: "request-created".into(),
            team_id: "team-1".into(),
            actor_member_id: "requester".into(),
            device_id: identity.device_id().into(),
            event_type: EventType::RequestCreated,
            payload: json!({
                "request_id": "request-1",
                "requester_member_id": "requester",
                "target_member_id": "owner",
                "kind": "Check",
                "payload": "question",
                "created_at": 1
            }),
            schema_version: crate::team::SUPPORTED_EVENT_SCHEMA_VERSION,
            logical_time: 1,
            signature: Some("local-signature".into()),
        };
        store.insert_event(&request_event, 1).unwrap();
        store
            .grant_permission(
                "team-1",
                &PermissionGrant {
                    grant_id: "grant-read".into(),
                    subject_member_id: "requester".into(),
                    resource: PermissionResource::Repository("repo-1".into()),
                    action: PermissionAction::ReadRepository,
                    grantor_member_id: "manager".into(),
                    delegable: false,
                    expires_at: None,
                    revoked: false,
                },
                1,
            )
            .unwrap();
        authorize_request_execution(&store, "team-1", "request-1", "repo-1", 2).unwrap();
        store
            .revoke_permission("team-1", "grant-read", "manager", 3)
            .unwrap();
        assert!(matches!(
            authorize_request_execution(&store, "team-1", "request-1", "repo-1", 4),
            Err(QueueError::Unauthorized(request_id)) if request_id == "request-1"
        ));
        let _ = fs::remove_file(identity_path);
        let _ = fs::remove_file(database);
    }

    #[test]
    fn queued_request_appears_on_the_target_after_reconnect() {
        let source_identity_path = path("source-identity", "json");
        let target_identity_path = path("target-identity", "json");
        let source_database = path("source", "sqlite");
        let target_database = path("target", "sqlite");
        let source_identity = DeviceIdentity::load_or_create(&source_identity_path).unwrap();
        let target_identity = DeviceIdentity::load_or_create(&target_identity_path).unwrap();
        let mut source = TeamStore::open(&source_database).unwrap();
        let mut target = TeamStore::open(&target_database).unwrap();
        for store in [&source, &target] {
            store
                .save_member("team-1", &member("requester"), 1)
                .unwrap();
            store.save_member("team-1", &member("owner"), 1).unwrap();
        }
        source
            .save_device(
                &Device {
                    device_id: source_identity.device_id().into(),
                    member_id: "requester".into(),
                    public_key: source_identity.public_key(),
                    active: true,
                },
                1,
            )
            .unwrap();
        target
            .save_device(
                &Device {
                    device_id: source_identity.device_id().into(),
                    member_id: "requester".into(),
                    public_key: source_identity.public_key(),
                    active: true,
                },
                1,
            )
            .unwrap();
        target
            .save_device(
                &Device {
                    device_id: target_identity.device_id().into(),
                    member_id: "owner".into(),
                    public_key: target_identity.public_key(),
                    active: true,
                },
                1,
            )
            .unwrap();
        let mut event = Event {
            event_id: "offline-request".into(),
            team_id: "team-1".into(),
            actor_member_id: "requester".into(),
            device_id: source_identity.device_id().into(),
            event_type: EventType::RequestCreated,
            payload: json!({
                "request_id": "request-offline",
                "requester_member_id": "requester",
                "target_member_id": "owner",
                "kind": "Check",
                "payload": "offline question",
                "created_at": 1
            }),
            schema_version: crate::team::SUPPORTED_EVENT_SCHEMA_VERSION,
            logical_time: 1,
            signature: None,
        };
        event.signature = Some(source_identity.sign(&event.signing_bytes()));
        source.insert_event(&event, 1).unwrap();
        queue_event(&source, &event.event_id, 1).unwrap();
        let claimed = claim_ready(&mut source, 1, 1).unwrap();
        assert_eq!(claimed.len(), 1);
        let message = SyncMessage::Events {
            team_id: "team-1".into(),
            events: vec![event],
        };
        let report = apply_remote_events(&mut target, "team-1", &message, 2).unwrap();
        assert_eq!(report.inserted, 1);
        acknowledge(&source, "offline-request").unwrap();
        assert!(target.request("request-offline").unwrap().is_some());
        for path in [
            source_identity_path,
            target_identity_path,
            source_database,
            target_database,
        ] {
            let _ = fs::remove_file(path);
        }
    }
}
