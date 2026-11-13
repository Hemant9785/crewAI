//! Domain types for team coordination.
//!
//! This module intentionally has no UI, database, network, or process-launching
//! dependencies. Keeping the state machines here makes them testable before the
//! synchronization and Herdr UI adapters are added.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) const MAX_EVENT_PAYLOAD_BYTES: usize = 256 * 1024;
pub(crate) const MAX_BROADCAST_MESSAGE_BYTES: usize = 16 * 1024;
pub(crate) const MAX_BROADCAST_SCOPE_BYTES: usize = 256;
pub(crate) const SUPPORTED_EVENT_SCHEMA_VERSION: u32 = 1;

static EVENT_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct Team {
    pub team_id: String,
    pub name: String,
    pub authority_member_id: String,
    pub created_at: u64,
}

impl Team {
    pub(crate) fn create(
        name: impl Into<String>,
        authority_member_id: impl Into<String>,
        created_at: u64,
    ) -> Result<Self, DomainError> {
        let name = name.into();
        let authority_member_id = authority_member_id.into();
        validate_nonempty(&name, "team.name")?;
        validate_nonempty(&authority_member_id, "team.authority_member_id")?;
        Ok(Self {
            team_id: next_unique_id("team", &authority_member_id),
            name,
            authority_member_id,
            created_at,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) enum Role {
    Manager,
    GroupHead,
    PartHead,
    Implementer,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct Member {
    pub member_id: String,
    pub display_name: String,
    pub role: Role,
    pub active: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct Device {
    pub device_id: String,
    pub member_id: String,
    pub public_key: String,
    pub active: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct TeamInvitation {
    pub invitation_id: String,
    pub team_id: String,
    pub team_name: String,
    pub authority_member_id: String,
    pub authority_device_id: String,
    pub authority_public_key: String,
    pub pairing_code_hash: String,
    pub created_at: u64,
    pub expires_at: u64,
    pub signature: String,
}

impl TeamInvitation {
    pub(crate) fn new_unsigned(
        team: &Team,
        authority_device: &Device,
        pairing_code: &str,
        created_at: u64,
        expires_at: u64,
    ) -> Result<Self, DomainError> {
        if pairing_code.trim().len() < 12 {
            return Err(DomainError::InvalidPayload(
                "pairing code must contain at least 12 non-space characters",
            ));
        }
        let invitation = Self {
            invitation_id: next_invitation_id(&authority_device.device_id),
            team_id: team.team_id.clone(),
            team_name: team.name.clone(),
            authority_member_id: team.authority_member_id.clone(),
            authority_device_id: authority_device.device_id.clone(),
            authority_public_key: authority_device.public_key.clone(),
            pairing_code_hash: hash_pairing_code(pairing_code),
            created_at,
            expires_at,
            signature: String::new(),
        };
        invitation.validate_unsigned()?;
        Ok(invitation)
    }

    pub(crate) fn with_signature(mut self, signature: String) -> Result<Self, DomainError> {
        self.signature = signature;
        self.validate()?;
        Ok(self)
    }

    pub(crate) fn signing_bytes(&self) -> Vec<u8> {
        [
            "team-invitation-v1",
            self.invitation_id.as_str(),
            self.team_id.as_str(),
            self.team_name.as_str(),
            self.authority_member_id.as_str(),
            self.authority_device_id.as_str(),
            self.authority_public_key.as_str(),
            self.pairing_code_hash.as_str(),
            &self.created_at.to_string(),
            &self.expires_at.to_string(),
        ]
        .join("\n")
        .into_bytes()
    }

    pub(crate) fn validate(&self) -> Result<(), DomainError> {
        self.validate_unsigned()?;
        validate_nonempty(&self.signature, "invitation.signature")
    }

    pub(crate) fn is_expired(&self, now: u64) -> bool {
        now >= self.expires_at
    }

    pub(crate) fn pairing_code_hash(pairing_code: &str) -> String {
        hash_pairing_code(pairing_code)
    }

    fn validate_unsigned(&self) -> Result<(), DomainError> {
        validate_nonempty(&self.invitation_id, "invitation.invitation_id")?;
        validate_nonempty(&self.team_id, "invitation.team_id")?;
        validate_nonempty(&self.team_name, "invitation.team_name")?;
        validate_nonempty(&self.authority_member_id, "invitation.authority_member_id")?;
        validate_nonempty(&self.authority_device_id, "invitation.authority_device_id")?;
        validate_nonempty(
            &self.authority_public_key,
            "invitation.authority_public_key",
        )?;
        if self.pairing_code_hash.len() != 64 {
            return Err(DomainError::InvalidPayload(
                "invitation pairing-code hash must be a SHA-256 digest",
            ));
        }
        if self.expires_at <= self.created_at {
            return Err(DomainError::InvalidPayload(
                "invitation expiry must be after creation",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct RepositoryApproval {
    pub repository_id: String,
    pub owner_member_id: String,
    pub local_path: String,
    pub approved_at: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct PermissionGrant {
    pub grant_id: String,
    pub subject_member_id: String,
    pub resource: PermissionResource,
    pub action: PermissionAction,
    pub grantor_member_id: String,
    pub delegable: bool,
    pub expires_at: Option<u64>,
    pub revoked: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) enum PermissionResource {
    Team(String),
    Group(String),
    Repository(String),
    Task(String),
    MessageScope(String),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) enum PermissionAction {
    ReadRepository,
    CreateTask,
    UpdateTask,
    Broadcast,
    ManageMembers,
    ManagePermissions,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PermissionDecisionReason {
    Allowed,
    NoMatchingGrant,
    GrantRevoked(String),
    GrantExpired(String),
    GrantChainUnavailable(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PermissionDecision {
    pub allowed: bool,
    pub chain: Vec<String>,
    pub reason: PermissionDecisionReason,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PermissionError {
    InvalidGrant(&'static str),
    DuplicateGrant(String),
    GrantorUnauthorized(String),
    GrantNotDelegable(String),
    CycleDetected(String),
    GrantNotFound(String),
    RevocationUnauthorized(String),
}

impl fmt::Display for PermissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidGrant(message) => formatter.write_str(message),
            Self::DuplicateGrant(grant_id) => {
                write!(formatter, "permission grant already exists: {grant_id}")
            }
            Self::GrantorUnauthorized(member_id) => {
                write!(
                    formatter,
                    "member cannot grant this permission: {member_id}"
                )
            }
            Self::GrantNotDelegable(grant_id) => {
                write!(formatter, "permission grant is not delegable: {grant_id}")
            }
            Self::CycleDetected(member_id) => {
                write!(
                    formatter,
                    "permission grant would create a cycle through: {member_id}"
                )
            }
            Self::GrantNotFound(grant_id) => {
                write!(formatter, "permission grant not found: {grant_id}")
            }
            Self::RevocationUnauthorized(member_id) => {
                write!(
                    formatter,
                    "member cannot revoke this permission: {member_id}"
                )
            }
        }
    }
}

impl std::error::Error for PermissionError {}

#[derive(Clone, Debug)]
pub(crate) struct PermissionEvaluator {
    authority_member_id: String,
    grants: Vec<PermissionGrant>,
}

impl PermissionEvaluator {
    pub(crate) fn new(authority_member_id: impl Into<String>) -> Self {
        Self {
            authority_member_id: authority_member_id.into(),
            grants: Vec::new(),
        }
    }

    pub(crate) fn from_grants(
        authority_member_id: impl Into<String>,
        grants: Vec<PermissionGrant>,
    ) -> Self {
        Self {
            authority_member_id: authority_member_id.into(),
            grants,
        }
    }

    pub(crate) fn grants(&self) -> &[PermissionGrant] {
        &self.grants
    }

    pub(crate) fn add_grant(
        &mut self,
        grant: PermissionGrant,
        now: u64,
    ) -> Result<(), PermissionError> {
        validate_grant(&grant)?;
        if self
            .grants
            .iter()
            .any(|existing| existing.grant_id == grant.grant_id)
        {
            return Err(PermissionError::DuplicateGrant(grant.grant_id));
        }
        if self.would_create_cycle(&grant) {
            return Err(PermissionError::CycleDetected(grant.subject_member_id));
        }
        if grant.grantor_member_id != self.authority_member_id
            && !self.can_delegate_to(
                &grant.grantor_member_id,
                &grant.resource,
                &grant.action,
                now,
            )
        {
            return Err(PermissionError::GrantorUnauthorized(
                grant.grantor_member_id,
            ));
        }
        self.grants.push(grant);
        Ok(())
    }

    pub(crate) fn revoke(
        &mut self,
        grant_id: &str,
        actor_member_id: &str,
        now: u64,
    ) -> Result<(), PermissionError> {
        let index = self
            .grants
            .iter()
            .position(|grant| grant.grant_id == grant_id)
            .ok_or_else(|| PermissionError::GrantNotFound(grant_id.into()))?;
        let target = &self.grants[index];
        let can_revoke = actor_member_id == self.authority_member_id
            || self
                .check(
                    actor_member_id,
                    &target.resource,
                    &PermissionAction::ManagePermissions,
                    now,
                )
                .allowed;
        if !can_revoke {
            return Err(PermissionError::RevocationUnauthorized(
                actor_member_id.into(),
            ));
        }
        self.grants[index].revoked = true;
        Ok(())
    }

    pub(crate) fn check(
        &self,
        subject_member_id: &str,
        resource: &PermissionResource,
        action: &PermissionAction,
        now: u64,
    ) -> PermissionDecision {
        let mut denied_reason = None;
        let mut denied_chain = Vec::new();
        for (index, grant) in self.grants.iter().enumerate() {
            if grant.subject_member_id == subject_member_id
                && grant.resource == *resource
                && grant.action == *action
            {
                if let Some(chain) = self.effective_chain(index, now, &mut HashSet::new()) {
                    return PermissionDecision {
                        allowed: true,
                        chain,
                        reason: PermissionDecisionReason::Allowed,
                    };
                }
                if denied_reason.is_none() {
                    let (reason, chain) = self.failure_explanation(index, now, &mut HashSet::new());
                    denied_reason = Some(reason);
                    denied_chain = chain;
                }
            }
        }
        PermissionDecision {
            allowed: false,
            chain: denied_chain,
            reason: denied_reason.unwrap_or(PermissionDecisionReason::NoMatchingGrant),
        }
    }

    fn failure_explanation(
        &self,
        index: usize,
        now: u64,
        visiting: &mut HashSet<String>,
    ) -> (PermissionDecisionReason, Vec<String>) {
        let Some(grant) = self.grants.get(index) else {
            return (
                PermissionDecisionReason::GrantChainUnavailable("unknown".into()),
                Vec::new(),
            );
        };
        if grant.revoked {
            return (
                PermissionDecisionReason::GrantRevoked(grant.grant_id.clone()),
                vec![grant.grant_id.clone()],
            );
        }
        if grant.expires_at.is_some_and(|expires_at| now >= expires_at) {
            return (
                PermissionDecisionReason::GrantExpired(grant.grant_id.clone()),
                vec![grant.grant_id.clone()],
            );
        }
        if !visiting.insert(grant.grant_id.clone()) {
            return (
                PermissionDecisionReason::GrantChainUnavailable(grant.grant_id.clone()),
                vec![grant.grant_id.clone()],
            );
        }
        if grant.grantor_member_id == self.authority_member_id {
            visiting.remove(&grant.grant_id);
            return (
                PermissionDecisionReason::GrantChainUnavailable(grant.grant_id.clone()),
                vec![grant.grant_id.clone()],
            );
        }
        let parent_index = self.grants.iter().position(|parent| {
            parent.subject_member_id == grant.grantor_member_id
                && parent.delegable
                && parent.resource == grant.resource
                && (parent.action == grant.action
                    || parent.action == PermissionAction::ManagePermissions)
        });
        let (reason, mut chain) = parent_index.map_or_else(
            || {
                (
                    PermissionDecisionReason::GrantChainUnavailable(grant.grant_id.clone()),
                    vec![grant.grant_id.clone()],
                )
            },
            |parent_index| self.failure_explanation(parent_index, now, visiting),
        );
        chain.push(grant.grant_id.clone());
        visiting.remove(&grant.grant_id);
        (reason, chain)
    }

    fn can_delegate_to(
        &self,
        member_id: &str,
        resource: &PermissionResource,
        action: &PermissionAction,
        now: u64,
    ) -> bool {
        self.grants.iter().enumerate().any(|(index, grant)| {
            grant.subject_member_id == member_id
                && grant.delegable
                && grant.resource == *resource
                && (grant.action == *action || grant.action == PermissionAction::ManagePermissions)
                && self
                    .effective_chain(index, now, &mut HashSet::new())
                    .is_some()
        })
    }

    fn effective_chain(
        &self,
        index: usize,
        now: u64,
        visiting: &mut HashSet<String>,
    ) -> Option<Vec<String>> {
        let grant = self.grants.get(index)?;
        if grant.revoked || grant.expires_at.is_some_and(|expires_at| now >= expires_at) {
            return None;
        }
        if !visiting.insert(grant.grant_id.clone()) {
            return None;
        }
        if grant.grantor_member_id == self.authority_member_id {
            let mut chain = vec![grant.grant_id.clone()];
            visiting.remove(&grant.grant_id);
            return Some(std::mem::take(&mut chain));
        }

        let result = self
            .grants
            .iter()
            .enumerate()
            .find_map(|(parent_index, parent)| {
                if parent.subject_member_id != grant.grantor_member_id
                    || !parent.delegable
                    || parent.resource != grant.resource
                    || (parent.action != grant.action
                        && parent.action != PermissionAction::ManagePermissions)
                {
                    return None;
                }
                let mut chain = self.effective_chain(parent_index, now, visiting)?;
                chain.push(grant.grant_id.clone());
                Some(chain)
            });
        visiting.remove(&grant.grant_id);
        result
    }

    fn would_create_cycle(&self, grant: &PermissionGrant) -> bool {
        if grant.grantor_member_id == grant.subject_member_id {
            return true;
        }
        let mut visited = HashSet::new();
        self.reaches(
            &grant.subject_member_id,
            &grant.grantor_member_id,
            &mut visited,
        )
    }

    fn reaches(&self, from: &str, target: &str, visited: &mut HashSet<String>) -> bool {
        if from == target {
            return true;
        }
        if !visited.insert(from.into()) {
            return false;
        }
        self.grants.iter().any(|grant| {
            !grant.revoked
                && grant.grantor_member_id == from
                && self.reaches(&grant.subject_member_id, target, visited)
        })
    }
}

fn validate_grant(grant: &PermissionGrant) -> Result<(), PermissionError> {
    for value in [
        (&grant.grant_id, "permission grant ID"),
        (&grant.subject_member_id, "permission subject"),
        (&grant.grantor_member_id, "permission grantor"),
    ] {
        if value.0.trim().is_empty() {
            return Err(PermissionError::InvalidGrant(value.1));
        }
    }
    if grant.expires_at.is_some_and(|expires_at| expires_at == 0) {
        return Err(PermissionError::InvalidGrant(
            "permission expiry must be greater than zero",
        ));
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct TaskRecord {
    pub task_id: String,
    pub assigner_member_id: String,
    pub assignee_member_id: String,
    pub description: String,
    pub status: TaskStatus,
    pub created_at: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct TaskActivityRecord {
    pub activity_id: String,
    pub task_id: String,
    pub actor_member_id: String,
    pub activity_type: String,
    pub message: String,
    pub status: Option<TaskStatus>,
    pub created_at: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct BroadcastRecord {
    pub message_id: String,
    pub sender_member_id: String,
    pub scope: String,
    pub message: String,
    pub created_at: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct RequestRecord {
    pub request_id: String,
    pub requester_member_id: String,
    pub target_member_id: String,
    pub kind: RequestKind,
    pub payload: String,
    pub delivery: DeliveryState,
    pub created_at: u64,
    pub result: Option<RequestResult>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct RequestResult {
    pub isolation: String,
    pub network: String,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub output_truncated: bool,
    pub started_at: u64,
    pub duration_ms: u128,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OutboxRecord {
    pub event_id: String,
    pub attempts: u32,
    pub next_attempt_at: Option<u64>,
    pub state: OutboxState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OutboxState {
    Queued,
    InFlight,
    Delivered,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) enum RequestKind {
    Task,
    Check,
    Read,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) enum TaskStatus {
    Queued,
    Delivered,
    InProgress,
    Done,
    Cancelled,
    Failed,
    NeedsAttention,
}

impl TaskStatus {
    pub(crate) fn can_transition_to(&self, next: &Self) -> bool {
        use TaskStatus::*;

        matches!(
            (self, next),
            (Queued, Delivered | Cancelled | Failed)
                | (Delivered, InProgress | Cancelled | Failed | NeedsAttention)
                | (InProgress, Done | Cancelled | Failed | NeedsAttention)
                | (
                    NeedsAttention,
                    Queued | InProgress | Done | Cancelled | Failed
                )
        )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) enum DeliveryState {
    Created,
    Queued,
    Delivered,
    Running,
    Completed,
    Cancelled,
    Expired,
    Failed,
    Unauthorized,
}

impl DeliveryState {
    pub(crate) fn can_transition_to(&self, next: &Self) -> bool {
        use DeliveryState::*;

        matches!(
            (self, next),
            (Created, Queued | Cancelled | Unauthorized)
                | (
                    Queued,
                    Delivered | Cancelled | Expired | Unauthorized | Failed
                )
                | (
                    Delivered,
                    Running | Cancelled | Expired | Unauthorized | Failed
                )
                | (Running, Completed | Cancelled | Failed | Unauthorized)
                | (Failed, Queued | Cancelled)
        )
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) enum EventType {
    #[serde(rename = "team.created")]
    TeamCreated,
    #[serde(rename = "member.added")]
    MemberAdded,
    #[serde(rename = "device.added")]
    DeviceAdded,
    #[serde(rename = "repository.approved")]
    RepositoryApproved,
    #[serde(rename = "permission.granted")]
    PermissionGranted,
    #[serde(rename = "permission.revoked")]
    PermissionRevoked,
    #[serde(rename = "task.created")]
    TaskCreated,
    #[serde(rename = "task.status_changed")]
    TaskStatusChanged,
    #[serde(rename = "task.follow_up_added")]
    TaskFollowUpAdded,
    #[serde(rename = "broadcast.created")]
    BroadcastCreated,
    #[serde(rename = "request.created")]
    RequestCreated,
    #[serde(rename = "request.queued")]
    RequestQueued,
    #[serde(rename = "request.delivered")]
    RequestDelivered,
    #[serde(rename = "request.started")]
    RequestStarted,
    #[serde(rename = "request.completed")]
    RequestCompleted,
    #[serde(rename = "request.failed")]
    RequestFailed,
    #[serde(rename = "request.cancelled")]
    RequestCancelled,
}

impl EventType {
    pub(crate) fn from_str(value: &str) -> Option<Self> {
        Some(match value {
            "team.created" => Self::TeamCreated,
            "member.added" => Self::MemberAdded,
            "device.added" => Self::DeviceAdded,
            "repository.approved" => Self::RepositoryApproved,
            "permission.granted" => Self::PermissionGranted,
            "permission.revoked" => Self::PermissionRevoked,
            "task.created" => Self::TaskCreated,
            "task.status_changed" => Self::TaskStatusChanged,
            "task.follow_up_added" => Self::TaskFollowUpAdded,
            "broadcast.created" => Self::BroadcastCreated,
            "request.created" => Self::RequestCreated,
            "request.queued" => Self::RequestQueued,
            "request.delivered" => Self::RequestDelivered,
            "request.started" => Self::RequestStarted,
            "request.completed" => Self::RequestCompleted,
            "request.failed" => Self::RequestFailed,
            "request.cancelled" => Self::RequestCancelled,
            _ => return None,
        })
    }

    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::TeamCreated => "team.created",
            Self::MemberAdded => "member.added",
            Self::DeviceAdded => "device.added",
            Self::RepositoryApproved => "repository.approved",
            Self::PermissionGranted => "permission.granted",
            Self::PermissionRevoked => "permission.revoked",
            Self::TaskCreated => "task.created",
            Self::TaskStatusChanged => "task.status_changed",
            Self::TaskFollowUpAdded => "task.follow_up_added",
            Self::BroadcastCreated => "broadcast.created",
            Self::RequestCreated => "request.created",
            Self::RequestQueued => "request.queued",
            Self::RequestDelivered => "request.delivered",
            Self::RequestStarted => "request.started",
            Self::RequestCompleted => "request.completed",
            Self::RequestFailed => "request.failed",
            Self::RequestCancelled => "request.cancelled",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct Event {
    pub event_id: String,
    pub team_id: String,
    pub actor_member_id: String,
    pub device_id: String,
    pub event_type: EventType,
    pub payload: Value,
    pub schema_version: u32,
    pub logical_time: u64,
    pub signature: Option<String>,
}

impl Event {
    pub(crate) fn create(
        team_id: impl Into<String>,
        actor_member_id: impl Into<String>,
        device_id: impl Into<String>,
        event_type: EventType,
        payload: Value,
        logical_time: u64,
        signature: impl Into<String>,
    ) -> Result<Self, DomainError> {
        let team_id = team_id.into();
        let actor_member_id = actor_member_id.into();
        let device_id = device_id.into();
        let event = Self {
            event_id: next_event_id(&device_id),
            team_id,
            actor_member_id,
            device_id,
            event_type,
            payload,
            schema_version: SUPPORTED_EVENT_SCHEMA_VERSION,
            logical_time,
            signature: Some(signature.into()),
        };
        event.validate()?;
        Ok(event)
    }

    pub(crate) fn validate(&self) -> Result<(), DomainError> {
        for (field, value) in [
            ("event_id", self.event_id.as_str()),
            ("team_id", self.team_id.as_str()),
            ("actor_member_id", self.actor_member_id.as_str()),
            ("device_id", self.device_id.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(DomainError::MissingField(field));
            }
        }

        if self.schema_version != SUPPORTED_EVENT_SCHEMA_VERSION {
            return Err(DomainError::InvalidSchemaVersion);
        }

        if self
            .signature
            .as_deref()
            .is_none_or(|signature| signature.trim().is_empty())
        {
            return Err(DomainError::MissingField("signature"));
        }

        let payload_bytes = serde_json::to_vec(&self.payload)
            .map_err(|_| DomainError::InvalidPayload("payload is not serializable"))?;
        if payload_bytes.len() > MAX_EVENT_PAYLOAD_BYTES {
            return Err(DomainError::PayloadTooLarge {
                actual: payload_bytes.len(),
                maximum: MAX_EVENT_PAYLOAD_BYTES,
            });
        }

        Ok(())
    }

    pub(crate) fn signing_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(&(
            &self.event_id,
            &self.team_id,
            &self.actor_member_id,
            &self.device_id,
            &self.event_type,
            &self.payload,
            self.schema_version,
            self.logical_time,
        ))
        .expect("event signing fields must be serializable")
    }
}

fn next_event_id(device_id: &str) -> String {
    next_unique_id("evt", device_id)
}

fn next_invitation_id(device_id: &str) -> String {
    next_unique_id("invite", device_id)
}

fn next_unique_id(prefix: &str, subject_id: &str) -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let sequence = EVENT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let process_id = std::process::id();
    let subject_component = subject_id
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("{prefix}-{timestamp:032x}-{process_id:08x}-{sequence:016x}-{subject_component}")
}

fn hash_pairing_code(pairing_code: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"team-assistant/pairing-code/v1");
    digest.update(pairing_code.trim().as_bytes());
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn validate_nonempty(value: &str, field: &'static str) -> Result<(), DomainError> {
    if value.trim().is_empty() {
        return Err(DomainError::MissingField(field));
    }
    Ok(())
}

pub(crate) fn validate_broadcast_scope(scope: &str) -> Result<(), DomainError> {
    if scope.as_bytes().len() > MAX_BROADCAST_SCOPE_BYTES {
        return Err(DomainError::PayloadTooLarge {
            actual: scope.as_bytes().len(),
            maximum: MAX_BROADCAST_SCOPE_BYTES,
        });
    }
    let valid = scope == "team"
        || scope
            .strip_prefix("group:")
            .is_some_and(|group_id| !group_id.trim().is_empty());
    if !valid {
        return Err(DomainError::InvalidPayload(
            "broadcast scope must be team or group:<group-id>",
        ));
    }
    Ok(())
}

pub(crate) fn validate_broadcast_message(message: &str) -> Result<(), DomainError> {
    validate_nonempty(message, "broadcast.message")?;
    if message.as_bytes().len() > MAX_BROADCAST_MESSAGE_BYTES {
        return Err(DomainError::PayloadTooLarge {
            actual: message.as_bytes().len(),
            maximum: MAX_BROADCAST_MESSAGE_BYTES,
        });
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DomainError {
    MissingField(&'static str),
    InvalidSchemaVersion,
    InvalidPayload(&'static str),
    PayloadTooLarge { actual: usize, maximum: usize },
    InvalidTransition { from: String, to: String },
}

impl fmt::Display for DomainError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingField(field) => write!(formatter, "missing required field: {field}"),
            Self::InvalidSchemaVersion => write!(
                formatter,
                "unsupported event schema version; only version {SUPPORTED_EVENT_SCHEMA_VERSION} is supported"
            ),
            Self::InvalidPayload(message) => formatter.write_str(message),
            Self::PayloadTooLarge { actual, maximum } => {
                write!(formatter, "event payload is {actual} bytes; maximum is {maximum}")
            }
            Self::InvalidTransition { from, to } => {
                write!(formatter, "invalid transition from {from} to {to}")
            }
        }
    }
}

impl std::error::Error for DomainError {}

pub(crate) fn validate_task_transition(
    current: &TaskStatus,
    next: &TaskStatus,
) -> Result<(), DomainError> {
    if current.can_transition_to(next) {
        Ok(())
    } else {
        Err(DomainError::InvalidTransition {
            from: format_task_status(current),
            to: format_task_status(next),
        })
    }
}

pub(crate) fn validate_delivery_transition(
    current: &DeliveryState,
    next: &DeliveryState,
) -> Result<(), DomainError> {
    if current.can_transition_to(next) {
        Ok(())
    } else {
        Err(DomainError::InvalidTransition {
            from: format_delivery_state(current),
            to: format_delivery_state(next),
        })
    }
}

fn format_task_status(status: &TaskStatus) -> String {
    format!("{status:?}")
}

fn format_delivery_state(state: &DeliveryState) -> String {
    format!("{state:?}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event(payload: Value) -> Event {
        Event {
            event_id: "event-1".into(),
            team_id: "team-1".into(),
            actor_member_id: "member-1".into(),
            device_id: "device-1".into(),
            event_type: EventType::TaskCreated,
            payload,
            schema_version: 1,
            logical_time: 1,
            signature: Some("test-signature".into()),
        }
    }

    #[test]
    fn task_state_machine_accepts_normal_progression() {
        assert!(validate_task_transition(&TaskStatus::Queued, &TaskStatus::Delivered).is_ok());
        assert!(validate_task_transition(&TaskStatus::Delivered, &TaskStatus::InProgress).is_ok());
        assert!(validate_task_transition(&TaskStatus::InProgress, &TaskStatus::Done).is_ok());
    }

    #[test]
    fn task_state_machine_rejects_resurrection_of_done_task() {
        let error = validate_task_transition(&TaskStatus::Done, &TaskStatus::InProgress)
            .expect_err("done tasks must be terminal");
        assert!(matches!(error, DomainError::InvalidTransition { .. }));
    }

    #[test]
    fn task_state_machine_rejects_skipped_and_backward_transitions() {
        for (current, next) in [
            (TaskStatus::Queued, TaskStatus::InProgress),
            (TaskStatus::Delivered, TaskStatus::Queued),
            (TaskStatus::InProgress, TaskStatus::Queued),
            (TaskStatus::Cancelled, TaskStatus::Queued),
            (TaskStatus::Failed, TaskStatus::Done),
        ] {
            assert!(
                validate_task_transition(&current, &next).is_err(),
                "unexpected task transition: {current:?} -> {next:?}"
            );
        }
    }

    #[test]
    fn task_state_machine_allows_needs_attention_recovery() {
        assert!(
            validate_task_transition(&TaskStatus::Delivered, &TaskStatus::NeedsAttention).is_ok()
        );
        assert!(validate_task_transition(&TaskStatus::NeedsAttention, &TaskStatus::Queued).is_ok());
    }

    #[test]
    fn broadcast_scope_and_message_validation_is_bounded() {
        assert!(validate_broadcast_scope("team").is_ok());
        assert!(validate_broadcast_scope("group:backend").is_ok());
        assert!(validate_broadcast_scope("group:").is_err());
        assert!(validate_broadcast_scope("workspace:backend").is_err());
        assert!(validate_broadcast_message("release status").is_ok());
        assert!(validate_broadcast_message("  ").is_err());
        let oversized = "x".repeat(MAX_BROADCAST_MESSAGE_BYTES + 1);
        assert!(matches!(
            validate_broadcast_message(&oversized),
            Err(DomainError::PayloadTooLarge { .. })
        ));
    }

    #[test]
    fn delivery_state_machine_accepts_queue_and_execution() {
        assert!(
            validate_delivery_transition(&DeliveryState::Created, &DeliveryState::Queued).is_ok()
        );
        assert!(
            validate_delivery_transition(&DeliveryState::Queued, &DeliveryState::Delivered).is_ok()
        );
        assert!(
            validate_delivery_transition(&DeliveryState::Delivered, &DeliveryState::Running)
                .is_ok()
        );
        assert!(
            validate_delivery_transition(&DeliveryState::Running, &DeliveryState::Completed)
                .is_ok()
        );
    }

    #[test]
    fn delivery_state_machine_rejects_completed_to_running() {
        assert!(
            validate_delivery_transition(&DeliveryState::Completed, &DeliveryState::Running)
                .is_err()
        );
    }

    #[test]
    fn delivery_state_machine_rejects_skipped_backward_and_terminal_transitions() {
        for (current, next) in [
            (DeliveryState::Created, DeliveryState::Delivered),
            (DeliveryState::Queued, DeliveryState::Running),
            (DeliveryState::Running, DeliveryState::Queued),
            (DeliveryState::Completed, DeliveryState::Queued),
            (DeliveryState::Unauthorized, DeliveryState::Queued),
        ] {
            assert!(
                validate_delivery_transition(&current, &next).is_err(),
                "unexpected delivery transition: {current:?} -> {next:?}"
            );
        }
    }

    #[test]
    fn delivery_state_machine_allows_exceptional_terminal_states() {
        for (current, next) in [
            (DeliveryState::Created, DeliveryState::Cancelled),
            (DeliveryState::Created, DeliveryState::Unauthorized),
            (DeliveryState::Queued, DeliveryState::Expired),
            (DeliveryState::Delivered, DeliveryState::Failed),
            (DeliveryState::Running, DeliveryState::Cancelled),
            (DeliveryState::Failed, DeliveryState::Queued),
        ] {
            assert!(
                validate_delivery_transition(&current, &next).is_ok(),
                "missing delivery transition: {current:?} -> {next:?}"
            );
        }
    }

    #[test]
    fn permission_evaluator_supports_delegation_and_revocation_cascade() {
        let resource = PermissionResource::Repository("repo-1".into());
        let action = PermissionAction::ReadRepository;
        let mut evaluator = PermissionEvaluator::new("manager");
        evaluator
            .add_grant(
                PermissionGrant {
                    grant_id: "grant-manager-lead".into(),
                    subject_member_id: "lead".into(),
                    resource: resource.clone(),
                    action: action.clone(),
                    grantor_member_id: "manager".into(),
                    delegable: true,
                    expires_at: None,
                    revoked: false,
                },
                10,
            )
            .unwrap();
        evaluator
            .add_grant(
                PermissionGrant {
                    grant_id: "grant-lead-implementer".into(),
                    subject_member_id: "implementer".into(),
                    resource: resource.clone(),
                    action: action.clone(),
                    grantor_member_id: "lead".into(),
                    delegable: false,
                    expires_at: Some(100),
                    revoked: false,
                },
                10,
            )
            .unwrap();
        assert!(evaluator.check("lead", &resource, &action, 10).allowed);
        assert_eq!(
            evaluator.check("lead", &resource, &action, 10).reason,
            PermissionDecisionReason::Allowed
        );
        assert_eq!(
            evaluator.check("implementer", &resource, &action, 10).chain,
            vec!["grant-manager-lead", "grant-lead-implementer"]
        );

        evaluator
            .revoke("grant-manager-lead", "manager", 11)
            .unwrap();
        assert!(!evaluator.check("lead", &resource, &action, 11).allowed);
        assert_eq!(
            evaluator.check("lead", &resource, &action, 11).reason,
            PermissionDecisionReason::GrantRevoked("grant-manager-lead".into())
        );
        assert!(
            !evaluator
                .check("implementer", &resource, &action, 11)
                .allowed
        );
    }

    #[test]
    fn permission_evaluator_rejects_nondelegable_grants_and_cycles() {
        let resource = PermissionResource::Repository("repo-1".into());
        let action = PermissionAction::ReadRepository;
        let mut evaluator = PermissionEvaluator::new("manager");
        evaluator
            .add_grant(
                PermissionGrant {
                    grant_id: "grant-manager-lead".into(),
                    subject_member_id: "lead".into(),
                    resource: resource.clone(),
                    action: action.clone(),
                    grantor_member_id: "manager".into(),
                    delegable: false,
                    expires_at: None,
                    revoked: false,
                },
                10,
            )
            .unwrap();
        assert!(matches!(
            evaluator.add_grant(
                PermissionGrant {
                    grant_id: "grant-lead-implementer".into(),
                    subject_member_id: "implementer".into(),
                    resource: resource.clone(),
                    action: action.clone(),
                    grantor_member_id: "lead".into(),
                    delegable: false,
                    expires_at: None,
                    revoked: false,
                },
                10,
            ),
            Err(PermissionError::GrantorUnauthorized(_))
        ));

        let mut cycle_evaluator = PermissionEvaluator::new("manager");
        cycle_evaluator
            .add_grant(
                PermissionGrant {
                    grant_id: "grant-manager-lead".into(),
                    subject_member_id: "lead".into(),
                    resource: resource.clone(),
                    action: action.clone(),
                    grantor_member_id: "manager".into(),
                    delegable: true,
                    expires_at: None,
                    revoked: false,
                },
                10,
            )
            .unwrap();
        assert!(matches!(
            cycle_evaluator.add_grant(
                PermissionGrant {
                    grant_id: "grant-lead-manager".into(),
                    subject_member_id: "manager".into(),
                    resource,
                    action,
                    grantor_member_id: "lead".into(),
                    delegable: true,
                    expires_at: None,
                    revoked: false,
                },
                10,
            ),
            Err(PermissionError::CycleDetected(_))
        ));
    }

    #[test]
    fn event_validation_accepts_bounded_serializable_payload() {
        let value = event(json!({"task_id": "task-1", "description": "inspect"}));
        assert!(value.validate().is_ok());
        assert_eq!(EventType::TaskCreated.as_str(), "task.created");
    }

    #[test]
    fn event_creation_generates_unique_signed_ids() {
        let first = Event::create(
            "team-1",
            "member-1",
            "device-1",
            EventType::TaskCreated,
            json!({"task_id": "task-1"}),
            1,
            "test-signature",
        )
        .unwrap();
        let second = Event::create(
            "team-1",
            "member-1",
            "device-1",
            EventType::TaskCreated,
            json!({"task_id": "task-2"}),
            2,
            "test-signature",
        )
        .unwrap();
        assert_ne!(first.event_id, second.event_id);
        assert!(first.signature.is_some());
        assert!(second.signature.is_some());
    }

    #[test]
    fn event_deserialization_accepts_allow_list_name() {
        let mut value = serde_json::to_value(event(json!({}))).unwrap();
        value["event_type"] = json!(EventType::TaskCreated.as_str());
        let parsed = serde_json::from_value::<Event>(value).unwrap();
        assert_eq!(parsed.event_type, EventType::TaskCreated);
    }

    #[test]
    fn event_validation_rejects_missing_identity() {
        let mut value = event(json!({}));
        value.device_id.clear();
        assert_eq!(
            value.validate(),
            Err(DomainError::MissingField("device_id"))
        );
    }

    #[test]
    fn event_validation_rejects_zero_schema_version() {
        let mut value = event(json!({}));
        value.schema_version = 0;
        assert_eq!(value.validate(), Err(DomainError::InvalidSchemaVersion));
    }

    #[test]
    fn event_validation_rejects_unknown_schema_version() {
        let mut value = event(json!({}));
        value.schema_version = SUPPORTED_EVENT_SCHEMA_VERSION + 1;
        assert_eq!(value.validate(), Err(DomainError::InvalidSchemaVersion));
    }

    #[test]
    fn event_deserialization_rejects_unknown_event_type() {
        let mut value = serde_json::to_value(event(json!({}))).unwrap();
        value["event_type"] = json!("task.unknown");
        assert!(serde_json::from_value::<Event>(value).is_err());
    }

    #[test]
    fn event_validation_rejects_oversized_payload() {
        let value = event(json!("x".repeat(MAX_EVENT_PAYLOAD_BYTES + 1)));
        assert!(matches!(
            value.validate(),
            Err(DomainError::PayloadTooLarge { .. })
        ));
    }
}
