//! Task policy applies at the common signed-event boundary, for local and remote writers.
use super::*;

impl TeamStore {
    pub(super) fn validate_task_event(
        &self,
        event: &Event,
        received_at: u64,
        remote: bool,
    ) -> Result<(), StorageError> {
        if !matches!(
            event.event_type,
            EventType::TaskCreated | EventType::TaskStatusChanged | EventType::TaskFollowUpAdded
        ) {
            return Ok(());
        }
        let denied = || StorageError::UnauthorizedEvent {
            actor_member_id: event.actor_member_id.clone(),
            device_id: event.device_id.clone(),
        };
        let authority = self.team_authority(&event.team_id)?;
        if event.event_type == EventType::TaskCreated {
            let payload: TaskCreatedPayload = serde_json::from_value(event.payload.clone())?;
            if payload.assigner_member_id != event.actor_member_id
                || !self.active_member_exists(&event.team_id, &payload.assignee_member_id)?
            {
                return Err(denied());
            }
            if authority != event.actor_member_id
                && !self
                    .permission_decision(
                        &event.team_id,
                        &event.actor_member_id,
                        &PermissionResource::Team(event.team_id.clone()),
                        &PermissionAction::CreateTask,
                        received_at,
                    )?
                    .allowed
            {
                return Err(denied());
            }
        } else {
            let task_id = event
                .payload
                .get("task_id")
                .and_then(Value::as_str)
                .ok_or(StorageError::MembershipConflict("task ID"))?;
            let task = self
                .task(task_id)?
                .ok_or(StorageError::Sql(rusqlite::Error::QueryReturnedNoRows))?;
            let origin: Option<String> = self.connection.query_row("SELECT team_id FROM event_log WHERE event_type='task.created' AND json_extract(payload_json,'$.task_id')=?1", params![task_id], |row| row.get(0)).optional()?;
            if origin.as_deref() != Some(event.team_id.as_str()) {
                return Err(denied());
            }
            if authority != event.actor_member_id
                && task.assignee_member_id != event.actor_member_id
                && !self
                    .permission_decision(
                        &event.team_id,
                        &event.actor_member_id,
                        &PermissionResource::Task(task_id.to_owned()),
                        &PermissionAction::UpdateTask,
                        received_at,
                    )?
                    .allowed
            {
                return Err(denied());
            }
            // Local commands must follow the visible workflow. Remote signed history
            // can be redundant or arrive after a concurrent later transition.
            if !remote && event.event_type == EventType::TaskStatusChanged {
                let exists: bool = self.connection.query_row(
                    "SELECT EXISTS(SELECT 1 FROM event_log WHERE event_id=?1)",
                    params![event.event_id],
                    |row| row.get(0),
                )?;
                if !exists {
                    let payload: TaskStatusChangedPayload =
                        serde_json::from_value(event.payload.clone())?;
                    validate_task_transition(&task.status, &payload.status)?;
                }
            }
        }
        Ok(())
    }
}

