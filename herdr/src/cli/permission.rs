use serde::Serialize;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::team::{
    PermissionAction, PermissionDecision, PermissionDecisionReason, PermissionResource,
};
use crate::team_storage::TeamStore;

const EXPLAIN_USAGE: &str = concat!(
    "usage: herdr permission explain <repository-question|task-update> ",
    "--db PATH --team TEAM_ID --subject MEMBER_ID ",
    "[--repository REPOSITORY_ID|--task TASK_ID] [--at UNIX_SECONDS] [--json]"
);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExplanationKind {
    RepositoryQuestion,
    TaskUpdate,
}

impl ExplanationKind {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "repository-question" | "repo-question" => Some(Self::RepositoryQuestion),
            "task-update" => Some(Self::TaskUpdate),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::RepositoryQuestion => "repository question",
            Self::TaskUpdate => "task update",
        }
    }

    fn action(self) -> PermissionAction {
        match self {
            Self::RepositoryQuestion => PermissionAction::ReadRepository,
            Self::TaskUpdate => PermissionAction::UpdateTask,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ExplainRequest {
    database: PathBuf,
    team_id: String,
    subject_member_id: String,
    kind: ExplanationKind,
    resource_id: String,
    at: u64,
    json: bool,
}

impl ExplainRequest {
    fn resource(&self) -> PermissionResource {
        match self.kind {
            ExplanationKind::RepositoryQuestion => {
                PermissionResource::Repository(self.resource_id.clone())
            }
            ExplanationKind::TaskUpdate => PermissionResource::Task(self.resource_id.clone()),
        }
    }
}

#[derive(Serialize)]
struct JsonExplanation<'a> {
    request: &'static str,
    team_id: &'a str,
    subject_member_id: &'a str,
    resource_type: &'static str,
    resource_id: &'a str,
    action: &'static str,
    allowed: bool,
    reason_code: &'static str,
    reason: String,
    state: &'static str,
    grant_chain: &'a [String],
    evaluated_at: u64,
}

pub(crate) fn run_permission_command(args: &[String]) -> std::io::Result<i32> {
    if args.is_empty()
        || matches!(
            args.first().map(String::as_str),
            Some("help" | "--help" | "-h")
        )
    {
        print_help();
        return Ok(0);
    }

    let request = match parse_explain_request(args) {
        Ok(request) => request,
        Err(message) => {
            eprintln!("permission: {message}");
            eprintln!("{EXPLAIN_USAGE}");
            return Ok(2);
        }
    };

    let store = TeamStore::open_read_only(&request.database).map_err(|error| {
        std::io::Error::other(format!(
            "could not open permission database read-only: {error}"
        ))
    })?;
    let decision = store
        .permission_decision(
            &request.team_id,
            &request.subject_member_id,
            &request.resource(),
            &request.kind.action(),
            request.at,
        )
        .map_err(|error| std::io::Error::other(format!("could not inspect permission: {error}")))?;

    if request.json {
        let output = json_explanation(&request, &decision);
        println!(
            "{}",
            serde_json::to_string_pretty(&output).map_err(std::io::Error::other)?
        );
    } else {
        print!("{}", human_explanation(&request, &decision));
    }

    Ok(i32::from(!decision.allowed))
}

fn parse_explain_request(args: &[String]) -> Result<ExplainRequest, String> {
    if args.first().map(String::as_str) != Some("explain") {
        return Err("expected `explain`".into());
    }
    let kind = args
        .get(1)
        .and_then(|value| ExplanationKind::parse(value))
        .ok_or_else(|| "expected repository-question or task-update".to_string())?;

    let mut database = None;
    let mut team_id = None;
    let mut subject_member_id = None;
    let mut resource_id = None;
    let mut at = None;
    let mut json = false;
    let mut index = 2;
    while index < args.len() {
        let flag = args[index].as_str();
        match flag {
            "--db" => database = Some(required_value(args, &mut index, flag)?),
            "--team" => team_id = Some(required_value(args, &mut index, flag)?),
            "--subject" => subject_member_id = Some(required_value(args, &mut index, flag)?),
            "--repository" if kind == ExplanationKind::RepositoryQuestion => {
                resource_id = Some(required_value(args, &mut index, flag)?);
            }
            "--task" if kind == ExplanationKind::TaskUpdate => {
                resource_id = Some(required_value(args, &mut index, flag)?);
            }
            "--at" => {
                let value = required_value(args, &mut index, flag)?;
                at = Some(
                    value
                        .parse::<u64>()
                        .map_err(|_| "--at must be an unsigned Unix timestamp".to_string())?,
                );
            }
            "--json" => {
                json = true;
                index += 1;
            }
            "help" | "--help" | "-h" => return Err(EXPLAIN_USAGE.into()),
            _ => return Err(format!("unknown or invalid option: {flag}")),
        }
    }

    let database = database.ok_or_else(|| "missing --db".to_string())?;
    let team_id = team_id.ok_or_else(|| "missing --team".to_string())?;
    let subject_member_id = subject_member_id.ok_or_else(|| "missing --subject".to_string())?;
    let resource_id = resource_id.ok_or_else(|| {
        format!(
            "missing {}",
            match kind {
                ExplanationKind::RepositoryQuestion => "--repository",
                ExplanationKind::TaskUpdate => "--task",
            }
        )
    })?;

    if database.is_empty()
        || team_id.trim().is_empty()
        || subject_member_id.trim().is_empty()
        || resource_id.trim().is_empty()
    {
        return Err("database, team, subject, and resource values must not be empty".into());
    }

    Ok(ExplainRequest {
        database: PathBuf::from(database),
        team_id,
        subject_member_id,
        kind,
        resource_id,
        at: at.unwrap_or_else(current_unix_seconds),
        json,
    })
}

fn required_value(args: &[String], index: &mut usize, flag: &str) -> Result<String, String> {
    let value = args
        .get(*index + 1)
        .ok_or_else(|| format!("{flag} requires a value"))?;
    if value.starts_with('-') {
        return Err(format!("{flag} requires a value"));
    }
    *index += 2;
    Ok(value.clone())
}

fn current_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn print_help() {
    eprintln!("herdr permission commands:");
    eprintln!("  {EXPLAIN_USAGE}");
    eprintln!("  repository-question reads the ReadRepository decision.");
    eprintln!("  task-update reads the UpdateTask decision.");
    eprintln!("  The SQLite database is opened read-only and is never migrated by this command.");
    eprintln!("  Exit status is 0 for allowed and 1 for denied decisions.");
}

fn json_explanation<'a>(
    request: &'a ExplainRequest,
    decision: &'a PermissionDecision,
) -> JsonExplanation<'a> {
    let (reason_code, reason, state) = reason_parts(&decision.reason);
    JsonExplanation {
        request: request.kind.label(),
        team_id: &request.team_id,
        subject_member_id: &request.subject_member_id,
        resource_type: match request.kind {
            ExplanationKind::RepositoryQuestion => "repository",
            ExplanationKind::TaskUpdate => "task",
        },
        resource_id: &request.resource_id,
        action: match request.kind {
            ExplanationKind::RepositoryQuestion => "read_repo",
            ExplanationKind::TaskUpdate => "update_task",
        },
        allowed: decision.allowed,
        reason_code,
        reason,
        state,
        grant_chain: &decision.chain,
        evaluated_at: request.at,
    }
}

