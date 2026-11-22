//! SQLite storage for local team records.
//!
//! This layer owns migrations and persistence only. Domain transitions,
//! permission evaluation, synchronization, and agent execution remain outside
//! this module.

mod task_events;
mod membership_events;

use crate::identity::DeviceIdentity;
use crate::team::{
    validate_broadcast_message, validate_broadcast_scope, validate_delivery_transition,
    validate_task_transition, BroadcastRecord, DeliveryState, Device, Event, EventType, Member,
    OutboxRecord, OutboxState, PermissionAction, PermissionDecision, PermissionError,
    PermissionEvaluator, PermissionGrant, PermissionResource, RepositoryApproval, RequestKind,
    RequestRecord, RequestResult, TaskActivityRecord, TaskRecord, TaskStatus, Team, TeamInvitation,
};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, Transaction};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) const CURRENT_SCHEMA_VERSION: i64 = 6;
pub(crate) const MAX_OUTBOX_ATTEMPTS: u32 = 5;

const MIGRATION_1: &str = r#"
CREATE TABLE IF NOT EXISTS local_profile (
    profile_id INTEGER PRIMARY KEY CHECK (profile_id = 1),
    member_id TEXT NOT NULL,
    display_name TEXT NOT NULL,
    role_json TEXT NOT NULL,
    active INTEGER NOT NULL CHECK (active IN (0, 1)),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS teams (
    team_id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    authority_member_id TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS team_members (
    team_id TEXT NOT NULL,
    member_id TEXT NOT NULL,
    display_name TEXT NOT NULL,
    role_json TEXT NOT NULL,
    active INTEGER NOT NULL CHECK (active IN (0, 1)),
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (team_id, member_id)
);

CREATE TABLE IF NOT EXISTS devices (
    device_id TEXT PRIMARY KEY,
    member_id TEXT NOT NULL,
    public_key TEXT NOT NULL,
    active INTEGER NOT NULL CHECK (active IN (0, 1)),
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS repository_approvals (
    repository_id TEXT PRIMARY KEY,
    owner_member_id TEXT NOT NULL,
    local_path TEXT NOT NULL,
    approved_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS permission_grants (
    grant_id TEXT PRIMARY KEY,
    subject_member_id TEXT NOT NULL,
    resource_type TEXT NOT NULL,
    resource_id TEXT NOT NULL,
    action TEXT NOT NULL,
    grantor_member_id TEXT NOT NULL,
    delegable INTEGER NOT NULL CHECK (delegable IN (0, 1)),
    expires_at INTEGER,
    revoked INTEGER NOT NULL CHECK (revoked IN (0, 1)),
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS event_log (
    event_id TEXT PRIMARY KEY,
    team_id TEXT NOT NULL,
    actor_member_id TEXT NOT NULL,
    device_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    schema_version INTEGER NOT NULL,
    logical_time INTEGER NOT NULL,
    signature TEXT,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS tasks (
    task_id TEXT PRIMARY KEY,
    assigner_member_id TEXT NOT NULL,
    assignee_member_id TEXT NOT NULL,
    description TEXT NOT NULL,
    status_json TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS broadcasts (
    message_id TEXT PRIMARY KEY,
    sender_member_id TEXT NOT NULL,
    scope TEXT NOT NULL,
    message TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS requests (
    request_id TEXT PRIMARY KEY,
    requester_member_id TEXT NOT NULL,
    target_member_id TEXT NOT NULL,
    kind_json TEXT NOT NULL,
    payload TEXT NOT NULL,
    delivery_json TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS inbox_events (
    event_id TEXT PRIMARY KEY,
    received_at INTEGER NOT NULL,
    state TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS outbox_events (
    event_id TEXT PRIMARY KEY,
    attempts INTEGER NOT NULL DEFAULT 0,
    next_attempt_at INTEGER,
    state TEXT NOT NULL
);
"#;

const MIGRATION_2: &str = r#"
CREATE TABLE IF NOT EXISTS team_invitations (
    invitation_id TEXT PRIMARY KEY,
    team_id TEXT NOT NULL,
    team_name TEXT NOT NULL,
    authority_member_id TEXT NOT NULL,
    authority_device_id TEXT NOT NULL,
    authority_public_key TEXT NOT NULL,
    pairing_code_hash TEXT NOT NULL,
    signature TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    accepted_at INTEGER,
    revoked INTEGER NOT NULL CHECK (revoked IN (0, 1))
);
"#;

const MIGRATION_3: &str = r#"
ALTER TABLE permission_grants ADD COLUMN team_id TEXT NOT NULL DEFAULT '';
"#;

const MIGRATION_4: &str = r#"
CREATE TABLE IF NOT EXISTS task_activity (
    activity_id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL,
    actor_member_id TEXT NOT NULL,
    activity_type TEXT NOT NULL,
    message TEXT NOT NULL,
    status_json TEXT,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS task_activity_task_created
    ON task_activity(task_id, created_at, activity_id);
"#;

const MIGRATION_5: &str = r#"
ALTER TABLE requests ADD COLUMN result_json TEXT;
"#;

const MIGRATION_6: &str = r#"
CREATE TABLE relay_cursors (
    team_id TEXT PRIMARY KEY,
    sequence INTEGER NOT NULL CHECK (sequence >= 0),
    updated_at INTEGER NOT NULL
);
"#;

#[derive(Debug)]
pub(crate) struct TeamStore {
    path: PathBuf,
    connection: Connection,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StoredProfile {
    pub member: Member,
    pub created_at: u64,
}

#[derive(Debug)]
pub(crate) enum StorageError {
    Io(std::io::Error),
    Sql(rusqlite::Error),
    Serialization(serde_json::Error),
    Domain(crate::team::DomainError),
    Identity(crate::identity::IdentityError),
    Permission(PermissionError),
    UnsignedEvent,
    InvalidEventSignature,
    UnauthorizedEvent {
        actor_member_id: String,
        device_id: String,
    },
    ConflictingEvent {
        event_id: String,
    },
    InvalidInvitationSignature,
    PairingCodeMismatch,
    InvitationExpired,
    InvitationRevoked,
    InvitationAlreadyAccepted,
    MembershipConflict(&'static str),
    UnsupportedSchema(i64),
    OutboxNotFound(String),
}

impl fmt::Display for StorageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "storage I/O error: {error}"),
            Self::Sql(error) => write!(formatter, "SQLite error: {error}"),
            Self::Serialization(error) => write!(formatter, "serialization error: {error}"),
            Self::Domain(error) => write!(formatter, "invalid domain event: {error}"),
            Self::Identity(error) => write!(formatter, "identity error: {error}"),
            Self::Permission(error) => write!(formatter, "permission error: {error}"),
            Self::UnsignedEvent => formatter.write_str("event signature is required"),
            Self::InvalidEventSignature => {
                formatter.write_str("event signature verification failed")
            }
            Self::UnauthorizedEvent {
                actor_member_id,
                device_id,
            } => write!(
                formatter,
                "event actor {actor_member_id} or device {device_id} is not an active team member"
            ),
            Self::ConflictingEvent { event_id } => {
                write!(
                    formatter,
                    "event ID already exists with different contents: {event_id}"
                )
            }
            Self::InvalidInvitationSignature => {
                formatter.write_str("team invitation signature verification failed")
            }
            Self::PairingCodeMismatch => {
                formatter.write_str("team invitation pairing code is invalid")
            }
            Self::InvitationExpired => formatter.write_str("team invitation has expired"),
            Self::InvitationRevoked => formatter.write_str("team invitation has been revoked"),
            Self::InvitationAlreadyAccepted => {
                formatter.write_str("team invitation has already been accepted")
            }
            Self::MembershipConflict(field) => {
                write!(formatter, "team membership conflicts with existing {field}")
            }
            Self::UnsupportedSchema(version) => {
                write!(formatter, "unsupported SQLite schema version: {version}")
            }
            Self::OutboxNotFound(event_id) => {
                write!(formatter, "outbox event not found: {event_id}")
            }
        }
    }
}

impl std::error::Error for StorageError {}

impl From<std::io::Error> for StorageError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<rusqlite::Error> for StorageError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sql(error)
    }
}

impl From<serde_json::Error> for StorageError {
    fn from(error: serde_json::Error) -> Self {
        Self::Serialization(error)
    }
}

impl From<crate::team::DomainError> for StorageError {
    fn from(error: crate::team::DomainError) -> Self {
        Self::Domain(error)
    }
}

impl From<crate::identity::IdentityError> for StorageError {
    fn from(error: crate::identity::IdentityError) -> Self {
        Self::Identity(error)
    }
}

impl From<PermissionError> for StorageError {
    fn from(error: PermissionError) -> Self {
        Self::Permission(error)
    }
}

impl TeamStore {
    pub(crate) fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let path = path.as_ref().to_path_buf();
        if path.as_os_str() != ":memory:" {
            if let Some(parent) = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
        }

        let connection = Connection::open(&path)?;
        connection.execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;")?;
        let mut store = Self { path, connection };
        store.migrate()?;
        Ok(store)
    }

    /// Open an existing team database without creating files or applying migrations.
    pub(crate) fn open_read_only(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let path = path.as_ref().to_path_buf();
        let connection = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        connection.execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;")?;
        Ok(Self { path, connection })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn schema_version(&self) -> Result<i64, StorageError> {
        Ok(self
            .connection
            .query_row("SELECT version FROM schema_version LIMIT 1", [], |row| {
                row.get(0)
            })?)
    }

    pub(crate) fn save_profile(
        &self,
        member: &Member,
        created_at: u64,
        updated_at: u64,
    ) -> Result<(), StorageError> {
        self.connection.execute(
            "INSERT INTO local_profile
                (profile_id, member_id, display_name, role_json, active, created_at, updated_at)
             VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(profile_id) DO UPDATE SET
                member_id = excluded.member_id,
                display_name = excluded.display_name,
                role_json = excluded.role_json,
                active = excluded.active,
                updated_at = excluded.updated_at",
            params![
                member.member_id,
                member.display_name,
                encode(&member.role)?,
                bool_to_int(member.active),
                created_at,
                updated_at,
            ],
        )?;
        Ok(())
    }

    pub(crate) fn load_profile(&self) -> Result<Option<StoredProfile>, StorageError> {
        self.connection
            .query_row(
                "SELECT member_id, display_name, role_json, active, created_at
                 FROM local_profile WHERE profile_id = 1",
                [],
                |row| {
                    let role_json: String = row.get(2)?;
                    Ok(StoredProfile {
                        member: Member {
                            member_id: row.get(0)?,
                            display_name: row.get(1)?,
                            role: decode(&role_json).map_err(to_sql_error)?,
                            active: int_to_bool(row.get(3)?)?,
                        },
                        created_at: row.get(4)?,
                    })
                },
            )
            .optional()
            .map_err(StorageError::from)
    }

    pub(crate) fn members(&self) -> Result<Vec<Member>, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT display_name, member_id, role_json, active
             FROM team_members ORDER BY display_name, member_id",
        )?;
        let rows = statement.query_map([], |row| {
            let role_json: String = row.get(2)?;
            Ok(Member {
                member_id: row.get(1)?,
                display_name: row.get(0)?,
                role: decode(&role_json).map_err(to_sql_error)?,
                active: int_to_bool(row.get(3)?)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub(crate) fn save_team(&self, team: &Team) -> Result<(), StorageError> {
        self.connection.execute(
            "INSERT INTO teams (team_id, name, authority_member_id, created_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(team_id) DO UPDATE SET
                name = excluded.name,
                authority_member_id = excluded.authority_member_id,
                created_at = excluded.created_at",
            params![
                team.team_id,
                team.name,
                team.authority_member_id,
                team.created_at
            ],
        )?;
        Ok(())
    }

    pub(crate) fn team(&self, team_id: &str) -> Result<Option<Team>, StorageError> {
        self.connection
            .query_row(
                "SELECT team_id, name, authority_member_id, created_at FROM teams WHERE team_id = ?1",
                params![team_id],
                |row| Ok(Team {
                    team_id: row.get(0)?,
                    name: row.get(1)?,
                    authority_member_id: row.get(2)?,
                    created_at: row.get(3)?,
                }),
            )
            .optional()
            .map_err(StorageError::from)
    }

    pub(crate) fn teams(&self) -> Result<Vec<Team>, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT team_id, name, authority_member_id, created_at FROM teams ORDER BY created_at, team_id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(Team {
                team_id: row.get(0)?,
                name: row.get(1)?,
                authority_member_id: row.get(2)?,
                created_at: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub(crate) fn save_invitation(
        &self,
        invitation: &TeamInvitation,
        now: u64,
    ) -> Result<(), StorageError> {
        invitation.validate()?;
        if invitation.is_expired(now) {
            return Err(StorageError::InvitationExpired);
        }

        let authority = self
            .connection
            .query_row(
                "SELECT authority_member_id FROM teams WHERE team_id = ?1",
                params![invitation.team_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if authority.as_deref() != Some(invitation.authority_member_id.as_str()) {
            return Err(StorageError::MembershipConflict("team authority"));
        }

        let authority_key = self
            .connection
            .query_row(
                "SELECT public_key FROM devices
                 WHERE device_id = ?1 AND member_id = ?2 AND active = 1",
                params![
                    invitation.authority_device_id,
                    invitation.authority_member_id
                ],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if authority_key.as_deref() != Some(invitation.authority_public_key.as_str()) {
            return Err(StorageError::InvalidInvitationSignature);
        }
        DeviceIdentity::verify(
            &invitation.authority_public_key,
            &invitation.signing_bytes(),
            &invitation.signature,
        )
        .map_err(|_| StorageError::InvalidInvitationSignature)?;

        self.connection.execute(
            "INSERT INTO team_invitations
                (invitation_id, team_id, team_name, authority_member_id,
                 authority_device_id, authority_public_key, pairing_code_hash,
                 signature, created_at, expires_at, accepted_at, revoked)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL, 0)",
            params![
                invitation.invitation_id,
                invitation.team_id,
                invitation.team_name,
                invitation.authority_member_id,
                invitation.authority_device_id,
                invitation.authority_public_key,
                invitation.pairing_code_hash,
                invitation.signature,
                invitation.created_at,
                invitation.expires_at,
            ],
        )?;
        Ok(())
    }

    pub(crate) fn validate_approval_invitation(
        &self,
        invitation_id: &str,
        team_id: &str,
        requested_at: u64,
        now: u64,
    ) -> Result<bool, StorageError> {
        let record = self.connection.query_row(
            "SELECT team_id, created_at, expires_at, revoked FROM team_invitations WHERE invitation_id = ?1",
            params![invitation_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?, row.get::<_, u64>(2)?, row.get::<_, i64>(3)?)),
        ).optional()?;
        Ok(
            record.is_some_and(|(stored_team, created, expires, revoked)| {
                stored_team == team_id
                    && revoked == 0
                    && requested_at >= created
                    && requested_at <= now
                    && requested_at < expires
            }),
        )
    }

    pub(crate) fn accept_invitation(
        &mut self,
        invitation: &TeamInvitation,
        pairing_code: &str,
        authority_member: &Member,
        authority_device: &Device,
        joining_member: &Member,
        joining_device: &Device,
        joining_identity: &DeviceIdentity,
        now: u64,
    ) -> Result<(), StorageError> {
        invitation.validate()?;
        if invitation.is_expired(now) {
            return Err(StorageError::InvitationExpired);
        }
        if TeamInvitation::pairing_code_hash(pairing_code) != invitation.pairing_code_hash {
            return Err(StorageError::PairingCodeMismatch);
        }
        if authority_member.member_id != invitation.authority_member_id
            || !authority_member.active
            || authority_device.device_id != invitation.authority_device_id
            || authority_device.member_id != authority_member.member_id
            || !authority_device.active
            || authority_device.public_key != invitation.authority_public_key
        {
            return Err(StorageError::MembershipConflict("invitation authority"));
        }
        DeviceIdentity::verify(
            &invitation.authority_public_key,
            &invitation.signing_bytes(),
            &invitation.signature,
        )
        .map_err(|_| StorageError::InvalidInvitationSignature)?;

        if !joining_member.active
            || joining_device.member_id != joining_member.member_id
            || !joining_device.active
            || joining_device.device_id != joining_identity.device_id()
            || joining_device.public_key != joining_identity.public_key()
        {
            return Err(StorageError::MembershipConflict("joining device identity"));
        }

        let transaction = self.connection.transaction()?;
        let existing_invitation = transaction
            .query_row(
                "SELECT accepted_at, revoked FROM team_invitations
                 WHERE invitation_id = ?1",
                params![invitation.invitation_id],
                |row| Ok((row.get::<_, Option<u64>>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?;
        if let Some((accepted_at, revoked)) = existing_invitation {
            if revoked != 0 {
                return Err(StorageError::InvitationRevoked);
            }
            if accepted_at.is_some() {
                return Err(StorageError::InvitationAlreadyAccepted);
            }
        }

        ensure_team(&transaction, invitation, now)?;
        ensure_member(&transaction, &invitation.team_id, authority_member, now)?;
        ensure_member(&transaction, &invitation.team_id, joining_member, now)?;
        ensure_device(&transaction, authority_device, now)?;
        ensure_device(&transaction, joining_device, now)?;
        transaction.execute(
            "INSERT INTO team_invitations
                (invitation_id, team_id, team_name, authority_member_id,
                 authority_device_id, authority_public_key, pairing_code_hash,
                 signature, created_at, expires_at, accepted_at, revoked)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 0)",
            params![
                invitation.invitation_id,
                invitation.team_id,
                invitation.team_name,
                invitation.authority_member_id,
                invitation.authority_device_id,
                invitation.authority_public_key,
                invitation.pairing_code_hash,
                invitation.signature,
                invitation.created_at,
                invitation.expires_at,
                now,
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn save_member(
        &self,
        team_id: &str,
        member: &Member,
        updated_at: u64,
    ) -> Result<(), StorageError> {
        self.connection.execute(
            "INSERT INTO team_members
                (team_id, member_id, display_name, role_json, active, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(team_id, member_id) DO UPDATE SET
                display_name = excluded.display_name,
                role_json = excluded.role_json,
                active = excluded.active,
                updated_at = excluded.updated_at",
            params![
                team_id,
                member.member_id,
                member.display_name,
                encode(&member.role)?,
                bool_to_int(member.active),
                updated_at,
            ],
        )?;
        Ok(())
    }

    pub(crate) fn save_device(&self, device: &Device, updated_at: u64) -> Result<(), StorageError> {
        self.connection.execute(
            "INSERT INTO devices (device_id, member_id, public_key, active, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(device_id) DO UPDATE SET
                member_id = excluded.member_id,
                public_key = excluded.public_key,
                active = excluded.active,
                updated_at = excluded.updated_at",
            params![
                device.device_id,
                device.member_id,
                device.public_key,
                bool_to_int(device.active),
                updated_at,
            ],
        )?;
        Ok(())
    }

    pub(crate) fn grant_permission(
        &self,
        team_id: &str,
        grant: &PermissionGrant,
        now: u64,
    ) -> Result<(), StorageError> {
        let authority_member_id = self.team_authority(team_id)?;
        if !self.active_member_exists(team_id, &grant.subject_member_id)?
            || !self.active_member_exists(team_id, &grant.grantor_member_id)?
        {
            return Err(StorageError::Permission(PermissionError::InvalidGrant(
                "permission subject and grantor must be active team members",
            )));
        }
        let mut evaluator = self.load_permission_evaluator(team_id, &authority_member_id)?;
        evaluator.add_grant(grant.clone(), now)?;
        self.connection.execute(
            "INSERT INTO permission_grants
                (grant_id, team_id, subject_member_id, resource_type, resource_id,
                 action, grantor_member_id, delegable, expires_at, revoked, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                grant.grant_id,
                team_id,
                grant.subject_member_id,
                permission_resource_type(&grant.resource),
                encode(&grant.resource)?,
                encode(&grant.action)?,
                grant.grantor_member_id,
                bool_to_int(grant.delegable),
                grant.expires_at,
                bool_to_int(grant.revoked),
                now,
            ],
        )?;
        Ok(())
    }

    pub(crate) fn revoke_permission(
        &self,
        team_id: &str,
        grant_id: &str,
        actor_member_id: &str,
        now: u64,
    ) -> Result<(), StorageError> {
        let authority_member_id = self.team_authority(team_id)?;
        let mut evaluator = self.load_permission_evaluator(team_id, &authority_member_id)?;
        evaluator.revoke(grant_id, actor_member_id, now)?;
        self.connection.execute(
            "UPDATE permission_grants SET revoked = 1, updated_at = ?1
             WHERE team_id = ?2 AND grant_id = ?3",
            params![now, team_id, grant_id],
        )?;
        Ok(())
    }

    pub(crate) fn permission_decision(
        &self,
        team_id: &str,
        subject_member_id: &str,
        resource: &PermissionResource,
        action: &PermissionAction,
        now: u64,
    ) -> Result<PermissionDecision, StorageError> {
        let authority_member_id = self.team_authority(team_id)?;
        let evaluator = self.load_permission_evaluator(team_id, &authority_member_id)?;
        Ok(evaluator.check(subject_member_id, resource, action, now))
    }

    pub(crate) fn team_authority(&self, team_id: &str) -> Result<String, StorageError> {
        self.connection
            .query_row(
                "SELECT authority_member_id FROM teams WHERE team_id = ?1",
                params![team_id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(StorageError::MembershipConflict("team"))
    }

    fn active_member_exists(&self, team_id: &str, member_id: &str) -> Result<bool, StorageError> {
        let count: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM team_members
             WHERE team_id = ?1 AND member_id = ?2 AND active = 1",
            params![team_id, member_id],
            |row| row.get(0),
        )?;
        Ok(count == 1)
    }

    fn load_permission_evaluator(
        &self,
        team_id: &str,
        authority_member_id: &str,
    ) -> Result<PermissionEvaluator, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT grant_id, subject_member_id, resource_type, resource_id, action,
                    grantor_member_id, delegable, expires_at, revoked
             FROM permission_grants WHERE team_id = ?1 ORDER BY grant_id",
        )?;
        let rows = statement.query_map(params![team_id], |row| {
            let resource_json: String = row.get(3)?;
            let action_json: String = row.get(4)?;
            Ok(PermissionGrant {
                grant_id: row.get(0)?,
                subject_member_id: row.get(1)?,
                resource: decode(&resource_json).map_err(to_sql_error)?,
                action: decode(&action_json).map_err(to_sql_error)?,
                grantor_member_id: row.get(5)?,
                delegable: int_to_bool(row.get(6)?)?,
                expires_at: row.get(7)?,
                revoked: int_to_bool(row.get(8)?)?,
            })
        })?;
        let grants = rows.collect::<Result<Vec<_>, _>>()?;
        Ok(PermissionEvaluator::from_grants(
            authority_member_id,
            grants,
        ))
    }

    pub(crate) fn save_repository_approval(
        &self,
        approval: &RepositoryApproval,
    ) -> Result<(), StorageError> {
        self.connection.execute(
            "INSERT INTO repository_approvals
                (repository_id, owner_member_id, local_path, approved_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(repository_id) DO UPDATE SET
                owner_member_id = excluded.owner_member_id,
                local_path = excluded.local_path,
                approved_at = excluded.approved_at",
            params![
                approval.repository_id,
                approval.owner_member_id,
                approval.local_path,
                approval.approved_at,
            ],
        )?;
        Ok(())
    }

    pub(crate) fn repository_approval(
        &self,
        repository_id: &str,
    ) -> Result<Option<RepositoryApproval>, StorageError> {
        self.connection
            .query_row(
                "SELECT repository_id, owner_member_id, local_path, approved_at
                 FROM repository_approvals WHERE repository_id = ?1",
                params![repository_id],
                |row| {
                    Ok(RepositoryApproval {
                        repository_id: row.get(0)?,
                        owner_member_id: row.get(1)?,
                        local_path: row.get(2)?,
                        approved_at: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(StorageError::from)
    }

    pub(crate) fn repository_approvals(&self) -> Result<Vec<RepositoryApproval>, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT repository_id, owner_member_id, local_path, approved_at
             FROM repository_approvals ORDER BY approved_at, repository_id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(RepositoryApproval {
                repository_id: row.get(0)?,
                owner_member_id: row.get(1)?,
                local_path: row.get(2)?,
                approved_at: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub(crate) fn validate_verified_event(&self, event: &Event) -> Result<(), StorageError> {
        if event
            .signature
            .as_deref()
            .is_none_or(|signature| signature.trim().is_empty())
        {
            return Err(StorageError::UnsignedEvent);
        }
        event.validate()?;
        self.verify_event_signature(event)?;
        if event.event_type != EventType::TeamCreated && !self.is_authorized_actor(event)? {
            return Err(StorageError::UnauthorizedEvent {
                actor_member_id: event.actor_member_id.clone(),
                device_id: event.device_id.clone(),
            });
        }
        Ok(())
    }

    pub(crate) fn insert_event(
        &mut self,
        event: &Event,
        created_at: u64,
    ) -> Result<bool, StorageError> {
        self.insert_event_inner(event, created_at, false)
    }

    /// Insert an event received from a peer. Unlike the local command path,
    /// this path verifies the actor's Ed25519 signature against the active
    /// device record before allowing projection.
    pub(crate) fn insert_verified_event(
        &mut self,
        event: &Event,
        created_at: u64,
    ) -> Result<bool, StorageError> {
        self.insert_event_inner(event, created_at, true)
    }

    fn insert_event_inner(
        &mut self,
        event: &Event,
        created_at: u64,
        verify_signature: bool,
    ) -> Result<bool, StorageError> {
        if verify_signature || matches!(event.event_type, EventType::MemberAdded | EventType::DeviceAdded) {
            self.validate_verified_event(event)?;
        } else {
            if event
                .signature
                .as_deref()
                .is_none_or(|signature| signature.trim().is_empty())
            {
                return Err(StorageError::UnsignedEvent);
            }
            event.validate()?;
            if event.event_type != EventType::TeamCreated && !self.is_authorized_actor(event)? {
                return Err(StorageError::UnauthorizedEvent {
                    actor_member_id: event.actor_member_id.clone(),
                    device_id: event.device_id.clone(),
                });
            }
        }

        self.validate_task_event(event, created_at, verify_signature)?;
        let payload_json = serde_json::to_string(&event.payload)?;
        let transaction = self.connection.transaction()?;
        let changed = transaction.execute(
            "INSERT OR IGNORE INTO event_log
                (event_id, team_id, actor_member_id, device_id, event_type, payload_json,
                 schema_version, logical_time, signature, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                event.event_id,
                event.team_id,
                event.actor_member_id,
                event.device_id,
                event.event_type.as_str(),
                payload_json,
                event.schema_version,
                event.logical_time,
                event.signature,
                created_at,
            ],
        )?;
        if changed == 0 {
            let existing = transaction.query_row(
                "SELECT team_id, actor_member_id, device_id, event_type, payload_json,
                        schema_version, logical_time, signature
                 FROM event_log WHERE event_id = ?1",
                params![event.event_id],
                |row| {
                    let payload_json: String = row.get(4)?;
                    let payload: Value =
                        serde_json::from_str(&payload_json).map_err(to_sql_error)?;
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        payload,
                        row.get::<_, i64>(5)?,
                        row.get::<_, u64>(6)?,
                        row.get::<_, Option<String>>(7)?,
                    ))
                },
            )?;
            let identical = existing.0 == event.team_id
                && existing.1 == event.actor_member_id
                && existing.2 == event.device_id
                && existing.3 == event.event_type.as_str()
                && existing.4 == event.payload
                && existing.5 == i64::from(event.schema_version)
                && existing.6 == event.logical_time
                && existing.7 == event.signature;
            if identical {
                transaction.commit()?;
                return Ok(false);
            }
            return Err(StorageError::ConflictingEvent {
                event_id: event.event_id.clone(),
            });
        }

        project_event(&transaction, event, created_at)?;
        transaction.commit()?;
        Ok(true)
    }

    pub(crate) fn event_ids(&self, limit: usize) -> Result<Vec<String>, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT event_id FROM event_log
             ORDER BY logical_time, event_id LIMIT ?1",
        )?;
        let rows = statement.query_map(params![limit as u64], |row| row.get(0))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub(crate) fn events_by_ids(&self, event_ids: &[String]) -> Result<Vec<Event>, StorageError> {
        if event_ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut statement = self.connection.prepare(
            "SELECT event_id, team_id, actor_member_id, device_id, event_type,
                    payload_json, schema_version, logical_time, signature
             FROM event_log ORDER BY logical_time, event_id",
        )?;
        let wanted = event_ids.iter().collect::<std::collections::HashSet<_>>();
        let rows = statement.query_map([], |row| {
            let event_type: String = row.get(4)?;
            let event_type = EventType::from_str(&event_type).ok_or_else(|| {
                rusqlite::Error::InvalidColumnType(
                    4,
                    "event_type".into(),
                    rusqlite::types::Type::Text,
                )
            })?;
            let payload_json: String = row.get(5)?;
            Ok(Event {
                event_id: row.get(0)?,
                team_id: row.get(1)?,
                actor_member_id: row.get(2)?,
                device_id: row.get(3)?,
                event_type,
                payload: serde_json::from_str(&payload_json).map_err(to_sql_error)?,
                schema_version: row.get(6)?,
                logical_time: row.get(7)?,
                signature: row.get(8)?,
            })
        })?;
        let events = rows
            .filter_map(|row| match row {
                Ok(event) if wanted.contains(&event.event_id) => Some(Ok(event)),
                Ok(_) => None,
                Err(error) => Some(Err(error)),
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(events)
    }

    pub(crate) fn team_events(
        &self,
        team_id: &str,
        limit: usize,
    ) -> Result<Vec<Event>, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT event_id, team_id, actor_member_id, device_id, event_type,
                    payload_json, schema_version, logical_time, signature
             FROM event_log WHERE team_id = ?1 ORDER BY logical_time, event_id LIMIT ?2",
        )?;
        let rows = statement.query_map(params![team_id, limit as u64], |row| {
            let event_type: String = row.get(4)?;
            let event_type = EventType::from_str(&event_type).ok_or_else(|| {
                rusqlite::Error::InvalidColumnType(
                    4,
                    "event_type".into(),
                    rusqlite::types::Type::Text,
                )
            })?;
            let payload_json: String = row.get(5)?;
            Ok(Event {
                event_id: row.get(0)?,
                team_id: row.get(1)?,
                actor_member_id: row.get(2)?,
                device_id: row.get(3)?,
                event_type,
                payload: serde_json::from_str(&payload_json).map_err(to_sql_error)?,
                schema_version: row.get(6)?,
                logical_time: row.get(7)?,
                signature: row.get(8)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub(crate) fn device_public_key(
        &self,
        device_id: &str,
    ) -> Result<Option<String>, StorageError> {
        self.connection
            .query_row(
                "SELECT public_key FROM devices WHERE device_id = ?1 AND active = 1",
                params![device_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(StorageError::from)
    }

    pub(crate) fn relay_cursor(&self, team_id: &str) -> Result<u64, StorageError> {
        Ok(self
            .connection
            .query_row(
                "SELECT sequence FROM relay_cursors WHERE team_id = ?1",
                params![team_id],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(0))
    }

    pub(crate) fn save_relay_cursor(
        &self,
        team_id: &str,
        sequence: u64,
        updated_at: u64,
    ) -> Result<(), StorageError> {
        self.connection.execute(
            "INSERT INTO relay_cursors (team_id, sequence, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(team_id) DO UPDATE SET sequence = MAX(sequence, excluded.sequence), updated_at = excluded.updated_at",
            params![team_id, sequence, updated_at],
        )?;
        Ok(())
    }

    pub(crate) fn event_count(&self) -> Result<u64, StorageError> {
        Ok(self
            .connection
            .query_row("SELECT COUNT(*) FROM event_log", [], |row| row.get(0))?)
    }

    pub(crate) fn task_count(&self) -> Result<u64, StorageError> {
        Ok(self
            .connection
            .query_row("SELECT COUNT(*) FROM tasks", [], |row| row.get(0))?)
    }

    pub(crate) fn task(&self, task_id: &str) -> Result<Option<TaskRecord>, StorageError> {
        self.connection
            .query_row(
                "SELECT task_id, assigner_member_id, assignee_member_id, description,
                        status_json, created_at
                 FROM tasks WHERE task_id = ?1",
                params![task_id],
                |row| {
                    let status_json: String = row.get(4)?;
                    Ok(TaskRecord {
                        task_id: row.get(0)?,
                        assigner_member_id: row.get(1)?,
                        assignee_member_id: row.get(2)?,
                        description: row.get(3)?,
                        status: decode(&status_json).map_err(to_sql_error)?,
                        created_at: row.get(5)?,
                    })
                },
            )
            .optional()
            .map_err(StorageError::from)
    }

    pub(crate) fn tasks(&self) -> Result<Vec<TaskRecord>, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT task_id, assigner_member_id, assignee_member_id, description,
                    status_json, created_at
             FROM tasks ORDER BY created_at, task_id",
        )?;
        let rows = statement.query_map([], |row| {
            let status_json: String = row.get(4)?;
            Ok(TaskRecord {
                task_id: row.get(0)?,
                assigner_member_id: row.get(1)?,
                assignee_member_id: row.get(2)?,
                description: row.get(3)?,
                status: decode(&status_json).map_err(to_sql_error)?,
                created_at: row.get(5)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub(crate) fn task_activity(
        &self,
        task_id: &str,
    ) -> Result<Vec<TaskActivityRecord>, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT activity_id, task_id, actor_member_id, activity_type, message,
                    status_json, created_at
             FROM task_activity WHERE task_id = ?1
             ORDER BY created_at, activity_id",
        )?;
        let rows = statement.query_map(params![task_id], |row| {
            let status_json: Option<String> = row.get(5)?;
            Ok(TaskActivityRecord {
                activity_id: row.get(0)?,
                task_id: row.get(1)?,
                actor_member_id: row.get(2)?,
                activity_type: row.get(3)?,
                message: row.get(4)?,
                status: status_json
                    .as_deref()
                    .map(decode)
                    .transpose()
                    .map_err(to_sql_error)?,
                created_at: row.get(6)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub(crate) fn broadcast_count(&self) -> Result<u64, StorageError> {
        Ok(self
            .connection
            .query_row("SELECT COUNT(*) FROM broadcasts", [], |row| row.get(0))?)
    }

    pub(crate) fn broadcast(
        &self,
        message_id: &str,
    ) -> Result<Option<BroadcastRecord>, StorageError> {
        self.connection
            .query_row(
                "SELECT message_id, sender_member_id, scope, message, created_at
                 FROM broadcasts WHERE message_id = ?1",
                params![message_id],
                |row| {
                    Ok(BroadcastRecord {
                        message_id: row.get(0)?,
                        sender_member_id: row.get(1)?,
                        scope: row.get(2)?,
                        message: row.get(3)?,
                        created_at: row.get(4)?,
                    })
                },
            )
            .optional()
            .map_err(StorageError::from)
    }

    pub(crate) fn broadcasts(
        &self,
        scope: Option<&str>,
    ) -> Result<Vec<BroadcastRecord>, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT message_id, sender_member_id, scope, message, created_at
             FROM broadcasts
             WHERE (?1 IS NULL OR scope = ?1)
             ORDER BY created_at, message_id",
        )?;
        let rows = statement.query_map(params![scope], |row| {
            Ok(BroadcastRecord {
                message_id: row.get(0)?,
                sender_member_id: row.get(1)?,
                scope: row.get(2)?,
                message: row.get(3)?,
                created_at: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub(crate) fn requests(&self) -> Result<Vec<RequestRecord>, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT request_id, requester_member_id, target_member_id, kind_json,
                    payload, delivery_json, created_at, result_json
             FROM requests ORDER BY created_at, request_id",
        )?;
        let rows = statement.query_map([], |row| {
            let kind_json: String = row.get(3)?;
            let delivery_json: String = row.get(5)?;
            Ok(RequestRecord {
                request_id: row.get(0)?,
                requester_member_id: row.get(1)?,
                target_member_id: row.get(2)?,
                kind: decode(&kind_json).map_err(to_sql_error)?,
                payload: row.get(4)?,
                delivery: decode(&delivery_json).map_err(to_sql_error)?,
                created_at: row.get(6)?,
                result: row
                    .get::<_, Option<String>>(7)?
                    .map(|value| decode(&value).map_err(to_sql_error))
                    .transpose()?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub(crate) fn request(&self, request_id: &str) -> Result<Option<RequestRecord>, StorageError> {
        self.connection
            .query_row(
                "SELECT request_id, requester_member_id, target_member_id, kind_json,
                        payload, delivery_json, created_at, result_json
                 FROM requests WHERE request_id = ?1",
                params![request_id],
                |row| {
                    let kind_json: String = row.get(3)?;
                    let delivery_json: String = row.get(5)?;
                    Ok(RequestRecord {
                        request_id: row.get(0)?,
                        requester_member_id: row.get(1)?,
                        target_member_id: row.get(2)?,
                        kind: decode(&kind_json).map_err(to_sql_error)?,
                        payload: row.get(4)?,
                        delivery: decode(&delivery_json).map_err(to_sql_error)?,
                        created_at: row.get(6)?,
                        result: row
                            .get::<_, Option<String>>(7)?
                            .map(|value| decode(&value).map_err(to_sql_error))
                            .transpose()?,
                    })
                },
            )
            .optional()
            .map_err(StorageError::from)
    }

    pub(crate) fn enqueue_outbox(
        &self,
        event_id: &str,
        next_attempt_at: u64,
    ) -> Result<(), StorageError> {
        self.connection.execute(
            "INSERT INTO outbox_events (event_id, attempts, next_attempt_at, state)
             VALUES (?1, 0, ?2, 'queued')
             ON CONFLICT(event_id) DO NOTHING",
            params![event_id, next_attempt_at],
        )?;
        Ok(())
    }

    pub(crate) fn outbox(&self) -> Result<Vec<OutboxRecord>, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT event_id, attempts, next_attempt_at, state
             FROM outbox_events ORDER BY COALESCE(next_attempt_at, 0), event_id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(OutboxRecord {
                event_id: row.get(0)?,
                attempts: row.get(1)?,
                next_attempt_at: row.get(2)?,
                state: outbox_state(row.get::<_, String>(3)?.as_str())?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub(crate) fn claim_outbox(
        &mut self,
        now: u64,
        limit: usize,
    ) -> Result<Vec<OutboxRecord>, StorageError> {
        let transaction = self.connection.transaction()?;
        let mut statement = transaction.prepare(
            "SELECT event_id, attempts, next_attempt_at
             FROM outbox_events
             WHERE state = 'queued' AND (next_attempt_at IS NULL OR next_attempt_at <= ?1)
             ORDER BY COALESCE(next_attempt_at, 0), event_id LIMIT ?2",
        )?;
        let candidates = statement
            .query_map(params![now, limit as u64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, u32>(1)?,
                    row.get::<_, Option<u64>>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(statement);
        let mut claimed = Vec::with_capacity(candidates.len());
        for (event_id, attempts, next_attempt_at) in candidates {
            if attempts >= MAX_OUTBOX_ATTEMPTS {
                transaction.execute(
                    "UPDATE outbox_events SET state = 'failed' WHERE event_id = ?1",
                    params![event_id],
                )?;
                continue;
            }
            transaction.execute(
                "UPDATE outbox_events
                 SET attempts = attempts + 1, state = 'in_flight'
                 WHERE event_id = ?1 AND state = 'queued'",
                params![event_id],
            )?;
            claimed.push(OutboxRecord {
                event_id,
                attempts: attempts + 1,
                next_attempt_at,
                state: OutboxState::InFlight,
            });
        }
        transaction.commit()?;
        Ok(claimed)
    }

    pub(crate) fn acknowledge_outbox(&self, event_id: &str) -> Result<(), StorageError> {
        let changed = self.connection.execute(
            "UPDATE outbox_events SET state = 'delivered', next_attempt_at = NULL
             WHERE event_id = ?1 AND state = 'in_flight'",
            params![event_id],
        )?;
        if changed == 0 {
            return Err(StorageError::OutboxNotFound(event_id.into()));
        }
        Ok(())
    }

    pub(crate) fn fail_outbox(
        &self,
        event_id: &str,
        retry_at: u64,
    ) -> Result<OutboxState, StorageError> {
        let attempts = self
            .connection
            .query_row(
                "SELECT attempts FROM outbox_events
                 WHERE event_id = ?1 AND state = 'in_flight'",
                params![event_id],
                |row| row.get::<_, u32>(0),
            )
            .optional()?
            .ok_or_else(|| StorageError::OutboxNotFound(event_id.into()))?;
        let state = if attempts >= MAX_OUTBOX_ATTEMPTS {
            OutboxState::Failed
        } else {
            OutboxState::Queued
        };
        self.connection.execute(
            "UPDATE outbox_events SET state = ?1, next_attempt_at = ?2
             WHERE event_id = ?3 AND state = 'in_flight'",
            params![
                outbox_state_name(state),
                (state == OutboxState::Queued).then_some(retry_at),
                event_id
            ],
        )?;
        Ok(state)
    }

    pub(crate) fn cancel_outbox(&self, event_id: &str) -> Result<(), StorageError> {
        let changed = self.connection.execute(
            "UPDATE outbox_events SET state = 'cancelled', next_attempt_at = NULL
             WHERE event_id = ?1 AND state IN ('queued', 'in_flight')",
            params![event_id],
        )?;
        if changed == 0 {
            return Err(StorageError::OutboxNotFound(event_id.into()));
        }
        Ok(())
    }

    fn is_authorized_actor(&self, event: &Event) -> Result<bool, StorageError> {
        let active_member: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM team_members
             WHERE team_id = ?1 AND member_id = ?2 AND active = 1",
            params![event.team_id, event.actor_member_id],
            |row| row.get(0),
        )?;
        let active_device: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM devices
             WHERE device_id = ?1 AND member_id = ?2 AND active = 1",
            params![event.device_id, event.actor_member_id],
            |row| row.get(0),
        )?;
        Ok(active_member == 1 && active_device == 1)
    }

    fn verify_event_signature(&self, event: &Event) -> Result<(), StorageError> {
        let public_key = self
            .connection
            .query_row(
                "SELECT public_key FROM devices
                 WHERE device_id = ?1 AND member_id = ?2 AND active = 1",
                params![event.device_id, event.actor_member_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let Some(public_key) = public_key else {
            return Err(StorageError::UnauthorizedEvent {
                actor_member_id: event.actor_member_id.clone(),
                device_id: event.device_id.clone(),
            });
        };
        let Some(signature) = event.signature.as_deref() else {
            return Err(StorageError::UnsignedEvent);
        };
        DeviceIdentity::verify(&public_key, &event.signing_bytes(), signature)
            .map_err(|_| StorageError::InvalidEventSignature)
    }

    fn migrate(&mut self) -> Result<(), StorageError> {
        self.connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_version (
                version INTEGER NOT NULL CHECK (version >= 1)
            );",
        )?;

        let current: Option<i64> = self
            .connection
            .query_row("SELECT version FROM schema_version LIMIT 1", [], |row| {
                row.get(0)
            })
            .optional()?;

        if let Some(version) = current {
            if version > CURRENT_SCHEMA_VERSION {
                return Err(StorageError::UnsupportedSchema(version));
            }
        }

        let starting_version = current.unwrap_or(0);
        let transaction = self.connection.transaction()?;
        transaction.execute_batch(MIGRATION_1)?;
        transaction.execute_batch(MIGRATION_2)?;
        if starting_version < 3 {
            transaction.execute_batch(MIGRATION_3)?;
        }
        if starting_version < 4 {
            transaction.execute_batch(MIGRATION_4)?;
        }
        if starting_version < 5 {
            transaction.execute_batch(MIGRATION_5)?;
        }
        if starting_version < 6 {
            transaction.execute_batch(MIGRATION_6)?;
        }
        transaction.execute(
            "INSERT INTO schema_version(version)
             SELECT ?1 WHERE NOT EXISTS (SELECT 1 FROM schema_version)",
            params![CURRENT_SCHEMA_VERSION],
        )?;
        transaction.execute(
            "UPDATE schema_version SET version = ?1 WHERE version < ?1",
            params![CURRENT_SCHEMA_VERSION],
        )?;
        transaction.commit()?;
        Ok(())
    }
}

fn ensure_team(
    transaction: &Transaction<'_>,
    invitation: &TeamInvitation,
    created_at: u64,
) -> Result<(), StorageError> {
    let existing = transaction
        .query_row(
            "SELECT name, authority_member_id FROM teams WHERE team_id = ?1",
            params![invitation.team_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    if let Some((name, authority_member_id)) = existing {
        if name != invitation.team_name || authority_member_id != invitation.authority_member_id {
            return Err(StorageError::MembershipConflict("team"));
        }
        return Ok(());
    }
    transaction.execute(
        "INSERT INTO teams (team_id, name, authority_member_id, created_at)
         VALUES (?1, ?2, ?3, ?4)",
        params![
            invitation.team_id,
            invitation.team_name,
            invitation.authority_member_id,
            created_at,
        ],
    )?;
    Ok(())
}

fn ensure_member(
    transaction: &Transaction<'_>,
    team_id: &str,
    member: &Member,
    updated_at: u64,
) -> Result<(), StorageError> {
    let role_json = encode(&member.role)?;
    let existing = transaction
        .query_row(
            "SELECT display_name, role_json, active FROM team_members
             WHERE team_id = ?1 AND member_id = ?2",
            params![team_id, member.member_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .optional()?;
    if let Some((display_name, stored_role_json, active)) = existing {
        if display_name != member.display_name
            || stored_role_json != role_json
            || active != bool_to_int(member.active)
        {
            return Err(StorageError::MembershipConflict("member"));
        }
        return Ok(());
    }
    transaction.execute(
        "INSERT INTO team_members
            (team_id, member_id, display_name, role_json, active, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            team_id,
            member.member_id,
            member.display_name,
            role_json,
            bool_to_int(member.active),
            updated_at,
        ],
    )?;
    Ok(())
}

fn ensure_device(
    transaction: &Transaction<'_>,
    device: &Device,
    updated_at: u64,
) -> Result<(), StorageError> {
    let existing = transaction
        .query_row(
            "SELECT member_id, public_key, active FROM devices WHERE device_id = ?1",
            params![device.device_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .optional()?;
    if let Some((member_id, public_key, active)) = existing {
        if member_id != device.member_id
            || public_key != device.public_key
            || active != bool_to_int(device.active)
        {
            return Err(StorageError::MembershipConflict("device"));
        }
        return Ok(());
    }
    transaction.execute(
        "INSERT INTO devices (device_id, member_id, public_key, active, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            device.device_id,
            device.member_id,
            device.public_key,
            bool_to_int(device.active),
            updated_at,
        ],
    )?;
    Ok(())
}

#[derive(serde::Deserialize)]
struct TaskCreatedPayload {
    task_id: String,
    assigner_member_id: String,
    assignee_member_id: String,
    description: String,
    created_at: u64,
}

#[derive(serde::Deserialize)]
struct TaskStatusChangedPayload {
    task_id: String,
    status: TaskStatus,
}

#[derive(serde::Deserialize)]
struct TaskFollowUpPayload {
    task_id: String,
    message: String,
    created_at: u64,
}

#[derive(serde::Deserialize)]
struct BroadcastCreatedPayload {
    message_id: String,
    sender_member_id: String,
    scope: String,
    message: String,
    created_at: u64,
}

#[derive(serde::Deserialize)]
struct RequestCreatedPayload {
    request_id: String,
    requester_member_id: String,
    target_member_id: String,
    kind: RequestKind,
    payload: String,
    created_at: u64,
}

#[derive(serde::Deserialize)]
struct RequestLifecyclePayload {
    request_id: String,
    #[serde(default)]
    result: Option<RequestResult>,
}

fn project_event(
    transaction: &Transaction<'_>,
    event: &Event,
    received_at: u64,
) -> Result<(), StorageError> {
    if membership_events::apply(transaction, event, received_at)? { return Ok(()); }
    match event.event_type {
        EventType::TaskCreated => {
            let payload: TaskCreatedPayload = serde_json::from_value(event.payload.clone())?;
            validate_payload_text(&payload.task_id, "task.task_id")?;
            validate_payload_text(&payload.assigner_member_id, "task.assigner_member_id")?;
            validate_payload_text(&payload.assignee_member_id, "task.assignee_member_id")?;
            validate_payload_text(&payload.description, "task.description")?;
            transaction.execute(
                "INSERT INTO tasks
                    (task_id, assigner_member_id, assignee_member_id, description,
                     status_json, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    &payload.task_id,
                    &payload.assigner_member_id,
                    &payload.assignee_member_id,
                    &payload.description,
                    encode(&TaskStatus::Queued)?,
                    payload.created_at,
                    received_at,
                ],
            )?;
            transaction.execute(
                "INSERT INTO task_activity
                    (activity_id, task_id, actor_member_id, activity_type, message,
                     status_json, created_at)
                 VALUES (?1, ?2, ?3, 'created', ?4, ?5, ?6)",
                params![
                    event.event_id,
                    &payload.task_id,
                    event.actor_member_id,
                    &payload.description,
                    encode(&TaskStatus::Queued)?,
                    payload.created_at,
                ],
            )?;
        }
        EventType::TaskStatusChanged => {
            let payload: TaskStatusChangedPayload = serde_json::from_value(event.payload.clone())?;
            validate_payload_text(&payload.task_id, "task.task_id")?;
            task_events::rebuild_status(transaction, &payload.task_id, &event.team_id, received_at)?;
            transaction.execute(
                "INSERT INTO task_activity
                    (activity_id, task_id, actor_member_id, activity_type, message,
                     status_json, created_at)
                 VALUES (?1, ?2, ?3, 'status_changed', ?4, ?5, ?6)",
                params![
                    event.event_id,
                    &payload.task_id,
                    event.actor_member_id,
                    format!("status changed to {:?}", payload.status),
                    encode(&payload.status)?,
                    received_at,
                ],
            )?;
        }
        EventType::TaskFollowUpAdded => {
            let payload: TaskFollowUpPayload = serde_json::from_value(event.payload.clone())?;
            validate_payload_text(&payload.task_id, "task.task_id")?;
            validate_payload_text(&payload.message, "task.follow_up_message")?;
            transaction.query_row(
                "SELECT 1 FROM tasks WHERE task_id = ?1",
                params![&payload.task_id],
                |_| Ok(()),
            )?;
            transaction.execute(
                "INSERT INTO task_activity
                    (activity_id, task_id, actor_member_id, activity_type, message,
                     status_json, created_at)
                 VALUES (?1, ?2, ?3, 'follow_up', ?4, NULL, ?5)",
                params![
                    event.event_id,
                    &payload.task_id,
                    event.actor_member_id,
                    &payload.message,
                    payload.created_at,
                ],
            )?;
        }
        EventType::BroadcastCreated => {
            let payload: BroadcastCreatedPayload = serde_json::from_value(event.payload.clone())?;
            validate_payload_text(&payload.message_id, "broadcast.message_id")?;
            validate_payload_text(&payload.sender_member_id, "broadcast.sender_member_id")?;
            validate_broadcast_scope(&payload.scope)?;
            validate_broadcast_message(&payload.message)?;
            if payload.sender_member_id != event.actor_member_id {
                return Err(StorageError::Domain(
                    crate::team::DomainError::InvalidPayload(
                        "broadcast sender must match event actor",
                    ),
                ));
            }
            transaction.execute(
                "INSERT INTO broadcasts
                    (message_id, sender_member_id, scope, message, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    payload.message_id,
                    payload.sender_member_id,
                    payload.scope,
                    payload.message,
                    payload.created_at,
                ],
            )?;
        }
        EventType::RequestCreated => {
            let payload: RequestCreatedPayload = serde_json::from_value(event.payload.clone())?;
            validate_payload_text(&payload.request_id, "request.request_id")?;
            validate_payload_text(&payload.requester_member_id, "request.requester_member_id")?;
            validate_payload_text(&payload.target_member_id, "request.target_member_id")?;
            validate_payload_text(&payload.payload, "request.payload")?;
            transaction.execute(
                "INSERT INTO requests
                    (request_id, requester_member_id, target_member_id, kind_json, payload,
                     delivery_json, created_at, updated_at, result_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL)",
                params![
                    payload.request_id,
                    payload.requester_member_id,
                    payload.target_member_id,
                    encode(&payload.kind)?,
                    payload.payload,
                    encode(&DeliveryState::Created)?,
                    payload.created_at,
                    received_at,
                ],
            )?;
        }
        EventType::RequestQueued
        | EventType::RequestDelivered
        | EventType::RequestStarted
        | EventType::RequestCompleted
        | EventType::RequestFailed
        | EventType::RequestCancelled => {
            let payload: RequestLifecyclePayload = serde_json::from_value(event.payload.clone())?;
            validate_payload_text(&payload.request_id, "request.request_id")?;
            let current_json: String = transaction.query_row(
                "SELECT delivery_json FROM requests WHERE request_id = ?1",
                params![payload.request_id],
                |row| row.get(0),
            )?;
            let current: DeliveryState = decode(&current_json)?;
            let next = match event.event_type {
                EventType::RequestQueued => DeliveryState::Queued,
                EventType::RequestDelivered => DeliveryState::Delivered,
                EventType::RequestStarted => DeliveryState::Running,
                EventType::RequestCompleted => DeliveryState::Completed,
                EventType::RequestFailed => DeliveryState::Failed,
                EventType::RequestCancelled => DeliveryState::Cancelled,
                _ => unreachable!(),
            };
            validate_delivery_transition(&current, &next)?;
            let result_json = if matches!(
                event.event_type,
                EventType::RequestCompleted | EventType::RequestFailed
            ) {
                payload.result.as_ref().map(encode).transpose()?
            } else {
                None
            };
            transaction.execute(
                "UPDATE requests
                 SET delivery_json = ?1, updated_at = ?2,
                     result_json = COALESCE(?3, result_json)
                 WHERE request_id = ?4",
                params![encode(&next)?, received_at, result_json, payload.request_id],
            )?;
        }
        EventType::TeamCreated
        | EventType::MemberAdded
        | EventType::DeviceAdded
        | EventType::RepositoryApproved
        | EventType::PermissionGranted
        | EventType::PermissionRevoked => {}
    }
    Ok(())
}

fn outbox_state_name(state: OutboxState) -> &'static str {
    match state {
        OutboxState::Queued => "queued",
        OutboxState::InFlight => "in_flight",
        OutboxState::Delivered => "delivered",
        OutboxState::Failed => "failed",
        OutboxState::Cancelled => "cancelled",
    }
}

fn outbox_state(value: &str) -> Result<OutboxState, rusqlite::Error> {
    match value {
        "queued" => Ok(OutboxState::Queued),
        "in_flight" => Ok(OutboxState::InFlight),
        "delivered" => Ok(OutboxState::Delivered),
        "failed" => Ok(OutboxState::Failed),
        "cancelled" => Ok(OutboxState::Cancelled),
        other => Err(rusqlite::Error::FromSqlConversionFailure(
            3,
            rusqlite::types::Type::Text,
            format!("unknown outbox state: {other}").into(),
        )),
    }
}

fn validate_payload_text(value: &str, field: &'static str) -> Result<(), StorageError> {
    if value.trim().is_empty() {
        return Err(StorageError::Domain(
            crate::team::DomainError::MissingField(field),
        ));
    }
    Ok(())
}

fn permission_resource_type(resource: &PermissionResource) -> &'static str {
    match resource {
        PermissionResource::Team(_) => "team",
        PermissionResource::Group(_) => "group",
        PermissionResource::Repository(_) => "repository",
        PermissionResource::Task(_) => "task",
        PermissionResource::MessageScope(_) => "message_scope",
    }
}

fn encode<T: Serialize>(value: &T) -> Result<String, StorageError> {
    Ok(serde_json::to_string(value)?)
}

fn decode<T: DeserializeOwned>(value: &str) -> Result<T, serde_json::Error> {
    serde_json::from_str(value)
}

fn to_sql_error(error: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
}

fn bool_to_int(value: bool) -> i64 {
    i64::from(value)
}

fn int_to_bool(value: i64) -> Result<bool, rusqlite::Error> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        other => Err(rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Integer,
            format!("expected boolean integer, got {other}").into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::DeviceIdentity;
    use crate::team::{
        Device, EventType, PermissionAction, PermissionDecisionReason, PermissionError,
        PermissionGrant, PermissionResource, Role, Team, TeamInvitation,
    };
    use serde_json::json;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn db_path(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "team-assistant-{label}-{}-{nanos}.sqlite",
            std::process::id()
        ))
    }

    #[test]
    fn relay_cursor_is_persistent_and_never_moves_backwards() {
        let path = db_path("relay-cursor");
        {
            let store = TeamStore::open(&path).unwrap();
            assert_eq!(store.relay_cursor("team-1").unwrap(), 0);
            store.save_relay_cursor("team-1", 12, 100).unwrap();
            store.save_relay_cursor("team-1", 7, 101).unwrap();
            assert_eq!(store.relay_cursor("team-1").unwrap(), 12);
        }
        let reopened = TeamStore::open(&path).unwrap();
        assert_eq!(reopened.relay_cursor("team-1").unwrap(), 12);
        let _ = fs::remove_file(path);
    }

    fn identity_path(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "team-assistant-membership-{label}-{}-{nanos}.json",
            std::process::id()
        ))
    }

    fn member(id: &str, name: &str) -> Member {
        Member {
            member_id: id.into(),
            display_name: name.into(),
            role: Role::Implementer,
            active: true,
        }
    }

    fn event(id: &str) -> Event {
        Event {
            event_id: id.into(),
            team_id: "team-1".into(),
            actor_member_id: "member-1".into(),
            device_id: "device-1".into(),
            event_type: EventType::TaskCreated,
            payload: json!({
                "task_id": id,
                "assigner_member_id": "member-1",
                "assignee_member_id": "member-1",
                "description": "inspect repository",
                "created_at": 1
            }),
            schema_version: 1,
            logical_time: 1,
            signature: Some("test-signature".into()),
        }
    }

    fn authorize(store: &TeamStore) {
        store.save_team(&Team { team_id: "team-1".into(), name: "Test".into(), authority_member_id: "member-1".into(), created_at: 1 }).unwrap();
        store
            .save_member("team-1", &member("member-1", "Ravi"), 1)
            .unwrap();
        store
            .save_device(
                &Device {
                    device_id: "device-1".into(),
                    member_id: "member-1".into(),
                    public_key: "test-key".into(),
                    active: true,
                },
                1,
            )
            .unwrap();
    }

    fn seed_task_profile(
        path: &Path,
        team: &Team,
        profile: &Member,
        manager: &Member,
        manager_device: &Device,
        implementer: &Member,
        implementer_device: &Device,
    ) -> TeamStore {
        let store = TeamStore::open(path).unwrap();
        store.save_profile(profile, 1, 1).unwrap();
        store.save_team(team).unwrap();
        for member in [manager, implementer] {
            store.save_member(&team.team_id, member, 1).unwrap();
        }
        for device in [manager_device, implementer_device] {
            store.save_device(device, 1).unwrap();
        }
        store
    }

    fn signed_event(
        identity: &DeviceIdentity,
        team_id: &str,
        actor_member_id: &str,
        event_type: EventType,
        payload: serde_json::Value,
        logical_time: u64,
    ) -> Event {
        let mut event = Event::create(
            team_id,
            actor_member_id,
            identity.device_id(),
            event_type,
            payload,
            logical_time,
            "pending-signature",
        )
        .unwrap();
        event.signature = Some(identity.sign(&event.signing_bytes()));
        event
    }

    fn permission_team(store: &TeamStore) -> Team {
        let team = Team::create("Permission team", "manager-1", 10).unwrap();
        store.save_team(&team).unwrap();
        for (member_id, name) in [
            ("manager-1", "Manager"),
            ("lead-1", "Lead"),
            ("implementer-1", "Implementer"),
        ] {
            store
                .save_member(&team.team_id, &member(member_id, name), 10)
                .unwrap();
        }
        team
    }

    fn repository_read_grant(
        grant_id: &str,
        subject_member_id: &str,
        grantor_member_id: &str,
        delegable: bool,
        expires_at: Option<u64>,
    ) -> PermissionGrant {
        PermissionGrant {
            grant_id: grant_id.into(),
            subject_member_id: subject_member_id.into(),
            resource: PermissionResource::Repository("repo-1".into()),
            action: PermissionAction::ReadRepository,
            grantor_member_id: grantor_member_id.into(),
            delegable,
            expires_at,
            revoked: false,
        }
    }

    #[test]
    fn fresh_database_applies_schema_once() {
        let path = db_path("fresh");
        let store = TeamStore::open(&path).expect("open fresh database");
        assert_eq!(store.schema_version().unwrap(), CURRENT_SCHEMA_VERSION);
        assert_eq!(store.event_count().unwrap(), 0);
        drop(store);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn reopening_preserves_profile_and_event() {
        let path = db_path("reopen");
        {
            let store = TeamStore::open(&path).unwrap();
            store
                .save_profile(&member("member-1", "Ravi"), 10, 11)
                .unwrap();
            authorize(&store);
            let mut store = store;
            store.insert_event(&event("event-1"), 12).unwrap();
        }
        {
            let mut store = TeamStore::open(&path).unwrap();
            let profile = store.load_profile().unwrap().unwrap();
            assert_eq!(profile.member.member_id, "member-1");
            assert_eq!(profile.member.display_name, "Ravi");
            assert_eq!(profile.created_at, 10);
            assert_eq!(store.event_count().unwrap(), 1);
            assert!(!store.insert_event(&event("event-1"), 13).unwrap());
            assert_eq!(store.event_count().unwrap(), 1);
        }
        let _ = fs::remove_file(path);
    }

    #[test]
    fn independent_profile_databases_do_not_share_records() {
        let first_path = db_path("first");
        let second_path = db_path("second");
        let first = TeamStore::open(&first_path).unwrap();
        let second = TeamStore::open(&second_path).unwrap();
        first.save_profile(&member("member-a", "A"), 1, 1).unwrap();
        second.save_profile(&member("member-b", "B"), 2, 2).unwrap();
        assert_eq!(
            first.load_profile().unwrap().unwrap().member.member_id,
            "member-a"
        );
        assert_eq!(
            second.load_profile().unwrap().unwrap().member.member_id,
            "member-b"
        );
        drop((first, second));
        let _ = fs::remove_file(first_path);
        let _ = fs::remove_file(second_path);
    }

    #[test]
    fn repository_approval_persists_metadata_only() {
        let path = db_path("repository");
        let store = TeamStore::open(&path).unwrap();
        let approval = RepositoryApproval {
            repository_id: "repo-1".into(),
            owner_member_id: "member-1".into(),
            local_path: r"C:\work\repo".into(),
            approved_at: 20,
        };
        store.save_repository_approval(&approval).unwrap();
        assert_eq!(store.repository_approval("repo-1").unwrap(), Some(approval));
        let stored_columns: i64 = store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM repository_approvals WHERE local_path = ?1",
                params![r"C:\work\repo"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored_columns, 1);
        drop(store);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn event_projection_is_atomic_and_idempotent() {
        let path = db_path("projection");
        let store = TeamStore::open(&path).unwrap();
        authorize(&store);
        let mut store = store;
        assert!(store.insert_event(&event("event-1"), 12).unwrap());
        assert_eq!(store.event_count().unwrap(), 1);
        assert_eq!(store.task_count().unwrap(), 1);
        assert!(!store.insert_event(&event("event-1"), 13).unwrap());
        assert_eq!(store.event_count().unwrap(), 1);
        assert_eq!(store.task_count().unwrap(), 1);

        let mut broadcast = event("broadcast-event-1");
        broadcast.event_type = EventType::BroadcastCreated;
        broadcast.payload = json!({
            "message_id": "message-1",
            "sender_member_id": "member-1",
            "scope": "team",
            "message": "status update",
            "created_at": 12
        });
        assert!(store.insert_event(&broadcast, 12).unwrap());
        assert!(!store.insert_event(&broadcast, 13).unwrap());
        assert_eq!(store.broadcast_count().unwrap(), 1);
        drop(store);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn broadcast_projection_persists_team_and_group_scopes() {
        let path = db_path("broadcast-projection");
        let store = TeamStore::open(&path).unwrap();
        authorize(&store);
        let mut store = store;

        let mut team_broadcast = event("broadcast-team");
        team_broadcast.event_type = EventType::BroadcastCreated;
        team_broadcast.payload = json!({
            "message_id": "message-team",
            "sender_member_id": "member-1",
            "scope": "team",
            "message": "The release is ready for review.",
            "created_at": 12
        });
        assert!(store.insert_event(&team_broadcast, 12).unwrap());

        let mut group_broadcast = event("broadcast-group");
        group_broadcast.event_type = EventType::BroadcastCreated;
        group_broadcast.payload = json!({
            "message_id": "message-backend",
            "sender_member_id": "member-1",
            "scope": "group:backend",
            "message": "Backend deploy starts at 18:00.",
            "created_at": 13
        });
        assert!(store.insert_event(&group_broadcast, 13).unwrap());
        assert!(!store.insert_event(&group_broadcast, 14).unwrap());

        assert_eq!(store.broadcast_count().unwrap(), 2);
        assert_eq!(
            store.broadcast("message-team").unwrap().unwrap().scope,
            "team"
        );
        let group_messages = store.broadcasts(Some("group:backend")).unwrap();
        assert_eq!(group_messages.len(), 1);
        assert_eq!(group_messages[0].message_id, "message-backend");
        assert_eq!(store.broadcasts(None).unwrap().len(), 2);

        let mut invalid_scope = event("broadcast-invalid-scope");
        invalid_scope.event_type = EventType::BroadcastCreated;
        invalid_scope.payload = json!({
            "message_id": "message-invalid-scope",
            "sender_member_id": "member-1",
            "scope": "department:backend",
            "message": "should be rejected",
            "created_at": 15
        });
        assert!(store.insert_event(&invalid_scope, 15).is_err());

        let mut spoofed_sender = event("broadcast-spoofed-sender");
        spoofed_sender.event_type = EventType::BroadcastCreated;
        spoofed_sender.payload = json!({
            "message_id": "message-spoofed-sender",
            "sender_member_id": "member-2",
            "scope": "team",
            "message": "should be rejected",
            "created_at": 16
        });
        assert!(store.insert_event(&spoofed_sender, 16).is_err());
        assert_eq!(store.broadcast_count().unwrap(), 2);

        drop(store);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn task_lifecycle_projection_records_status_and_follow_up_activity() {
        let path = db_path("task-lifecycle");
        let store = TeamStore::open(&path).unwrap();
        authorize(&store);
        let mut store = store;

        let mut created = event("task-created");
        created.payload = json!({
            "task_id": "task-1",
            "assigner_member_id": "member-1",
            "assignee_member_id": "member-1",
            "description": "review the local changes",
            "created_at": 10
        });
        assert!(store.insert_event(&created, 10).unwrap());

        let mut delivered = event("task-delivered");
        delivered.event_type = EventType::TaskStatusChanged;
        delivered.payload = json!({
            "task_id": "task-1",
            "status": "Delivered"
        });
        assert!(store.insert_event(&delivered, 11).unwrap());

        let mut follow_up = event("task-follow-up");
        follow_up.event_type = EventType::TaskFollowUpAdded;
        follow_up.payload = json!({
            "task_id": "task-1",
            "message": "Please include the failing test output.",
            "created_at": 12
        });
        assert!(store.insert_event(&follow_up, 12).unwrap());

        let task = store.task("task-1").unwrap().unwrap();
        assert_eq!(task.status, TaskStatus::Delivered);
        assert_eq!(task.description, "review the local changes");
        let activity = store.task_activity("task-1").unwrap();
        assert_eq!(activity.len(), 3);
        assert_eq!(activity[0].activity_type, "created");
        assert_eq!(activity[1].status, Some(TaskStatus::Delivered));
        assert_eq!(activity[2].activity_type, "follow_up");
        assert_eq!(
            activity[2].message,
            "Please include the failing test output."
        );

        let mut invalid = event("task-invalid-transition");
        invalid.event_type = EventType::TaskStatusChanged;
        invalid.payload = json!({
            "task_id": "task-1",
            "status": "Queued"
        });
        assert!(store.insert_event(&invalid, 13).is_err());
        assert_eq!(store.task_activity("task-1").unwrap().len(), 3);

        drop(store);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn two_local_profiles_replay_full_signed_task_lifecycle_and_converge() {
        let manager_db = db_path("task-profile-manager");
        let implementer_db = db_path("task-profile-implementer");
        let manager_key = identity_path("task-profile-manager");
        let implementer_key = identity_path("task-profile-implementer");

        let manager_identity = DeviceIdentity::load_or_create(&manager_key).unwrap();
        let implementer_identity = DeviceIdentity::load_or_create(&implementer_key).unwrap();
        let manager = Member {
            member_id: "manager-1".into(),
            display_name: "Manager".into(),
            role: Role::Manager,
            active: true,
        };
        let implementer = member("implementer-1", "Implementer");
        let manager_device = Device {
            device_id: manager_identity.device_id().into(),
            member_id: manager.member_id.clone(),
            public_key: manager_identity.public_key(),
            active: true,
        };
        let implementer_device = Device {
            device_id: implementer_identity.device_id().into(),
            member_id: implementer.member_id.clone(),
            public_key: implementer_identity.public_key(),
            active: true,
        };
        let team = Team::create("Task team", manager.member_id.clone(), 1).unwrap();
        let mut manager_store = seed_task_profile(
            &manager_db,
            &team,
            &manager,
            &manager,
            &manager_device,
            &implementer,
            &implementer_device,
        );
        let mut implementer_store = seed_task_profile(
            &implementer_db,
            &team,
            &implementer,
            &manager,
            &manager_device,
            &implementer,
            &implementer_device,
        );

        let shared_task_events = [
            signed_event(
                &manager_identity,
                &team.team_id,
                &manager.member_id,
                EventType::TaskCreated,
                json!({
                    "task_id": "task-shared",
                    "assigner_member_id": "manager-1",
                    "assignee_member_id": "implementer-1",
                    "description": "review the release candidate",
                    "created_at": 10
                }),
                10,
            ),
            signed_event(
                &implementer_identity,
                &team.team_id,
                &implementer.member_id,
                EventType::TaskStatusChanged,
                json!({ "task_id": "task-shared", "status": "Delivered" }),
                11,
            ),
            signed_event(
                &implementer_identity,
                &team.team_id,
                &implementer.member_id,
                EventType::TaskStatusChanged,
                json!({ "task_id": "task-shared", "status": "InProgress" }),
                12,
            ),
            signed_event(
                &manager_identity,
                &team.team_id,
                &manager.member_id,
                EventType::TaskFollowUpAdded,
                json!({
                    "task_id": "task-shared",
                    "message": "Include the failing test output.",
                    "created_at": 13
                }),
                13,
            ),
            signed_event(
                &implementer_identity,
                &team.team_id,
                &implementer.member_id,
                EventType::TaskStatusChanged,
                json!({ "task_id": "task-shared", "status": "NeedsAttention" }),
                14,
            ),
            signed_event(
                &implementer_identity,
                &team.team_id,
                &implementer.member_id,
                EventType::TaskStatusChanged,
                json!({ "task_id": "task-shared", "status": "Queued" }),
                15,
            ),
            signed_event(
                &implementer_identity,
                &team.team_id,
                &implementer.member_id,
                EventType::TaskStatusChanged,
                json!({ "task_id": "task-shared", "status": "Delivered" }),
                16,
            ),
            signed_event(
                &implementer_identity,
                &team.team_id,
                &implementer.member_id,
                EventType::TaskStatusChanged,
                json!({ "task_id": "task-shared", "status": "InProgress" }),
                17,
            ),
            signed_event(
                &implementer_identity,
                &team.team_id,
                &implementer.member_id,
                EventType::TaskStatusChanged,
                json!({ "task_id": "task-shared", "status": "Done" }),
                18,
            ),
            signed_event(
                &manager_identity,
                &team.team_id,
                &manager.member_id,
                EventType::TaskCreated,
                json!({
                    "task_id": "task-cancelled",
                    "assigner_member_id": "manager-1",
                    "assignee_member_id": "implementer-1",
                    "description": "cancel this obsolete task",
                    "created_at": 19
                }),
                19,
            ),
            signed_event(
                &implementer_identity,
                &team.team_id,
                &implementer.member_id,
                EventType::TaskStatusChanged,
                json!({ "task_id": "task-cancelled", "status": "Cancelled" }),
                20,
            ),
            signed_event(
                &manager_identity,
                &team.team_id,
                &manager.member_id,
                EventType::TaskCreated,
                json!({
                    "task_id": "task-failed",
                    "assigner_member_id": "manager-1",
                    "assignee_member_id": "implementer-1",
                    "description": "exercise failure handling",
                    "created_at": 21
                }),
                21,
            ),
            signed_event(
                &implementer_identity,
                &team.team_id,
                &implementer.member_id,
                EventType::TaskStatusChanged,
                json!({ "task_id": "task-failed", "status": "Failed" }),
                22,
            ),
        ];

        for event in &shared_task_events {
            assert!(manager_store
                .insert_event(event, event.logical_time)
                .unwrap());
            assert!(implementer_store
                .insert_event(event, event.logical_time)
                .unwrap());
        }

        assert_eq!(manager_store.task_count().unwrap(), 3);
        assert_eq!(implementer_store.task_count().unwrap(), 3);
        assert_eq!(
            manager_store.task("task-shared").unwrap(),
            implementer_store.task("task-shared").unwrap()
        );
        assert_eq!(
            manager_store.task("task-shared").unwrap().unwrap().status,
            TaskStatus::Done
        );
        assert_eq!(
            manager_store
                .task("task-cancelled")
                .unwrap()
                .unwrap()
                .status,
            TaskStatus::Cancelled
        );
        assert_eq!(
            manager_store.task("task-failed").unwrap().unwrap().status,
            TaskStatus::Failed
        );
        assert_eq!(
            manager_store.task_activity("task-shared").unwrap(),
            implementer_store.task_activity("task-shared").unwrap()
        );
        assert_eq!(manager_store.task_activity("task-shared").unwrap().len(), 9);

        let invalid = signed_event(
            &implementer_identity,
            &team.team_id,
            &implementer.member_id,
            EventType::TaskStatusChanged,
            json!({ "task_id": "task-shared", "status": "Cancelled" }),
            23,
        );
        assert!(manager_store.insert_event(&invalid, 23).is_err());
        assert!(implementer_store.insert_event(&invalid, 23).is_err());
        assert_eq!(manager_store.event_count().unwrap(), 13);
        assert_eq!(implementer_store.event_count().unwrap(), 13);

        drop((
            manager_store,
            implementer_store,
            manager_identity,
            implementer_identity,
        ));
        for path in [manager_db, implementer_db, manager_key, implementer_key] {
            let _ = fs::remove_file(path);
        }
    }

    #[test]
    fn conflicting_event_id_is_rejected_without_changing_projection() {
        let path = db_path("event-conflict");
        let store = TeamStore::open(&path).unwrap();
        authorize(&store);
        let mut store = store;
        assert!(store.insert_event(&event("event-1"), 12).unwrap());
        let mut conflicting = event("event-1");
        conflicting.payload["description"] = json!("changed payload");
        assert!(matches!(
            store.insert_event(&conflicting, 13),
            Err(StorageError::ConflictingEvent { .. })
        ));
        assert_eq!(store.event_count().unwrap(), 1);
        assert_eq!(store.task_count().unwrap(), 1);
        drop(store);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn malformed_payload_does_not_update_projection() {
        let path = db_path("malformed-payload");
        let store = TeamStore::open(&path).unwrap();
        authorize(&store);
        let mut malformed = event("malformed");
        malformed.payload = json!({ "task_id": "task-1" });
        let mut store = store;
        assert!(store.insert_event(&malformed, 12).is_err());
        assert_eq!(store.event_count().unwrap(), 0);
        assert_eq!(store.task_count().unwrap(), 0);
        drop(store);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn unsigned_and_unauthorized_events_are_rejected() {
        let path = db_path("event-policy");
        let store = TeamStore::open(&path).unwrap();
        authorize(&store);
        let mut unsigned = event("unsigned");
        unsigned.signature = None;
        let mut store = store;
        assert!(matches!(
            store.insert_event(&unsigned, 1),
            Err(StorageError::UnsignedEvent)
        ));
        let mut unauthorized = event("unauthorized");
        unauthorized.actor_member_id = "unknown-member".into();
        assert!(matches!(
            store.insert_event(&unauthorized, 1),
            Err(StorageError::UnauthorizedEvent { .. })
        ));
        assert_eq!(store.event_count().unwrap(), 0);
        drop(store);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn signed_invitation_joins_profile_and_rejects_wrong_pairing_code() {
        let owner_db = db_path("owner");
        let joiner_db = db_path("joiner");
        let outsider_db = db_path("outsider");
        let owner_key_path = identity_path("owner");
        let joiner_key_path = identity_path("joiner");
        let outsider_key_path = identity_path("outsider");

        let owner_identity = DeviceIdentity::load_or_create(&owner_key_path).unwrap();
        let joiner_identity = DeviceIdentity::load_or_create(&joiner_key_path).unwrap();
        let outsider_identity = DeviceIdentity::load_or_create(&outsider_key_path).unwrap();
        let owner_member = member("manager-1", "Manager");
        let owner_device = Device {
            device_id: owner_identity.device_id().into(),
            member_id: owner_member.member_id.clone(),
            public_key: owner_identity.public_key(),
            active: true,
        };
        let team = Team::create("Product team", owner_member.member_id.clone(), 10).unwrap();

        let owner_store = TeamStore::open(&owner_db).unwrap();
        owner_store.save_team(&team).unwrap();
        owner_store
            .save_member(&team.team_id, &owner_member, 10)
            .unwrap();
        owner_store.save_device(&owner_device, 10).unwrap();

        let unsigned =
            TeamInvitation::new_unsigned(&team, &owner_device, "pairing-code-1234", 20, 100)
                .unwrap();
        let invitation_signature = owner_identity.sign(&unsigned.signing_bytes());
        let invitation = unsigned.with_signature(invitation_signature).unwrap();
        owner_store.save_invitation(&invitation, 21).unwrap();

        let joiner_member = member("implementer-1", "Implementer");
        let joiner_device = Device {
            device_id: joiner_identity.device_id().into(),
            member_id: joiner_member.member_id.clone(),
            public_key: joiner_identity.public_key(),
            active: true,
        };
        let mut joiner_store = TeamStore::open(&joiner_db).unwrap();
        joiner_store
            .accept_invitation(
                &invitation,
                "pairing-code-1234",
                &owner_member,
                &owner_device,
                &joiner_member,
                &joiner_device,
                &joiner_identity,
                30,
            )
            .unwrap();
        let joined_count: i64 = joiner_store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM team_members WHERE team_id = ?1 AND member_id = ?2",
                params![team.team_id, joiner_member.member_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(joined_count, 1);
        assert_eq!(
            joiner_store.schema_version().unwrap(),
            CURRENT_SCHEMA_VERSION
        );
        assert!(matches!(
            joiner_store.accept_invitation(
                &invitation,
                "pairing-code-1234",
                &owner_member,
                &owner_device,
                &joiner_member,
                &joiner_device,
                &joiner_identity,
                31,
            ),
            Err(StorageError::InvitationAlreadyAccepted)
        ));

        let outsider_member = member("outsider-1", "Outsider");
        let outsider_device = Device {
            device_id: outsider_identity.device_id().into(),
            member_id: outsider_member.member_id.clone(),
            public_key: outsider_identity.public_key(),
            active: true,
        };
        let mut outsider_store = TeamStore::open(&outsider_db).unwrap();
        assert!(matches!(
            outsider_store.accept_invitation(
                &invitation,
                "wrong-pairing-code",
                &owner_member,
                &owner_device,
                &outsider_member,
                &outsider_device,
                &outsider_identity,
                30,
            ),
            Err(StorageError::PairingCodeMismatch)
        ));
        let outsider_count: i64 = outsider_store
            .connection
            .query_row("SELECT COUNT(*) FROM team_members", [], |row| row.get(0))
            .unwrap();
        assert_eq!(outsider_count, 0);

        drop((owner_store, joiner_store, outsider_store));
        let _ = fs::remove_file(owner_db);
        let _ = fs::remove_file(joiner_db);
        let _ = fs::remove_file(outsider_db);
        let _ = fs::remove_file(owner_key_path);
        let _ = fs::remove_file(joiner_key_path);
        let _ = fs::remove_file(outsider_key_path);
    }

    #[test]
    fn permission_storage_supports_delegation_and_revocation_cascade() {
        let path = db_path("permissions-delegation");
        let store = TeamStore::open(&path).unwrap();
        let team = permission_team(&store);
        let repository = PermissionResource::Repository("repo-1".into());
        let action = PermissionAction::ReadRepository;

        store
            .grant_permission(
                &team.team_id,
                &repository_read_grant("grant-manager-lead", "lead-1", "manager-1", true, None),
                20,
            )
            .unwrap();
        store
            .grant_permission(
                &team.team_id,
                &repository_read_grant(
                    "grant-lead-implementer",
                    "implementer-1",
                    "lead-1",
                    false,
                    None,
                ),
                21,
            )
            .unwrap();

        let decision = store
            .permission_decision(&team.team_id, "implementer-1", &repository, &action, 22)
            .unwrap();
        assert!(decision.allowed);
        assert_eq!(
            decision.chain,
            vec!["grant-manager-lead", "grant-lead-implementer"]
        );
        assert_eq!(decision.reason, PermissionDecisionReason::Allowed);

        let task = PermissionResource::Task("task-1".into());
        store
            .grant_permission(
                &team.team_id,
                &PermissionGrant {
                    grant_id: "grant-manager-task-update".into(),
                    subject_member_id: "implementer-1".into(),
                    resource: task.clone(),
                    action: PermissionAction::UpdateTask,
                    grantor_member_id: "manager-1".into(),
                    delegable: false,
                    expires_at: None,
                    revoked: false,
                },
                22,
            )
            .unwrap();
        let task_decision = store
            .permission_decision(
                &team.team_id,
                "implementer-1",
                &task,
                &PermissionAction::UpdateTask,
                22,
            )
            .unwrap();
        assert!(task_decision.allowed);
        assert_eq!(task_decision.chain, vec!["grant-manager-task-update"]);
        assert_eq!(task_decision.reason, PermissionDecisionReason::Allowed);

        store
            .revoke_permission(&team.team_id, "grant-manager-lead", "manager-1", 23)
            .unwrap();
        let stale_request_decision = store
            .permission_decision(&team.team_id, "implementer-1", &repository, &action, 24)
            .unwrap();
        assert!(!stale_request_decision.allowed);
        assert_eq!(
            stale_request_decision.chain,
            vec!["grant-manager-lead", "grant-lead-implementer"]
        );
        assert_eq!(
            stale_request_decision.reason,
            PermissionDecisionReason::GrantRevoked("grant-manager-lead".into())
        );

        drop(store);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn permission_storage_rejects_nondelegable_grants_and_cycles() {
        let path = db_path("permissions-negative");
        let store = TeamStore::open(&path).unwrap();
        let team = permission_team(&store);

        store
            .grant_permission(
                &team.team_id,
                &repository_read_grant(
                    "grant-manager-lead-nondelegable",
                    "lead-1",
                    "manager-1",
                    false,
                    None,
                ),
                20,
            )
            .unwrap();
        assert!(matches!(
            store.grant_permission(
                &team.team_id,
                &repository_read_grant(
                    "grant-lead-implementer-denied",
                    "implementer-1",
                    "lead-1",
                    false,
                    None,
                ),
                21,
            ),
            Err(StorageError::Permission(
                PermissionError::GrantorUnauthorized(_)
            ))
        ));

        store
            .grant_permission(
                &team.team_id,
                &repository_read_grant(
                    "grant-manager-lead-delegable",
                    "lead-1",
                    "manager-1",
                    true,
                    None,
                ),
                22,
            )
            .unwrap();
        assert!(matches!(
            store.grant_permission(
                &team.team_id,
                &repository_read_grant(
                    "grant-lead-manager-cycle",
                    "manager-1",
                    "lead-1",
                    false,
                    None,
                ),
                23,
            ),
            Err(StorageError::Permission(PermissionError::CycleDetected(_)))
        ));

        store
            .grant_permission(
                &team.team_id,
                &repository_read_grant(
                    "grant-expired",
                    "implementer-1",
                    "manager-1",
                    false,
                    Some(30),
                ),
                24,
            )
            .unwrap();
        let expired = store
            .permission_decision(
                &team.team_id,
                "implementer-1",
                &PermissionResource::Repository("repo-1".into()),
                &PermissionAction::ReadRepository,
                30,
            )
            .unwrap();
        assert!(!expired.allowed);
        assert_eq!(
            expired.reason,
            PermissionDecisionReason::GrantExpired("grant-expired".into())
        );

        drop(store);
        let _ = fs::remove_file(path);
    }
}
