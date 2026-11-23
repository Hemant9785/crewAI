//! Recipient-scoped local direct-message storage.
use super::*;
use crate::crewai_message::DirectMessage;

impl TeamStore {
    /// Verify actor, signature, participant and team before accepting an envelope.
    /// Returns false for an identical retry and rejects conflicting message IDs.
    pub(crate) fn insert_direct_message(
        &self,
        message: &DirectMessage,
    ) -> Result<bool, StorageError> {
        let local = self
            .load_profile()?
            .ok_or(StorageError::MembershipConflict("local profile"))?
            .member;
        if !local.active
            || !self.active_member_exists(&message.team_id, &local.member_id)?
            || !self.active_member_exists(&message.team_id, &message.sender_member_id)?
            || !self.active_member_exists(&message.team_id, &message.recipient_member_id)?
        {
            return Err(StorageError::UnauthorizedEvent {
                actor_member_id: message.sender_member_id.clone(),
                device_id: message.sender_device_id.clone(),
            });
        }
        let public_key: Option<String> = self
            .connection
            .query_row(
                "SELECT public_key FROM devices WHERE device_id=?1 AND member_id=?2 AND active=1",
                params![message.sender_device_id, message.sender_member_id],
                |row| row.get(0),
            )
            .optional()?;
        let key = public_key.ok_or_else(|| StorageError::UnauthorizedEvent {
            actor_member_id: message.sender_member_id.clone(),
            device_id: message.sender_device_id.clone(),
        })?;
        message
            .verify(&key, &message.team_id, &local.member_id)
            .map_err(|_| StorageError::InvalidEventSignature)?;
        if let Some(reply) = &message.reply_to {
            let valid:bool=self.connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM direct_messages WHERE message_id=?1 AND team_id=?2 AND ((sender_member_id=?3 AND recipient_member_id=?4) OR (sender_member_id=?4 AND recipient_member_id=?3)))",
                params![reply,message.team_id,message.sender_member_id,message.recipient_member_id],|row|row.get(0))?;
            if !valid {
                return Err(StorageError::MembershipConflict("direct-message reply"));
            }
        }
        let json = serde_json::to_string(message)?;
        let existing: Option<String> = self
            .connection
            .query_row(
                "SELECT envelope_json FROM direct_messages WHERE message_id=?1",
                params![message.message_id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(existing) = existing {
            return if existing == json {
                Ok(false)
            } else {
                Err(StorageError::ConflictingEvent {
                    event_id: message.message_id.clone(),
                })
            };
        }
        let state = if local.member_id == message.sender_member_id {
            "queued"
        } else {
            "received"
        };
        self.connection.execute("INSERT INTO direct_messages(message_id,team_id,sender_member_id,recipient_member_id,reply_to,envelope_json,state,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![message.message_id,message.team_id,message.sender_member_id,message.recipient_member_id,message.reply_to,json,state,message.created_at])?;
        Ok(true)
    }

    pub(crate) fn direct_messages(
        &self,
        team_id: &str,
    ) -> Result<Vec<(DirectMessage, String)>, StorageError> {
        let local = self
            .load_profile()?
            .ok_or(StorageError::MembershipConflict("local profile"))?
            .member;
        if !local.active || !self.active_member_exists(team_id, &local.member_id)? {
            return Err(StorageError::MembershipConflict("active local membership"));
        }
        let mut statement=self.connection.prepare("SELECT envelope_json,state FROM direct_messages WHERE team_id=?1 AND (sender_member_id=?2 OR recipient_member_id=?2) ORDER BY created_at DESC,message_id DESC LIMIT 500")?;
        let rows = statement.query_map(params![team_id, local.member_id], |row| {
            let json: String = row.get(0)?;
            let state: String = row.get(1)?;
            let value = serde_json::from_str(&json).map_err(to_sql_error)?;
            Ok((value, state))
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub(crate) fn queued_direct_messages(
        &self,
        team_id: &str,
    ) -> Result<Vec<DirectMessage>, StorageError> {
        let local = self
            .load_profile()?
            .ok_or(StorageError::MembershipConflict("local profile"))?
            .member;
        if !local.active || !self.active_member_exists(team_id, &local.member_id)? {
            return Err(StorageError::MembershipConflict("active local membership"));
        }
        let mut statement=self.connection.prepare("SELECT envelope_json FROM direct_messages WHERE team_id=?1 AND sender_member_id=?2 AND state='queued' ORDER BY rowid LIMIT 100")?;
        let rows = statement.query_map(params![team_id, local.member_id], |row| {
            let json: String = row.get(0)?;
            serde_json::from_str(&json).map_err(to_sql_error)
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub(crate) fn mark_direct_synced(
        &self,
        team_id: &str,
        message_id: &str,
    ) -> Result<(), StorageError> {
        let local = self
            .load_profile()?
            .ok_or(StorageError::MembershipConflict("local profile"))?
            .member;
        if !local.active || !self.active_member_exists(team_id, &local.member_id)? {
            return Err(StorageError::MembershipConflict("active local membership"));
        }
        let changed=self.connection.execute("UPDATE direct_messages SET state='synced' WHERE team_id=?1 AND message_id=?2 AND sender_member_id=?3",params![team_id,message_id,local.member_id])?;
        if changed != 1 {
            return Err(StorageError::MembershipConflict("own direct message"));
        }
        Ok(())
    }

    pub(crate) fn direct_cursor(&self, team_id: &str) -> Result<u64, StorageError> {
        let local = self
            .load_profile()?
            .ok_or(StorageError::MembershipConflict("local profile"))?
            .member;
        if !local.active || !self.active_member_exists(team_id, &local.member_id)? {
            return Err(StorageError::MembershipConflict("active local membership"));
        }
        Ok(self
            .connection
            .query_row(
                "SELECT sequence FROM direct_message_cursors WHERE team_id=?1 AND member_id=?2",
                params![team_id, local.member_id],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(0))
    }

    pub(crate) fn advance_direct_cursor(
        &self,
        team_id: &str,
        sequence: u64,
    ) -> Result<(), StorageError> {
        let local = self
            .load_profile()?
            .ok_or(StorageError::MembershipConflict("local profile"))?
            .member;
        if !local.active || !self.active_member_exists(team_id, &local.member_id)? {
            return Err(StorageError::MembershipConflict("active local membership"));
        }
        self.connection.execute("INSERT INTO direct_message_cursors(team_id,member_id,sequence) VALUES(?1,?2,?3) ON CONFLICT(team_id,member_id) DO UPDATE SET sequence=MAX(sequence,excluded.sequence)",params![team_id,local.member_id,sequence])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::team::{Device, Member, Role, Team};
    #[test]
    fn recipient_scope_signatures_replies_and_replay_are_enforced() {
        let root = std::env::temp_dir().join(format!(
            "crewai-direct-store-{}",
            crate::cli::team::generate_token()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let alice = DeviceIdentity::load_or_create(root.join("alice.json")).unwrap();
        let bob = DeviceIdentity::load_or_create(root.join("bob.json")).unwrap();
        let charlie = DeviceIdentity::load_or_create(root.join("charlie.json")).unwrap();
        let store = TeamStore::open(root.join("team.sqlite")).unwrap();
        let team = Team::create("Test", "alice", 1).unwrap();
        store.save_team(&team).unwrap();
        let member = |id: &str| Member {
            member_id: id.into(),
            display_name: id.into(),
            role: Role::Implementer,
            active: true,
        };
        for (id, key) in [("alice", &alice), ("bob", &bob), ("charlie", &charlie)] {
            store.save_member(&team.team_id, &member(id), 1).unwrap();
            store
                .save_device(
                    &Device {
                        device_id: key.device_id().into(),
                        member_id: id.into(),
                        public_key: key.public_key(),
                        active: true,
                    },
                    1,
                )
                .unwrap();
        }
        store.save_profile(&member("alice"), 1, 1).unwrap();
        let msg = DirectMessage::create(
            &team.team_id,
            "alice",
            "bob",
            None,
            "Private text",
            1,
            &alice,
        )
        .unwrap();
        assert!(store.insert_direct_message(&msg).unwrap());
        assert!(!store.insert_direct_message(&msg).unwrap());
        let mut changed = msg.clone();
        changed.body = "altered".into();
        assert!(store.insert_direct_message(&changed).is_err());
        store.save_profile(&member("charlie"), 1, 1).unwrap();
        assert!(store.insert_direct_message(&msg).is_err());
        assert!(store.direct_messages(&team.team_id).unwrap().is_empty());
        store.save_profile(&member("bob"), 1, 1).unwrap();
        assert_eq!(store.direct_messages(&team.team_id).unwrap().len(), 1);
        let reply = DirectMessage::create(
            &team.team_id,
            "bob",
            "alice",
            Some(msg.message_id.clone()),
            "Reply",
            2,
            &bob,
        )
        .unwrap();
        assert!(store.insert_direct_message(&reply).unwrap());
        let fake = DirectMessage::create(
            &team.team_id,
            "bob",
            "charlie",
            Some(msg.message_id.clone()),
            "Wrong thread",
            3,
            &bob,
        )
        .unwrap();
        assert!(store.insert_direct_message(&fake).is_err());
        store
            .save_device(
                &Device {
                    device_id: bob.device_id().into(),
                    member_id: "bob".into(),
                    public_key: bob.public_key(),
                    active: false,
                },
                4,
            )
            .unwrap();
        let revoked =
            DirectMessage::create(&team.team_id, "bob", "alice", None, "Revoked", 4, &bob).unwrap();
        assert!(store.insert_direct_message(&revoked).is_err());
    }
}
