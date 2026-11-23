//! Device-local conversation history. Nothing in these tables is synchronized.
use super::*;
use sha2::{Digest, Sha256};

const MAX_TEXT: usize = 64 * 1024;
const MAX_HISTORY: usize = 500;

use crate::crewai_conversation::{ConversationEntry, ConversationKind};

fn valid_id(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 512 && !value.chars().any(char::is_control)
}
fn valid_scope(team: &str, thread: &str) -> Result<(), StorageError> {
    if !valid_id(team) || !valid_id(thread) {
        return Err(StorageError::MembershipConflict("conversation scope"));
    }
    Ok(())
}
fn draft_context(team: &str, thread: &str) -> String {
    let scope = format!("{}:{team}{thread}", team.len());
    format!(
        "conversation/draft/v1/{:x}",
        Sha256::digest(scope.as_bytes())
    )
}

impl TeamStore {
    /// Insert or update one stable entry. Identity and creation time cannot be reassigned.
    pub(crate) fn save_conversation_entry(
        &self,
        entry: &ConversationEntry,
    ) -> Result<(), StorageError> {
        valid_scope(&entry.team_id, &entry.thread_id)?;
        if !valid_id(&entry.entry_id)
            || entry.related_id.as_deref().is_some_and(|v| !valid_id(v))
            || entry.speaker_id.as_deref().is_some_and(|v| !valid_id(v))
            || entry.recipient_id.as_deref().is_some_and(|v| !valid_id(v))
            || entry.text.len() > MAX_TEXT
            || entry.status.len() > 256
            || entry.updated_at < entry.created_at
        {
            return Err(StorageError::MembershipConflict(
                "invalid conversation entry",
            ));
        }
        let changed = self.connection.execute(
            "INSERT INTO conversation_entries(team_id,thread_id,entry_id,kind,speaker_id,recipient_id,text,status,created_at,updated_at,related_id)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
             ON CONFLICT(team_id,thread_id,entry_id) DO UPDATE SET text=excluded.text,status=excluded.status,updated_at=excluded.updated_at
             WHERE kind=excluded.kind AND speaker_id IS excluded.speaker_id AND recipient_id IS excluded.recipient_id AND related_id IS excluded.related_id
               AND created_at=excluded.created_at AND (updated_at<excluded.updated_at OR
                 (updated_at=excluded.updated_at AND text=excluded.text AND status=excluded.status))",
            params![entry.team_id,entry.thread_id,entry.entry_id,entry.kind.as_str(),entry.speaker_id,entry.recipient_id,entry.text,entry.status,entry.created_at,entry.updated_at,entry.related_id],
        )?;
        if changed == 0 {
            return Err(StorageError::MembershipConflict(
                "conversation identity or stale update",
            ));
        }
        self.connection.execute(
            "DELETE FROM conversation_entries WHERE rowid IN (SELECT rowid FROM conversation_entries WHERE team_id=?1 AND thread_id=?2 ORDER BY created_at DESC,entry_id DESC LIMIT -1 OFFSET ?3)",
            params![entry.team_id,entry.thread_id,MAX_HISTORY as u64],
        )?;
        Ok(())
    }

