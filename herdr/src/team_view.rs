//! Read-only view data for the local team dashboard.
//!
//! This module deliberately sits between SQLite storage and the terminal UI.
//! It never creates or migrates a database, never loads device private keys,
//! and caps every collection before it is handed to a renderer.

use crate::team::{BroadcastRecord, Member, RequestRecord, TaskRecord};
use crate::team_storage::{StorageError, TeamStore};
use std::env;
use std::path::{Path, PathBuf};

pub(crate) const TEAM_DB_ENV: &str = "HERDR_TEAM_DB";
pub(crate) const MAX_DASHBOARD_ROWS: usize = 100;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TeamDashboardSnapshot {
    pub profile: Option<Member>,
    pub teammates: Vec<Member>,
    pub tasks: Vec<TaskRecord>,
    pub broadcasts: Vec<BroadcastRecord>,
    pub requests: Vec<RequestRecord>,
}

impl TeamDashboardSnapshot {
    pub(crate) fn empty() -> Self {
        Self {
            profile: None,
            teammates: Vec::new(),
            tasks: Vec::new(),
            broadcasts: Vec::new(),
            requests: Vec::new(),
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.profile.is_none()
            && self.teammates.is_empty()
            && self.tasks.is_empty()
            && self.broadcasts.is_empty()
            && self.requests.is_empty()
    }
}

#[derive(Debug)]
pub(crate) enum TeamDashboardError {
    NotConfigured,
    MissingDatabase(PathBuf),
    Storage(StorageError),
}

impl std::fmt::Display for TeamDashboardError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotConfigured => write!(
                formatter,
                "{TEAM_DB_ENV} is not set; run `herdr onboard setup` and point it at the team database"
            ),
            Self::MissingDatabase(path) => {
                write!(formatter, "team database does not exist: {}", path.display())
            }
            Self::Storage(error) => write!(formatter, "team dashboard storage error: {error}"),
        }
    }
}

impl std::error::Error for TeamDashboardError {}

impl From<StorageError> for TeamDashboardError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

pub(crate) fn load_from_env() -> Result<TeamDashboardSnapshot, TeamDashboardError> {
    let path = env::var_os(TEAM_DB_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or(TeamDashboardError::NotConfigured)?;
    load_from_path(path)
}

pub(crate) fn load_from_path(
    path: impl AsRef<Path>,
) -> Result<TeamDashboardSnapshot, TeamDashboardError> {
    let path = path.as_ref();
    if !path.is_file() {
        return Err(TeamDashboardError::MissingDatabase(path.to_path_buf()));
    }
    let store = TeamStore::open_read_only(path)?;
    let profile = store.load_profile()?.map(|stored| stored.member);
    let mut teammates = store.members()?;
    if let Some(profile_member) = profile.as_ref() {
        if !teammates
            .iter()
            .any(|member| member.member_id == profile_member.member_id)
        {
            teammates.push(profile_member.clone());
            teammates.sort_by(|left, right| {
                left.display_name
                    .cmp(&right.display_name)
                    .then_with(|| left.member_id.cmp(&right.member_id))
            });
        }
    }
    let teammates = bounded(teammates);
    let tasks = bounded(store.tasks()?);
    let broadcasts = bounded(store.broadcasts(None)?);
    let requests = bounded(store.requests()?);
    Ok(TeamDashboardSnapshot {
        profile,
        teammates,
        tasks,
        broadcasts,
        requests,
    })
}

fn bounded<T>(mut values: Vec<T>) -> Vec<T> {
    values.truncate(MAX_DASHBOARD_ROWS);
    values
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_path(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "team-assistant-dashboard-{label}-{}-{nanos}.sqlite",
            std::process::id()
        ))
    }

    #[test]
    fn missing_database_is_reported_without_creating_it() {
        let path = test_path("missing");
        let result = load_from_path(&path);
        assert!(
            matches!(result, Err(TeamDashboardError::MissingDatabase(actual)) if actual == path)
        );
        assert!(!path.exists());
    }

    #[test]
    fn unset_environment_is_not_treated_as_an_empty_database() {
        let original = env::var_os(TEAM_DB_ENV);
        env::remove_var(TEAM_DB_ENV);
        let result = load_from_env();
        match original {
            Some(value) => env::set_var(TEAM_DB_ENV, value),
            None => env::remove_var(TEAM_DB_ENV),
        }
        assert!(matches!(result, Err(TeamDashboardError::NotConfigured)));
    }

    #[test]
    fn empty_snapshot_has_no_records_or_private_material() {
        let snapshot = TeamDashboardSnapshot::empty();
        assert!(snapshot.is_empty());
        assert_eq!(MAX_DASHBOARD_ROWS, 100);
    }
}
