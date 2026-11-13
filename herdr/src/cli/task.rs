use serde::Serialize;
use serde_json::json;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::identity::DeviceIdentity;
use crate::team::{Event, EventType, TaskActivityRecord, TaskRecord, TaskStatus};
use crate::team_storage::TeamStore;

const TASK_USAGE: &str = concat!(
    "usage: herdr task <create|status|follow-up|show> ",
    "--db PATH --team TEAM_ID [options]"
);

#[derive(Clone, Debug, Eq, PartialEq)]
enum TaskCommand {
    Create {
        database: PathBuf,
        team_id: String,
        actor_member_id: String,
        identity: PathBuf,
        assignee_member_id: String,
        description: String,
        at: u64,
        json: bool,
    },
    Status {
        database: PathBuf,
        team_id: String,
        actor_member_id: String,
        identity: PathBuf,
        task_id: String,
        status: TaskStatus,
        at: u64,
        json: bool,
    },
    FollowUp {
        database: PathBuf,
        team_id: String,
        actor_member_id: String,
        identity: PathBuf,
        task_id: String,
        message: String,
        at: u64,
        json: bool,
    },
    Show {
        database: PathBuf,
        task_id: String,
        json: bool,
    },
}

#[derive(Serialize)]
struct TaskOutput<'a> {
    event_id: Option<&'a str>,
    task: Option<&'a TaskRecord>,
    activity: &'a [TaskActivityRecord],
}

pub(crate) fn run_task_command(args: &[String]) -> std::io::Result<i32> {
    if args.is_empty()
        || matches!(
            args.first().map(String::as_str),
            Some("help" | "--help" | "-h")
        )
    {
        print_help();
        return Ok(0);
    }

    let command = match parse_task_command(args) {
        Ok(command) => command,
        Err(message) => {
            eprintln!("task: {message}");
            eprintln!("{TASK_USAGE}");
            return Ok(2);
        }
    };

    match command {
        TaskCommand::Show {
            database,
            task_id,
            json,
        } => {
            let store = TeamStore::open_read_only(&database).map_err(|error| {
                std::io::Error::other(format!("could not open task database read-only: {error}"))
            })?;
            let task = store.task(&task_id).map_err(storage_error)?;
            let Some(task) = task else {
                eprintln!("task: task not found: {task_id}");
                return Ok(1);
            };
            let activity = store.task_activity(&task_id).map_err(storage_error)?;
            print_task_output(None, Some(&task), &activity, json);
            Ok(0)
        }
        command => {
            let (database, team_id, actor_member_id, identity_path, at, json) =
                command_context(&command);
            let identity = DeviceIdentity::load_existing(identity_path).map_err(|error| {
                std::io::Error::other(format!("could not load device identity: {error}"))
            })?;
            let (event_type, payload, task_id) = match &command {
                TaskCommand::Create {
                    assignee_member_id,
                    description,
                    ..
                } => (
                    EventType::TaskCreated,
                    json!({
                        "task_id": format!("task-{}", unique_suffix()),
                        "assigner_member_id": actor_member_id,
                        "assignee_member_id": assignee_member_id,
                        "description": description,
                        "created_at": at,
                    }),
                    None,
                ),
                TaskCommand::Status {
                    task_id, status, ..
                } => (
                    EventType::TaskStatusChanged,
                    json!({ "task_id": task_id, "status": status }),
                    Some(task_id.clone()),
                ),
                TaskCommand::FollowUp {
                    task_id, message, ..
                } => (
                    EventType::TaskFollowUpAdded,
                    json!({
                        "task_id": task_id,
                        "message": message,
                        "created_at": at,
                    }),
                    Some(task_id.clone()),
                ),
                TaskCommand::Show { .. } => unreachable!(),
            };
            let generated_task_id = payload["task_id"].as_str().map(str::to_owned);
            let mut event = Event::create(
                team_id.clone(),
                actor_member_id.clone(),
                identity.device_id().to_string(),
                event_type,
                payload,
                at,
                "pending-signature",
            )
            .map_err(|error| std::io::Error::other(format!("invalid task event: {error}")))?;
            event.signature = Some(identity.sign(&event.signing_bytes()));

            let mut store = TeamStore::open(database).map_err(storage_error)?;
            store.insert_event(&event, at).map_err(storage_error)?;
            let task_id = task_id
                .or(generated_task_id)
                .ok_or_else(|| std::io::Error::other("task event did not contain a task ID"))?;
            let task = store.task(&task_id).map_err(storage_error)?;
            let activity = store.task_activity(&task_id).map_err(storage_error)?;
            print_task_output(Some(&event.event_id), task.as_ref(), &activity, json);
            Ok(0)
        }
    }
}