fn human_explanation(request: &ExplainRequest, decision: &PermissionDecision) -> String {
    let (_, reason, state) = reason_parts(&decision.reason);
    let chain = if decision.chain.is_empty() {
        "(none)".to_string()
    } else {
        decision.chain.join(" -> ")
    };
    format!(
        "Permission explanation\n\
         request: {}\n\
         team: {}\n\
         subject: {}\n\
         resource: {} {}\n\
         action: {}\n\
         decision: {}\n\
         reason: {}\n\
         state: {}\n\
         grant chain: {}\n\
         evaluated at: {}\n",
        request.kind.label(),
        request.team_id,
        request.subject_member_id,
        match request.kind {
            ExplanationKind::RepositoryQuestion => "repository",
            ExplanationKind::TaskUpdate => "task",
        },
        request.resource_id,
        match request.kind {
            ExplanationKind::RepositoryQuestion => "read_repo",
            ExplanationKind::TaskUpdate => "update_task",
        },
        if decision.allowed {
            "ALLOWED"
        } else {
            "DENIED"
        },
        reason,
        state,
        chain,
        request.at,
    )
}

fn reason_parts(reason: &PermissionDecisionReason) -> (&'static str, String, &'static str) {
    match reason {
        PermissionDecisionReason::Allowed => (
            "allowed",
            "allowed by an active grant chain".into(),
            "active",
        ),
        PermissionDecisionReason::NoMatchingGrant => (
            "no_matching_grant",
            "no matching grant exists".into(),
            "not_granted",
        ),
        PermissionDecisionReason::GrantRevoked(grant_id) => (
            "grant_revoked",
            format!("grant {grant_id} is revoked"),
            "revoked",
        ),
        PermissionDecisionReason::GrantExpired(grant_id) => (
            "grant_expired",
            format!("grant {grant_id} is expired"),
            "expired",
        ),
        PermissionDecisionReason::GrantChainUnavailable(grant_id) => (
            "grant_chain_unavailable",
            format!("grant chain is unavailable at {grant_id}"),
            "unavailable",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::team::{Member, PermissionGrant, Role, Team};
    use crate::team_storage::TeamStore;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn base_args(kind: &str, resource_flag: &str, resource_id: &str) -> Vec<String> {
        vec![
            "explain".into(),
            kind.into(),
            "--db".into(),
            "team.sqlite".into(),
            "--team".into(),
            "team-1".into(),
            "--subject".into(),
            "member-1".into(),
            resource_flag.into(),
            resource_id.into(),
            "--at".into(),
            "42".into(),
        ]
    }

    fn test_database_path() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "herdr-permission-cli-{}-{nanos}.sqlite",
            std::process::id()
        ))
    }

    #[test]
    fn parses_repository_question_request() {
        let request =
            parse_explain_request(&base_args("repository-question", "--repository", "repo-1"))
                .unwrap();

        assert_eq!(request.kind, ExplanationKind::RepositoryQuestion);
        assert_eq!(
            request.resource(),
            PermissionResource::Repository("repo-1".into())
        );
        assert_eq!(request.at, 42);
        assert!(!request.json);
    }

    #[test]
    fn parses_task_update_json_request() {
        let mut args = base_args("task-update", "--task", "task-1");
        args.push("--json".into());
        let request = parse_explain_request(&args).unwrap();

        assert_eq!(request.kind, ExplanationKind::TaskUpdate);
        assert_eq!(
            request.resource(),
            PermissionResource::Task("task-1".into())
        );
        assert!(request.json);
    }

    #[test]
    fn rejects_wrong_resource_flag_and_missing_required_values() {
        let wrong_flag = base_args("task-update", "--repository", "repo-1");
        assert!(parse_explain_request(&wrong_flag).is_err());

        let missing_subject = vec![
            "explain".into(),
            "task-update".into(),
            "--db".into(),
            "team.sqlite".into(),
        ];
        assert!(parse_explain_request(&missing_subject).is_err());
    }

    #[test]
    fn renders_allowed_and_revoked_explanations_with_chain_and_state() {
        let request =
            parse_explain_request(&base_args("repository-question", "--repository", "repo-1"))
                .unwrap();
        let allowed = PermissionDecision {
            allowed: true,
            chain: vec!["grant-manager-lead".into(), "grant-lead-member".into()],
            reason: PermissionDecisionReason::Allowed,
        };
        let output = human_explanation(&request, &allowed);
        assert!(output.contains("decision: ALLOWED"));
        assert!(output.contains("grant chain: grant-manager-lead -> grant-lead-member"));
        assert!(output.contains("state: active"));

        let revoked = PermissionDecision {
            allowed: false,
            chain: vec!["grant-manager-lead".into(), "grant-lead-member".into()],
            reason: PermissionDecisionReason::GrantRevoked("grant-manager-lead".into()),
        };
        let json = serde_json::to_string(&json_explanation(&request, &revoked)).unwrap();
        assert!(json.contains("\\\"decision\\\":false") || json.contains("\"allowed\":false"));
        assert!(json.contains("grant_revoked"));
        assert!(json.contains("revoked"));
        assert!(json.contains("grant-lead-member"));
    }

    #[test]
    fn command_reads_repository_and_task_decisions_from_sqlite() {
        let database = test_database_path();
        let store = TeamStore::open(&database).unwrap();
        let team = Team::create("CLI team", "manager-1", 1).unwrap();
        store.save_team(&team).unwrap();
        for (member_id, display_name) in [("manager-1", "Manager"), ("member-1", "Member")] {
            store
                .save_member(
                    &team.team_id,
                    &Member {
                        member_id: member_id.into(),
                        display_name: display_name.into(),
                        role: Role::Implementer,
                        active: true,
                    },
                    1,
                )
                .unwrap();
        }
        store
            .grant_permission(
                &team.team_id,
                &PermissionGrant {
                    grant_id: "grant-cli-repository".into(),
                    subject_member_id: "member-1".into(),
                    resource: PermissionResource::Repository("repo-1".into()),
                    action: PermissionAction::ReadRepository,
                    grantor_member_id: "manager-1".into(),
                    delegable: false,
                    expires_at: None,
                    revoked: false,
                },
                1,
            )
            .unwrap();
        drop(store);

        let repository_args = vec![
            "explain".into(),
            "repository-question".into(),
            "--db".into(),
            database.display().to_string(),
            "--team".into(),
            team.team_id.clone(),
            "--subject".into(),
            "member-1".into(),
            "--repository".into(),
            "repo-1".into(),
            "--at".into(),
            "2".into(),
            "--json".into(),
        ];
        assert_eq!(run_permission_command(&repository_args).unwrap(), 0);

        let task_args = vec![
            "explain".into(),
            "task-update".into(),
            "--db".into(),
            database.display().to_string(),
            "--team".into(),
            team.team_id,
            "--subject".into(),
            "member-1".into(),
            "--task".into(),
            "task-1".into(),
            "--at".into(),
            "2".into(),
        ];
        assert_eq!(run_permission_command(&task_args).unwrap(), 1);

        let _ = fs::remove_file(database);
    }
}