pub(super) fn rebuild_status(
    transaction: &Transaction<'_>,
    task_id: &str,
    team_id: &str,
    received_at: u64,
) -> Result<(), StorageError> {
    let mut query = transaction.prepare("SELECT payload_json FROM event_log WHERE team_id=?1 AND event_type='task.status_changed' AND json_extract(payload_json,'$.task_id')=?2 ORDER BY logical_time,event_id")?;
    let rows = query.query_map(params![team_id, task_id], |row| row.get::<_, String>(0))?;
    let mut status = TaskStatus::Queued;
    for row in rows {
        let payload: TaskStatusChangedPayload = serde_json::from_str(&row?)?;
        if status == payload.status || status.can_transition_to(&payload.status) {
            status = payload.status;
        }
    }
    transaction.execute(
        "UPDATE tasks SET status_json=?1,updated_at=?2 WHERE task_id=?3",
        params![encode(&status)?, received_at, task_id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::team::Role;
    use serde_json::json;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn fixture(
        label: &str,
    ) -> (
        TeamStore,
        DeviceIdentity,
        DeviceIdentity,
        PathBuf,
        PathBuf,
        PathBuf,
    ) {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!(
            "herdr-task-policy-{label}-{}-{nonce}",
            std::process::id()
        ));
        let db = base.with_extension("sqlite");
        let owner_path = base.with_extension("owner.json");
        let peer_path = base.with_extension("peer.json");
        let owner = DeviceIdentity::load_or_create(&owner_path).unwrap();
        let peer = DeviceIdentity::load_or_create(&peer_path).unwrap();
        let store = TeamStore::open(&db).unwrap();
        store
            .save_team(&Team {
                team_id: "team-policy".into(),
                name: "Policy".into(),
                authority_member_id: "owner".into(),
                created_at: 1,
            })
            .unwrap();
        for (id, identity) in [("owner", &owner), ("peer", &peer)] {
            // Both display labels deliberately say manager; only authority authenticates.
            store
                .save_member(
                    "team-policy",
                    &Member {
                        member_id: id.into(),
                        display_name: id.into(),
                        role: Role::Manager,
                        active: true,
                    },
                    1,
                )
                .unwrap();
            store
                .save_device(
                    &Device {
                        device_id: identity.device_id().into(),
                        member_id: id.into(),
                        public_key: identity.public_key(),
                        active: true,
                    },
                    1,
                )
                .unwrap();
        }
        (store, owner, peer, db, owner_path, peer_path)
    }
    fn signed(
        identity: &DeviceIdentity,
        actor: &str,
        kind: EventType,
        payload: Value,
        clock: u64,
    ) -> Event {
        let mut event = Event::create(
            "team-policy",
            actor,
            identity.device_id(),
            kind,
            payload,
            clock,
            "pending",
        )
        .unwrap();
        event.signature = Some(identity.sign(&event.signing_bytes()));
        event
    }
    fn created(identity: &DeviceIdentity, assigner: &str, assignee: &str) -> Event {
        signed(
            identity,
            assigner,
            EventType::TaskCreated,
            json!({"task_id":"task-policy","assigner_member_id":assigner,"assignee_member_id":assignee,"description":"Verify task policy","created_at":1}),
            1,
        )
    }
    fn clean(store: TeamStore, paths: [PathBuf; 3]) {
        drop(store);
        for path in paths {
            let _ = fs::remove_file(path);
        }
    }
    #[test]
    fn task_policy_rejects_role_labels_spoofed_assigner_inactive_recipient_and_cross_team() {
        let (mut store, owner, peer, db, owner_path, peer_path) = fixture("authorization");
        assert!(store
            .insert_verified_event(&created(&peer, "peer", "owner"), 1)
            .is_err());
        let mut spoofed = created(&owner, "owner", "peer");
        spoofed.payload["assigner_member_id"] = json!("peer");
        spoofed.signature = Some(owner.sign(&spoofed.signing_bytes()));
        assert!(store.insert_verified_event(&spoofed, 1).is_err());
        store
            .save_member(
                "team-policy",
                &Member {
                    member_id: "peer".into(),
                    display_name: "Peer".into(),
                    role: Role::Manager,
                    active: false,
                },
                2,
            )
            .unwrap();
        assert!(store
            .insert_verified_event(&created(&owner, "owner", "peer"), 2)
            .is_err());
        store
            .save_member(
                "team-policy",
                &Member {
                    member_id: "peer".into(),
                    display_name: "Peer".into(),
                    role: Role::Manager,
                    active: true,
                },
                3,
            )
            .unwrap();
        store
            .insert_verified_event(&created(&owner, "owner", "owner"), 3)
            .unwrap();
        let update = signed(
            &peer,
            "peer",
            EventType::TaskStatusChanged,
            json!({"task_id":"task-policy","status":"Delivered"}),
            4,
        );
        assert!(store.insert_verified_event(&update, 4).is_err());
        assert_eq!(
            store.task("task-policy").unwrap().unwrap().status,
            TaskStatus::Queued
        );
        assert_eq!(store.task_activity("task-policy").unwrap().len(), 1);
        clean(store, [db, owner_path, peer_path]);
    }
    #[test]
    fn explicit_task_grants_work_and_revocation_stops_unrelated_writers() {
        let (mut store, owner, peer, db, owner_path, peer_path) = fixture("grants");
        let create = created(&peer, "peer", "owner");
        assert!(store.insert_verified_event(&create, 1).is_err());
        store
            .grant_permission(
                "team-policy",
                &PermissionGrant {
                    grant_id: "task-create-grant".into(),
                    subject_member_id: "peer".into(),
                    resource: PermissionResource::Team("team-policy".into()),
                    action: PermissionAction::CreateTask,
                    grantor_member_id: "owner".into(),
                    delegable: false,
                    expires_at: None,
                    revoked: false,
                },
                1,
            )
            .unwrap();
        assert!(store.insert_verified_event(&create, 2).unwrap());
        let update = signed(
            &peer,
            "peer",
            EventType::TaskStatusChanged,
            json!({"task_id":"task-policy","status":"Delivered"}),
            3,
        );
        assert!(store.insert_verified_event(&update, 3).is_err());
        store
            .grant_permission(
                "team-policy",
                &PermissionGrant {
                    grant_id: "task-update-grant".into(),
                    subject_member_id: "peer".into(),
                    resource: PermissionResource::Task("task-policy".into()),
                    action: PermissionAction::UpdateTask,
                    grantor_member_id: "owner".into(),
                    delegable: false,
                    expires_at: None,
                    revoked: false,
                },
                3,
            )
            .unwrap();
        assert!(store.insert_verified_event(&update, 4).unwrap());
        store
            .revoke_permission("team-policy", "task-update-grant", "owner", 5)
            .unwrap();
        let next = signed(
            &peer,
            "peer",
            EventType::TaskStatusChanged,
            json!({"task_id":"task-policy","status":"InProgress"}),
            6,
        );
        assert!(store.insert_verified_event(&next, 6).is_err());
        let own = signed(
            &owner,
            "owner",
            EventType::TaskStatusChanged,
            json!({"task_id":"task-policy","status":"InProgress"}),
            6,
        );
        assert!(store.insert_verified_event(&own, 6).unwrap());
        clean(store, [db, owner_path, peer_path]);
    }

    #[test]
    fn remote_task_status_history_converges_despite_reverse_delivery_and_redundancy() {
        let (mut first, owner, peer, a, ak, ap) = fixture("first");
        let second_path = a.with_extension("second.sqlite");
        let second = TeamStore::open(&second_path).unwrap();
        second
            .save_team(&Team {
                team_id: "team-policy".into(),
                name: "Policy".into(),
                authority_member_id: "owner".into(),
                created_at: 1,
            })
            .unwrap();
        for (id, identity) in [("owner", &owner), ("peer", &peer)] {
            second
                .save_member(
                    "team-policy",
                    &Member {
                        member_id: id.into(),
                        display_name: id.into(),
                        role: Role::Implementer,
                        active: true,
                    },
                    1,
                )
                .unwrap();
            second
                .save_device(
                    &Device {
                        device_id: identity.device_id().into(),
                        member_id: id.into(),
                        public_key: identity.public_key(),
                        active: true,
                    },
                    1,
                )
                .unwrap();
        }
        let mut second = second;
        let create = created(&owner, "owner", "peer");
        first.insert_verified_event(&create, 1).unwrap();
        second.insert_verified_event(&create, 1).unwrap();
        let events = [
            ("Delivered", 2),
            ("Delivered", 3),
            ("InProgress", 4),
            ("Done", 5),
            ("Cancelled", 5),
        ]
        .map(|(status, clock)| {
            signed(
                &peer,
                "peer",
                EventType::TaskStatusChanged,
                json!({"task_id":"task-policy","status":status}),
                clock,
            )
        });
        for event in &events {
            first.insert_verified_event(event, 10).unwrap();
        }
        for event in events.iter().rev() {
            second.insert_verified_event(event, 11).unwrap();
        }
        assert_eq!(
            first.task("task-policy").unwrap().unwrap().status,
            second.task("task-policy").unwrap().unwrap().status
        );
        assert_eq!(first.task_activity("task-policy").unwrap().len(), 6);
        assert_eq!(second.task_activity("task-policy").unwrap().len(), 6);
        assert!(!second.insert_verified_event(&events[0], 12).unwrap());
        let backwards = signed(
            &peer,
            "peer",
            EventType::TaskStatusChanged,
            json!({"task_id":"task-policy","status":"Queued"}),
            6,
        );
        assert!(second.insert_event(&backwards, 12).is_err());
        drop(second);
        let _ = fs::remove_file(second_path);
        clean(first, [a, ak, ap]);
    }
}