fn command_context(command: &TaskCommand) -> (&PathBuf, &String, &String, &PathBuf, u64, bool) {
    match command {
        TaskCommand::Create {
            database,
            team_id,
            actor_member_id,
            identity,
            at,
            json,
            ..
        }
        | TaskCommand::Status {
            database,
            team_id,
            actor_member_id,
            identity,
            at,
            json,
            ..
        }
        | TaskCommand::FollowUp {
            database,
            team_id,
            actor_member_id,
            identity,
            at,
            json,
            ..
        } => (database, team_id, actor_member_id, identity, *at, *json),
        TaskCommand::Show { .. } => unreachable!(),
    }
}

fn parse_task_command(args: &[String]) -> Result<TaskCommand, String> {
    let kind = args
        .first()
        .map(String::as_str)
        .ok_or_else(|| "missing task subcommand".to_string())?;
    if kind == "show" {
        let mut database = None;
        let mut task_id = None;
        let mut json_output = false;
        parse_options(&args[1..], |flag, value| match flag {
            "--db" => {
                database = Some(value);
                Ok(())
            }
            "--task" => {
                task_id = Some(value);
                Ok(())
            }
            "--json" => {
                json_output = true;
                Ok(())
            }
            _ => return Err(format!("unknown option: {flag}")),
        })?;
        return Ok(TaskCommand::Show {
            database: required_path(database, "--db")?,
            task_id: required_text(task_id, "--task")?,
            json: json_output,
        });
    }

    let mut database = None;
    let mut team_id = None;
    let mut actor_member_id = None;
    let mut identity = None;
    let mut assignee_member_id = None;
    let mut description = None;
    let mut task_id = None;
    let mut status = None;
    let mut message = None;
    let mut at = None;
    let mut json_output = false;
    parse_options(&args[1..], |flag, value| match flag {
        "--db" => {
            database = Some(value);
            Ok(())
        }
        "--team" => {
            team_id = Some(value);
            Ok(())
        }
        "--actor" => {
            actor_member_id = Some(value);
            Ok(())
        }
        "--identity" => {
            identity = Some(value);
            Ok(())
        }
        "--assignee" => {
            assignee_member_id = Some(value);
            Ok(())
        }
        "--description" => {
            description = Some(value);
            Ok(())
        }
        "--task" => {
            task_id = Some(value);
            Ok(())
        }
        "--status" => {
            status = Some(value);
            Ok(())
        }
        "--message" => {
            message = Some(value);
            Ok(())
        }
        "--at" => {
            at = Some(
                value
                    .parse::<u64>()
                    .map_err(|_| "--at must be an unsigned Unix timestamp".to_string())?,
            );
            Ok(())
        }
        "--json" => {
            json_output = true;
            Ok(())
        }
        _ => return Err(format!("unknown option: {flag}")),
    })?;

    let database = required_path(database, "--db")?;
    let team_id = required_text(team_id, "--team")?;
    let actor_member_id = required_text(actor_member_id, "--actor")?;
    let identity = required_path(identity, "--identity")?;
    let at = at.unwrap_or_else(current_unix_seconds);
    match kind {
        "create" => Ok(TaskCommand::Create {
            database,
            team_id,
            actor_member_id,
            identity,
            assignee_member_id: required_text(assignee_member_id, "--assignee")?,
            description: required_text(description, "--description")?,
            at,
            json: json_output,
        }),
        "status" => Ok(TaskCommand::Status {
            database,
            team_id,
            actor_member_id,
            identity,
            task_id: required_text(task_id, "--task")?,
            status: parse_status(&required_text(status, "--status")?)?,
            at,
            json: json_output,
        }),
        "follow-up" => Ok(TaskCommand::FollowUp {
            database,
            team_id,
            actor_member_id,
            identity,
            task_id: required_text(task_id, "--task")?,
            message: required_text(message, "--message")?,
            at,
            json: json_output,
        }),
        _ => Err("expected create, status, follow-up, or show".into()),
    }
}

