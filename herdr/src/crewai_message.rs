//! Signed direct-message envelopes. Transport and persistence are separate.
use crate::identity::DeviceIdentity;
use serde::{Deserialize, Serialize};

const MAX_MESSAGE_BYTES: usize = 16 * 1024;
const CONTEXT: &str = "team-assistant/direct-message/v1";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct DirectMessage {
    pub message_id: String,
    pub team_id: String,
    pub sender_member_id: String,
    pub sender_device_id: String,
    pub recipient_member_id: String,
    pub reply_to: Option<String>,
    pub body: String,
    pub created_at: u64,
    pub signature: String,
}

impl DirectMessage {
    pub(crate) fn create(
        team_id: &str,
        sender: &str,
        recipient: &str,
        reply_to: Option<String>,
        body: &str,
        at: u64,
        identity: &DeviceIdentity,
    ) -> Result<Self, String> {
        let mut result = Self {
            message_id: format!("dm-{}", crate::cli::team::generate_token()),
            team_id: team_id.into(),
            sender_member_id: sender.into(),
            sender_device_id: identity.device_id().into(),
            recipient_member_id: recipient.into(),
            reply_to,
            body: body.into(),
            created_at: at,
            signature: String::new(),
        };
        result.validate_fields()?;
        result.signature = identity.sign(&result.signing_bytes());
        Ok(result)
    }
    pub(crate) fn signing_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(&(
            CONTEXT,
            &self.message_id,
            &self.team_id,
            &self.sender_member_id,
            &self.sender_device_id,
            &self.recipient_member_id,
            &self.reply_to,
            &self.body,
            self.created_at,
        ))
        .expect("message fields serialize")
    }
    pub(crate) fn validate_fields(&self) -> Result<(), String> {
        for value in [
            &self.message_id,
            &self.team_id,
            &self.sender_member_id,
            &self.sender_device_id,
            &self.recipient_member_id,
        ] {
            if value.trim().is_empty() || value.len() > 512 || value.chars().any(char::is_control) {
                return Err("Invalid message identity".into());
            }
        }
        if self.sender_member_id == self.recipient_member_id {
            return Err("Choose another teammate".into());
        }
        if self.body.trim().is_empty()
            || self.body.len() > MAX_MESSAGE_BYTES
            || self
                .body
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
        {
            return Err("Message must contain at most 16 KiB of text".into());
        }
        if self
            .reply_to
            .as_ref()
            .is_some_and(|s| s.is_empty() || s.len() > 512 || s.chars().any(char::is_control))
        {
            return Err("Invalid reply reference".into());
        }
        Ok(())
    }
    pub(crate) fn verify(
        &self,
        public_key: &str,
        team: &str,
        local_member: &str,
    ) -> Result<(), String> {
        self.validate_fields()?;
        if self.team_id != team
            || (self.sender_member_id != local_member && self.recipient_member_id != local_member)
        {
            return Err("Message is not addressed to this team member".into());
        }
        if DeviceIdentity::device_id_for_public_key(public_key).map_err(|e| e.to_string())?
            != self.sender_device_id
        {
            return Err("Message device identity mismatch".into());
        }
        DeviceIdentity::verify(public_key, &self.signing_bytes(), &self.signature)
            .map_err(|_| "Message signature is invalid".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signatures_bind_content_team_and_recipient() {
        let root =
            std::env::temp_dir().join(format!("crewai-dm-{}", crate::cli::team::generate_token()));
        let sender = DeviceIdentity::load_or_create(root).unwrap();
        let msg =
            DirectMessage::create("team", "alice", "bob", None, "Hello Bob", 1, &sender).unwrap();
        assert!(msg.verify(&sender.public_key(), "team", "bob").is_ok());
        assert!(msg.verify(&sender.public_key(), "team", "alice").is_ok());
        assert!(msg.verify(&sender.public_key(), "team", "charlie").is_err());
        assert!(msg.verify(&sender.public_key(), "other", "bob").is_err());
        let mut forged = msg.clone();
        forged.body = "modified".into();
        assert!(forged.verify(&sender.public_key(), "team", "bob").is_err());
        forged = msg.clone();
        forged.recipient_member_id = "charlie".into();
        assert!(forged
            .verify(&sender.public_key(), "team", "charlie")
            .is_err());
        assert!(DirectMessage::create("team", "alice", "bob", None, "", 1, &sender).is_err());
        assert!(DirectMessage::create(
            "team",
            "alice",
            "bob",
            None,
            &"x".repeat(MAX_MESSAGE_BYTES + 1),
            1,
            &sender
        )
        .is_err());
    }
}