    pub(crate) fn conversation_history(
        &self,
        team: &str,
        thread: &str,
    ) -> Result<Vec<ConversationEntry>, StorageError> {
        valid_scope(team, thread)?;
        let mut statement = self.connection.prepare(
            "SELECT entry_id,kind,speaker_id,recipient_id,text,status,created_at,updated_at,related_id FROM
             (SELECT * FROM conversation_entries WHERE team_id=?1 AND thread_id=?2 ORDER BY created_at DESC,entry_id DESC LIMIT ?3)
             ORDER BY created_at,entry_id")?;
        let rows = statement.query_map(params![team, thread, MAX_HISTORY as u64], |row| {
            let kind: String = row.get(1)?;
            let kind = ConversationKind::parse(&kind).ok_or_else(|| {
                rusqlite::Error::InvalidColumnType(1, "kind".into(), rusqlite::types::Type::Text)
            })?;
            Ok(ConversationEntry {
                team_id: team.into(),
                thread_id: thread.into(),
                entry_id: row.get(0)?,
                related_id: row.get(8)?,
                kind,
                speaker_id: row.get(2)?,
                recipient_id: row.get(3)?,
                text: row.get(4)?,
                status: row.get(5)?,
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub(crate) fn save_conversation_draft(
        &self,
        team: &str,
        thread: &str,
        text: &str,
    ) -> Result<(), StorageError> {
        valid_scope(team, thread)?;
        if text.len() > MAX_TEXT {
            return Err(StorageError::MembershipConflict(
                "conversation draft too large",
            ));
        }
        if text.is_empty() {
            self.connection.execute(
                "DELETE FROM conversation_drafts WHERE team_id=?1 AND thread_id=?2",
                params![team, thread],
            )?;
        } else {
            let identity =
                DeviceIdentity::load_existing(crate::team_session::identity_path(&self.path))?;
            let sealed = identity.seal_local(&draft_context(team, thread), text.as_bytes())?;
            self.connection.execute("INSERT INTO conversation_drafts(team_id,thread_id,text) VALUES(?1,?2,?3) ON CONFLICT(team_id,thread_id) DO UPDATE SET text=excluded.text",params![team,thread,sealed])?;
        }
        Ok(())
    }

    pub(crate) fn conversation_draft(
        &self,
        team: &str,
        thread: &str,
    ) -> Result<Option<String>, StorageError> {
        valid_scope(team, thread)?;
        let sealed: Option<String> = self
            .connection
            .query_row(
                "SELECT text FROM conversation_drafts WHERE team_id=?1 AND thread_id=?2",
                params![team, thread],
                |row| row.get(0),
            )
            .optional()?;
        sealed
            .map(|sealed| {
                let identity =
                    DeviceIdentity::load_existing(crate::team_session::identity_path(&self.path))?;
                let bytes = identity.open_local(&draft_context(team, thread), &sealed)?;
                if bytes.len() > MAX_TEXT {
                    return Err(StorageError::MembershipConflict(
                        "conversation draft too large",
                    ));
                }
                String::from_utf8(bytes)
                    .map_err(|_| StorageError::MembershipConflict("invalid conversation draft"))
            })
            .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stable_entries_and_drafts_remain_separate_across_restarts_and_teams() {
        let path = std::env::temp_dir().join(format!(
            "crewai-history-{}.sqlite",
            crate::cli::team::generate_token()
        ));
        DeviceIdentity::load_or_create(crate::team_session::identity_path(&path)).unwrap();
        let mut first = ConversationEntry {
            team_id: "a".into(),
            thread_id: "main".into(),
            entry_id: "question:1".into(),
            related_id: Some("question:1".into()),
            kind: ConversationKind::Approval,
            speaker_id: Some("alice".into()),
            recipient_id: Some("bob".into()),
            text: "First".into(),
            status: "pending".into(),
            created_at: 1,
            updated_at: 1,
        };
        {
            let store = TeamStore::open(&path).unwrap();
            store.save_conversation_entry(&first).unwrap();
            store.save_conversation_entry(&first).unwrap();
            first.text = "Answered".into();
            first.status = "completed".into();
            first.updated_at = 2;
            store.save_conversation_entry(&first).unwrap();
            let mut second = first.clone();
            second.entry_id = "question:2".into();
            second.related_id = Some("question:2".into());
            second.text = "Another".into();
            second.status = "pending".into();
            store.save_conversation_entry(&second).unwrap();
            let mut collision = first.clone();
            collision.recipient_id = Some("eve".into());
            assert!(store.save_conversation_entry(&collision).is_err());
            let mut stale = first.clone();
            stale.updated_at = 1;
            assert!(store.save_conversation_entry(&stale).is_err());
            store
                .save_conversation_draft("a", "main", "draft A")
                .unwrap();
            store
                .save_conversation_draft("b", "main", "draft B")
                .unwrap();
            let stored: String = store
                .connection
                .query_row(
                    "SELECT text FROM conversation_drafts WHERE team_id='a'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(!stored.contains("draft A"));
        }
        let store = TeamStore::open(&path).unwrap();
        let history = store.conversation_history("a", "main").unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].status, "completed");
        assert_eq!(history[1].status, "pending");
        assert!(store.conversation_history("b", "main").unwrap().is_empty());
        assert_eq!(
            store.conversation_draft("a", "main").unwrap().as_deref(),
            Some("draft A")
        );
        assert_eq!(
            store.conversation_draft("b", "main").unwrap().as_deref(),
            Some("draft B")
        );
        assert!(store
            .save_conversation_draft("a", "main", &"x".repeat(MAX_TEXT + 1))
            .is_err());
        store.connection.execute("UPDATE conversation_drafts SET text=(SELECT text FROM conversation_drafts WHERE team_id='a') WHERE team_id='b'", []).unwrap();
        assert!(
            store.conversation_draft("b", "main").is_err(),
            "ciphertext cannot be moved between teams"
        );
        drop(store);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn history_retention_is_bounded_per_thread() {
        let store = TeamStore::open(":memory:").unwrap();
        for index in 0..=MAX_HISTORY {
            let entry = ConversationEntry {
                team_id: "a".into(),
                thread_id: "main".into(),
                entry_id: format!("event:{index:04}"),
                related_id: None,
                kind: ConversationKind::Broadcast,
                speaker_id: None,
                recipient_id: None,
                text: "message".into(),
                status: "recorded".into(),
                created_at: index as u64,
                updated_at: index as u64,
            };
            store.save_conversation_entry(&entry).unwrap();
        }
        let entries = store.conversation_history("a", "main").unwrap();
        assert_eq!(entries.len(), MAX_HISTORY);
        assert_eq!(entries[0].entry_id, "event:0001");
    }

    #[test]
    fn conversation_migration_preserves_existing_team_data() {
        let path = std::env::temp_dir().join(format!(
            "crewai-history-migration-{}.sqlite",
            crate::cli::team::generate_token()
        ));
        {
            let store = TeamStore::open(&path).unwrap();
            store.save_relay_cursor("existing-team", 42, 10).unwrap();
            store.connection.execute_batch("DROP TABLE conversation_entries; DROP TABLE conversation_drafts; UPDATE schema_version SET version=8;").unwrap();
        }
        let store = TeamStore::open(&path).unwrap();
        assert_eq!(store.relay_cursor("existing-team").unwrap(), 42);
        assert!(store
            .conversation_history("existing-team", "main")
            .unwrap()
            .is_empty());
        drop(store);
        let _ = std::fs::remove_file(path);
    }
}