fn parse_options<F>(args: &[String], mut handler: F) -> Result<(), String>
where
    F: FnMut(&str, String) -> Result<(), String>,
{
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        if flag == "--json" {
            handler(flag, String::new())?;
            index += 1;
            continue;
        }
        if !flag.starts_with("--") {
            return Err(format!("unexpected argument: {flag}"));
        }
        let value = args
            .get(index + 1)
            .ok_or_else(|| format!("{flag} requires a value"))?;
        if value.starts_with('-') {
            return Err(format!("{flag} requires a value"));
        }
        handler(flag, value.clone())?;
        index += 2;
    }
    Ok(())
}

fn required_path(value: Option<String>, flag: &str) -> Result<PathBuf, String> {
    let value = required_text(value, flag)?;
    Ok(PathBuf::from(value))
}

fn required_text(value: Option<String>, flag: &str) -> Result<String, String> {
    let value = value.ok_or_else(|| format!("missing {flag}"))?;
    if value.trim().is_empty() {
        return Err(format!("{flag} must not be empty"));
    }
    Ok(value)
}

fn parse_status(value: &str) -> Result<TaskStatus, String> {
    match value {
        "queued" => Ok(TaskStatus::Queued),
        "delivered" => Ok(TaskStatus::Delivered),
        "in_progress" => Ok(TaskStatus::InProgress),
        "done" => Ok(TaskStatus::Done),
        "cancelled" => Ok(TaskStatus::Cancelled),
        "failed" => Ok(TaskStatus::Failed),
        "needs_attention" => Ok(TaskStatus::NeedsAttention),
        _ => Err(format!(
            "invalid --status; expected queued, delivered, in_progress, done, cancelled, failed, or needs_attention"
        )),
    }
}

fn current_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("{nanos}-{}", std::process::id())
}

fn print_help() {
    eprintln!("herdr task commands:");
    eprintln!("  {TASK_USAGE}");
    eprintln!("  create creates a signed task event for a local team member.");
    eprintln!("  status validates and records a task status transition.");
    eprintln!("  follow-up records a bounded task follow-up activity.");
    eprintln!("  show reads a task and its activity using a read-only database connection.");
}

fn print_task_output(
    event_id: Option<&str>,
    task: Option<&TaskRecord>,
    activity: &[TaskActivityRecord],
    json_output: bool,
) {
    if json_output {
        let output = TaskOutput {
            event_id,
            task,
            activity,
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&output).expect("task output must be serializable")
        );
        return;
    }
    if let Some(event_id) = event_id {
        println!("event: {event_id}");
    }
    if let Some(task) = task {
        println!("task: {}", task.task_id);
        println!("assigner: {}", task.assigner_member_id);
        println!("assignee: {}", task.assignee_member_id);
        println!("status: {:?}", task.status);
        println!("description: {}", task.description);
        println!("activity:");
        for item in activity {
            println!(
                "  {} {}: {}",
                item.created_at, item.activity_type, item.message
            );
        }
    }
}

