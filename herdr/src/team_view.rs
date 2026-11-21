//! Read-only view data for the local team dashboard.
//!
//! This module deliberately sits between SQLite storage and the terminal UI.
//! It never creates or migrates a database, never loads device private keys,
//! and caps every collection before it is handed to a renderer.

use crate::team::{
    BroadcastRecord, Member, OutboxRecord, PermissionAction, PermissionResource, RequestRecord,
    TaskActivityRecord, TaskRecord, TaskStatus, Team,
};
use crate::team_storage::{StorageError, TeamStore};
use std::env;
use std::path::{Path, PathBuf};

pub(crate) const TEAM_DB_ENV: &str = "HERDR_TEAM_DB";
pub(crate) const MAX_DASHBOARD_ROWS: usize = 100;

pub(crate) fn default_database_path() -> PathBuf {
    if let Some(path) = env::var_os(TEAM_DB_ENV).filter(|value| !value.is_empty()) {
        return PathBuf::from(path);
    }
    database_path_from(
        env::var_os("LOCALAPPDATA"),
        env::var_os("XDG_DATA_HOME"),
        env::var_os("HOME"),
        env::temp_dir(),
    )
}

fn database_path_from(
    local_app_data: Option<std::ffi::OsString>,
    xdg_data_home: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
    temporary_directory: PathBuf,
) -> PathBuf {
    if let Some(path) = local_app_data.filter(|value| !value.is_empty()) {
        return PathBuf::from(path).join("Herdr").join("team.sqlite");
    }
    if let Some(path) = xdg_data_home.filter(|value| !value.is_empty()) {
        return PathBuf::from(path).join("herdr").join("team.sqlite");
    }
    if let Some(home) = home.filter(|value| !value.is_empty()) {
        return PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("herdr")
            .join("team.sqlite");
    }
    temporary_directory.join("Herdr").join("team.sqlite")
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TeamDashboardSnapshot {
    pub profile: Option<Member>,
    pub teams: Vec<Team>,
    pub repositories: Vec<String>,
    pub teammates: Vec<Member>,
    pub tasks: Vec<TaskRecord>,
    pub broadcasts: Vec<BroadcastRecord>,
    pub completion_announcements: Vec<TaskCompletionAnnouncement>,
    pub delivery: DeliverySummary,
    pub requests: Vec<RequestRecord>,
    pub permission_graph: Vec<PermissionGraphRow>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct DeliverySummary {
    pub queued: usize,
    pub retryable: usize,
    pub syncing: usize,
    pub delivered: usize,
    pub failed: usize,
}

impl DeliverySummary {
    fn from_outbox(records: &[OutboxRecord]) -> Self {
        let mut summary = Self::default();
        for record in records {
            match record.state {
                crate::team::OutboxState::Queued => {
                    summary.queued += 1;
                    summary.retryable += usize::from(record.attempts > 0);
                }
                crate::team::OutboxState::InFlight => summary.syncing += 1,
                crate::team::OutboxState::Delivered => summary.delivered += 1,
                crate::team::OutboxState::Failed => summary.failed += 1,
                crate::team::OutboxState::Cancelled => {}
            }
        }
        summary
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PermissionGraphRow {
    pub member_id: String,
    pub display_name: String,
    pub role: crate::team::Role,
    pub resource: String,
    pub action: String,
    pub active: bool,
    pub allowed: bool,
    pub reason: String,
    pub grant_chain: Vec<String>,
}

/// A shared activity item derived from an accepted, signed task status event.
/// It is projected from task history instead of creating a second broadcast
/// event, so relay replays cannot duplicate the announcement.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TaskCompletionAnnouncement {
    pub task_id: String,
    pub description: String,
    pub completed_by_member_id: String,
    pub completed_at: u64,
}

impl TeamDashboardSnapshot {
    pub(crate) fn empty() -> Self {
        Self {
            profile: None,
            teams: Vec::new(),
            repositories: Vec::new(),
            teammates: Vec::new(),
            tasks: Vec::new(),
            broadcasts: Vec::new(),
            completion_announcements: Vec::new(),
            delivery: DeliverySummary::default(),
            requests: Vec::new(),
            permission_graph: Vec::new(),
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.profile.is_none()
            && self.teammates.is_empty()
            && self.tasks.is_empty()
            && self.broadcasts.is_empty()
            && self.completion_announcements.is_empty()
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
                write!(
                    formatter,
                    "team database does not exist: {}",
                    path.display()
                )
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

pub(crate) fn load_default() -> Result<TeamDashboardSnapshot, TeamDashboardError> {
    load_from_path(default_database_path())
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
    let tasks = bounded_latest(store.tasks()?);
    let broadcasts = bounded_latest(store.broadcasts(None)?);
    let completion_announcements = load_completion_announcements(&store, &tasks)?;
    let requests = bounded(store.requests()?);
    let delivery = DeliverySummary::from_outbox(&store.outbox()?);
    let teams = store.teams()?;
    let repositories = store
        .repository_approvals()?
        .into_iter()
        .map(|approval| approval.repository_id)
        .collect::<Vec<_>>();
    let permission_graph = load_permission_graph(&store, &teams, &teammates, &repositories)?;
    Ok(TeamDashboardSnapshot {
        profile,
        teams,
        repositories,
        teammates,
        tasks,
        broadcasts,
        completion_announcements,
        delivery,
        requests,
        permission_graph,
    })
}

fn load_permission_graph(
    store: &TeamStore,
    teams: &[Team],
    teammates: &[Member],
    repositories: &[String],
) -> Result<Vec<PermissionGraphRow>, StorageError> {
    let Some(team) = teams.first() else {
        return Ok(Vec::new());
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let mut rows = Vec::new();
    for member in teammates {
        if rows.len() >= MAX_DASHBOARD_ROWS {
            break;
        }
        let task_decision = store.permission_decision(
            &team.team_id,
            &member.member_id,
            &PermissionResource::Team(team.team_id.clone()),
            &PermissionAction::CreateTask,
            now,
        )?;
        rows.push(PermissionGraphRow {
            member_id: member.member_id.clone(),
            display_name: member.display_name.clone(),
            role: member.role.clone(),
            resource: format!("team {}", team.name),
            action: "assign tasks".into(),
            active: member.active,
            allowed: member.active && task_decision.allowed,
            reason: if member.active {
                format!("{:?}", task_decision.reason)
            } else {
                "InactiveMember".into()
            },
            grant_chain: task_decision.chain,
        });
        for repository in repositories {
            if rows.len() >= MAX_DASHBOARD_ROWS {
                break;
            }
            let decision = store.permission_decision(
                &team.team_id,
                &member.member_id,
                &PermissionResource::Repository(repository.clone()),
                &PermissionAction::ReadRepository,
                now,
            )?;
            rows.push(PermissionGraphRow {
                member_id: member.member_id.clone(),
                display_name: member.display_name.clone(),
                role: member.role.clone(),
                resource: format!("repository {repository}"),
                action: "read repository".into(),
                active: member.active,
                allowed: member.active && decision.allowed,
                reason: if member.active {
                    format!("{:?}", decision.reason)
                } else {
                    "InactiveMember".into()
                },
                grant_chain: decision.chain,
            });
        }
    }
    Ok(rows)
}

fn load_completion_announcements(
    store: &TeamStore,
    tasks: &[TaskRecord],
) -> Result<Vec<TaskCompletionAnnouncement>, StorageError> {
    let mut announcements = Vec::new();
    for task in tasks.iter().filter(|task| task.status == TaskStatus::Done) {
        let activity = store.task_activity(&task.task_id)?;
        if let Some(announcement) = completion_announcement(task, &activity) {
            announcements.push(announcement);
        }
    }
    announcements.sort_by(|left, right| {
        left.completed_at
            .cmp(&right.completed_at)
            .then_with(|| left.task_id.cmp(&right.task_id))
    });
    Ok(bounded_latest(announcements))
}

fn completion_announcement(
    task: &TaskRecord,
    activity: &[TaskActivityRecord],
) -> Option<TaskCompletionAnnouncement> {
    if task.status != TaskStatus::Done {
        return None;
    }
    let completed = activity
        .iter()
        .filter(|item| item.status.as_ref() == Some(&TaskStatus::Done))
        .max_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.activity_id.cmp(&right.activity_id))
        })?;
    Some(TaskCompletionAnnouncement {
        task_id: task.task_id.clone(),
        description: task.description.clone(),
        completed_by_member_id: completed.actor_member_id.clone(),
        completed_at: completed.created_at,
    })
}

fn bounded<T>(mut values: Vec<T>) -> Vec<T> {
    values.truncate(MAX_DASHBOARD_ROWS);
    values
}

fn bounded_latest<T>(mut values: Vec<T>) -> Vec<T> {
    if values.len() > MAX_DASHBOARD_ROWS {
        values.drain(..values.len() - MAX_DASHBOARD_ROWS);
    }
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

    #[test]
    fn default_database_path_uses_persistent_linux_data_directories() {
        let xdg_path = database_path_from(
            None,
            Some("/home/ec2-user/.local/share".into()),
            Some("/home/ec2-user".into()),
            PathBuf::from("/tmp"),
        );
        assert_eq!(
            xdg_path,
            PathBuf::from("/home/ec2-user/.local/share/herdr/team.sqlite")
        );

        let home_path = database_path_from(
            None,
            None,
            Some("/home/ec2-user".into()),
            PathBuf::from("/tmp"),
        );
        assert_eq!(
            home_path,
            PathBuf::from("/home/ec2-user/.local/share/herdr/team.sqlite")
        );
    }

    #[test]
    fn delivery_summary_uses_persisted_outbox_states() {
        let outbox = vec![
            OutboxRecord {
                event_id: "queued".into(),
                attempts: 0,
                next_attempt_at: None,
                state: crate::team::OutboxState::Queued,
            },
            OutboxRecord {
                event_id: "retryable".into(),
                attempts: 2,
                next_attempt_at: Some(20),
                state: crate::team::OutboxState::Queued,
            },
            OutboxRecord {
                event_id: "syncing".into(),
                attempts: 1,
                next_attempt_at: None,
                state: crate::team::OutboxState::InFlight,
            },
            OutboxRecord {
                event_id: "delivered".into(),
                attempts: 1,
                next_attempt_at: None,
                state: crate::team::OutboxState::Delivered,
            },
            OutboxRecord {
                event_id: "failed".into(),
                attempts: 5,
                next_attempt_at: None,
                state: crate::team::OutboxState::Failed,
            },
        ];
        assert_eq!(
            DeliverySummary::from_outbox(&outbox),
            DeliverySummary {
                queued: 2,
                retryable: 1,
                syncing: 1,
                delivered: 1,
                failed: 1,
            }
        );
    }

    #[test]
    fn completion_announcement_is_derived_once_from_done_task_history() {
        let task = TaskRecord {
            task_id: "task-1".into(),
            assigner_member_id: "manager".into(),
            assignee_member_id: "implementer".into(),
            description: "Implement the page".into(),
            status: TaskStatus::Done,
            created_at: 10,
        };
        let activity = vec![
            TaskActivityRecord {
                activity_id: "created-event".into(),
                task_id: task.task_id.clone(),
                actor_member_id: "manager".into(),
                activity_type: "created".into(),
                message: task.description.clone(),
                status: Some(TaskStatus::Queued),
                created_at: 10,
            },
            TaskActivityRecord {
                activity_id: "done-event".into(),
                task_id: task.task_id.clone(),
                actor_member_id: "implementer".into(),
                activity_type: "status_changed".into(),
                message: "status changed to Done".into(),
                status: Some(TaskStatus::Done),
                created_at: 20,
            },
            // Duplicate activity input still produces one derived feed item.
            TaskActivityRecord {
                activity_id: "done-event".into(),
                task_id: task.task_id.clone(),
                actor_member_id: "implementer".into(),
                activity_type: "status_changed".into(),
                message: "status changed to Done".into(),
                status: Some(TaskStatus::Done),
                created_at: 20,
            },
        ];

        let announcements = vec![completion_announcement(&task, &activity).unwrap()];
        assert_eq!(announcements.len(), 1);
        assert_eq!(announcements[0].completed_by_member_id, "implementer");
        assert_eq!(announcements[0].description, "Implement the page");
        assert_eq!(announcements[0].completed_at, 20);
    }

    #[test]
    fn incomplete_task_never_creates_completion_announcement() {
        let task = TaskRecord {
            task_id: "task-1".into(),
            assigner_member_id: "manager".into(),
            assignee_member_id: "implementer".into(),
            description: "Implement the page".into(),
            status: TaskStatus::InProgress,
            created_at: 10,
        };
        let activity = vec![TaskActivityRecord {
            activity_id: "progress-event".into(),
            task_id: task.task_id.clone(),
            actor_member_id: "implementer".into(),
            activity_type: "status_changed".into(),
            message: "status changed to InProgress".into(),
            status: Some(TaskStatus::InProgress),
            created_at: 20,
        }];
        assert!(completion_announcement(&task, &activity).is_none());
    }

    #[test]
    fn bounded_latest_retains_the_newest_records_in_chronological_order() {
        assert_eq!(bounded_latest(vec![1, 2, 3, 4]), vec![1, 2, 3, 4]);
        assert_eq!(
            bounded_latest((0..105).collect::<Vec<_>>()),
            (5..105).collect::<Vec<_>>()
        );
    }
}