fn storage_error(error: crate::team_storage::StorageError) -> std::io::Error {
    std::io::Error::other(format!("task storage error: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::DeviceIdentity;
    use crate::team::{Device, Member, Role, Team};
    use crate::team_storage::TeamStore;
    use std::fs;
    use std::path::Path;

    fn test_path(label: &str, extension: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "team-assistant-cli-task-{label}-{}-{nanos}.{extension}",
            std::process::id()
        ))
    }

    fn task_command_args(
        subcommand: &str,
        database: &Path,
        team_id: &str,
        actor_member_id: &str,
        identity: &Path,
        extra: &[&str],
    ) -> Vec<String> {
        let mut args = vec![
            subcommand.to_string(),
            "--db".into(),
            database.to_string_lossy().into_owned(),
            "--team".into(),
            team_id.into(),
            "--actor".into(),
            actor_member_id.into(),
            "--identity".into(),
            identity.to_string_lossy().into_owned(),
        ];
        args.extend(extra.iter().map(|value| (*value).to_string()));
        args
    }

    #[test]
    fn parses_task_status_values() {
        assert_eq!(parse_status("in_progress"), Ok(TaskStatus::InProgress));
        assert_eq!(
            parse_status("needs_attention"),
            Ok(TaskStatus::NeedsAttention)
        );
        assert!(parse_status("running").is_err());
    }

    #[test]
    fn rejects_missing_task_creation_fields() {
        let args = vec!["create".into(), "--db".into(), "tasks.sqlite".into()];
        assert!(parse_task_command(&args).is_err());
    }

    #[test]
    fn task_commands_create_progress_follow_up_show_and_reject_invalid_transition() {
        let database = test_path("workflow", "sqlite");
        let identity_path = test_path("workflow", "json");
        let identity = DeviceIdentity::load_or_create(&identity_path).unwrap();
        let member = Member {
            member_id: "member-1".into(),
            display_name: "Local member".into(),
            role: Role::Implementer,
            active: true,
        };
        let team = Team::create("CLI task team", member.member_id.clone(), 1).unwrap();
        let device = Device {
            device_id: identity.device_id().into(),
            member_id: member.member_id.clone(),
            public_key: identity.public_key(),
            active: true,
        };
        {
            let store = TeamStore::open(&database).unwrap();
            store.save_profile(&member, 1, 1).unwrap();
            store.save_team(&team).unwrap();
            store.save_member(&team.team_id, &member, 1).unwrap();
            store.save_device(&device, 1).unwrap();
        }

        let create = task_command_args(
            "create",
            &database,
            &team.team_id,
            &member.member_id,
            &identity_path,
            &[
                "--assignee",
                "member-1",
                "--description",
                "review the CLI workflow",
                "--at",
                "10",
                "--json",
            ],
        );
        assert_eq!(run_task_command(&create).unwrap(), 0);

        let task_id = {
            let connection = rusqlite::Connection::open(&database).unwrap();
            connection
                .query_row("SELECT task_id FROM tasks", [], |row| {
                    row.get::<_, String>(0)
                })
                .unwrap()
        };
        for (status, at) in [("delivered", "11"), ("in_progress", "12")] {
            let args = task_command_args(
                "status",
                &database,
                &team.team_id,
                &member.member_id,
                &identity_path,
                &["--task", &task_id, "--status", status, "--at", at, "--json"],
            );
            assert_eq!(run_task_command(&args).unwrap(), 0);
        }

        let follow_up = task_command_args(
            "follow-up",
            &database,
            &team.team_id,
            &member.member_id,
            &identity_path,
            &[
                "--task",
                &task_id,
                "--message",
                "Please attach the test output.",
                "--at",
                "13",
                "--json",
            ],
        );
        assert_eq!(run_task_command(&follow_up).unwrap(), 0);

        let done = task_command_args(
            "status",
            &database,
            &team.team_id,
            &member.member_id,
            &identity_path,
            &[
                "--task", &task_id, "--status", "done", "--at", "14", "--json",
            ],
        );
        assert_eq!(run_task_command(&done).unwrap(), 0);

        let show = vec![
            "show".into(),
            "--db".into(),
            database.to_string_lossy().into_owned(),
            "--task".into(),
            task_id.clone(),
            "--json".into(),
        ];
        assert_eq!(run_task_command(&show).unwrap(), 0);

        let invalid = task_command_args(
            "status",
            &database,
            &team.team_id,
            &member.member_id,
            &identity_path,
            &["--task", &task_id, "--status", "queued", "--at", "15"],
        );
        assert!(run_task_command(&invalid).is_err());

        let store = TeamStore::open(&database).unwrap();
        let task = store.task(&task_id).unwrap().unwrap();
        assert_eq!(task.status, TaskStatus::Done);
        assert_eq!(store.task_activity(&task_id).unwrap().len(), 5);

        drop((store, identity));
        let _ = fs::remove_file(database);
        let _ = fs::remove_file(identity_path);
    }
}
