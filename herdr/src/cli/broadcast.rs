use serde::Serialize;
use serde_json::json;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::identity::DeviceIdentity;
use crate::team::{
    validate_broadcast_message, validate_broadcast_scope, BroadcastRecord, Event, EventType,
};
use crate::team_storage::TeamStore;

const BROADCAST_USAGE: &str = concat!(
    "usage: herdr broadcast <create|list|show> ",
    "--db PATH [options]"
);

#[derive(Clone, Debug, Eq, PartialEq)]
enum BroadcastCommand {
    Create {
        database: PathBuf,
        team_id: String,
        actor_member_id: String,
        identity: PathBuf,
        scope: String,
        message: String,
        at: u64,
        json: bool,
    },
    List {
        database: PathBuf,
        scope: Option<String>,
        json: bool,
    },
    Show {
        database: PathBuf,
        message_id: String,
        json: bool,
    },
}

#[derive(Serialize)]
struct BroadcastOutput<'a> {
    event_id: Option<&'a str>,
    broadcast: Option<&'a BroadcastRecord>,
    broadcasts: &'a [BroadcastRecord],
}

pub(crate) fn run_broadcast_command(args: &[String]) -> std::io::Result<i32> {
    if args.is_empty()
        || matches!(
            args.first().map(String::as_str),
            Some("help" | "--help" | "-h")
        )
    {
        print_help();
        return Ok(0);
    }

    let command = match parse_broadcast_command(args) {
        Ok(command) => command,
        Err(message) => {
            eprintln!("broadcast: {message}");
            eprintln!("{BROADCAST_USAGE}");
            return Ok(2);
        }
    };

    match command {
        BroadcastCommand::Create {
            database,
            team_id,
            actor_member_id,
            identity: identity_path,
            scope,
            message,
            at,
            json,
        } => {
            let identity = DeviceIdentity::load_existing(&identity_path).map_err(|error| {
                std::io::Error::other(format!("could not load device identity: {error}"))
            })?;
            let message_id = format!("message-{}", unique_suffix());
            let sender_member_id = actor_member_id.clone();
            let mut event = Event::create(
                team_id,
                actor_member_id,
                identity.device_id().to_string(),
                EventType::BroadcastCreated,
                json!({
                    "message_id": message_id.clone(),
                    "sender_member_id": sender_member_id,
                    "scope": scope,
                    "message": message,
                    "created_at": at,
                }),
                at,
                "pending-signature",
            )
            .map_err(|error| std::io::Error::other(format!("invalid broadcast event: {error}")))?;
            event.signature = Some(identity.sign(&event.signing_bytes()));

            let mut store = TeamStore::open(&database).map_err(storage_error)?;
            store.insert_event(&event, at).map_err(storage_error)?;
            match super::relay::auto_sync_after_write(store.path(), &event.team_id) {
                Ok(true) => {}
                Ok(false) => eprintln!("broadcast saved locally; set HERDR_RELAY_URL and HERDR_RELAY_TOKEN to deliver it over the Internet"),
                Err(error) => eprintln!("broadcast saved locally but relay delivery failed: {error}; retry with `herdr relay sync`"),
            }
            let broadcast = store
                .broadcast(&message_id)
                .map_err(storage_error)?
                .ok_or_else(|| std::io::Error::other("broadcast projection was not created"))?;
            print_broadcast_output(Some(&event.event_id), Some(&broadcast), &[], json);
            Ok(0)
        }
        BroadcastCommand::List {
            database,
            scope,
            json,
        } => {
            let store = TeamStore::open_read_only(&database).map_err(|error| {
                std::io::Error::other(format!(
                    "could not open broadcast database read-only: {error}"
                ))
            })?;
            let broadcasts = store.broadcasts(scope.as_deref()).map_err(storage_error)?;
            print_broadcast_output(None, None, &broadcasts, json);
            Ok(0)
        }
        BroadcastCommand::Show {
            database,
            message_id,
            json,
        } => {
            let store = TeamStore::open_read_only(&database).map_err(|error| {
                std::io::Error::other(format!(
                    "could not open broadcast database read-only: {error}"
                ))
            })?;
            let broadcast = store.broadcast(&message_id).map_err(storage_error)?;
            let Some(broadcast) = broadcast else {
                eprintln!("broadcast: message not found: {message_id}");
                return Ok(1);
            };
            print_broadcast_output(None, Some(&broadcast), &[], json);
            Ok(0)
        }
    }
}

fn parse_broadcast_command(args: &[String]) -> Result<BroadcastCommand, String> {
    let kind = args
        .first()
        .map(String::as_str)
        .ok_or_else(|| "missing broadcast subcommand".to_string())?;
    let mut database = None;
    let mut team_id = None;
    let mut actor_member_id = None;
    let mut identity = None;
    let mut scope = None;
    let mut message = None;
    let mut message_id = None;
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
        "--scope" => {
            scope = Some(parse_scope(&value)?);
            Ok(())
        }
        "--message" => {
            message = Some(value);
            Ok(())
        }
        "--message-id" => {
            message_id = Some(value);
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
        _ => Err(format!("unknown option: {flag}")),
    })?;

    let database = required_path(database, "--db")?;
    match kind {
        "create" => {
            let scope = scope.ok_or_else(|| "missing --scope".to_string())?;
            let message = required_text(message, "--message")?;
            validate_broadcast_message(&message).map_err(|error| error.to_string())?;
            Ok(BroadcastCommand::Create {
                database,
                team_id: required_text(team_id, "--team")?,
                actor_member_id: required_text(actor_member_id, "--actor")?,
                identity: required_path(identity, "--identity")?,
                scope,
                message,
                at: at.unwrap_or_else(current_unix_seconds),
                json: json_output,
            })
        }
        "list" => Ok(BroadcastCommand::List {
            database,
            scope,
            json: json_output,
        }),
        "show" => Ok(BroadcastCommand::Show {
            database,
            message_id: required_text(message_id, "--message-id")?,
            json: json_output,
        }),
        _ => Err("expected create, list, or show".into()),
    }
}

fn parse_scope(value: &str) -> Result<String, String> {
    validate_broadcast_scope(value).map_err(|error| error.to_string())?;
    Ok(value.to_string())
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
    Ok(PathBuf::from(required_text(value, flag)?))
}

fn required_text(value: Option<String>, flag: &str) -> Result<String, String> {
    let value = value.ok_or_else(|| format!("missing {flag}"))?;
    if value.trim().is_empty() {
        return Err(format!("{flag} must not be empty"));
    }
    Ok(value)
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
    eprintln!("herdr broadcast commands:");
    eprintln!("  {BROADCAST_USAGE}");
    eprintln!("  create records a signed team or group broadcast.");
    eprintln!("  list displays persisted broadcasts; add --scope to filter team or group:id.");
    eprintln!("  show reads one broadcast using a read-only database connection.");
}

fn print_broadcast_output(
    event_id: Option<&str>,
    broadcast: Option<&BroadcastRecord>,
    broadcasts: &[BroadcastRecord],
    json_output: bool,
) {
    if json_output {
        let output = BroadcastOutput {
            event_id,
            broadcast,
            broadcasts,
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&output).expect("broadcast output must be serializable")
        );
        return;
    }
    if let Some(event_id) = event_id {
        println!("event: {event_id}");
    }
    if let Some(broadcast) = broadcast {
        print_broadcast(broadcast);
    }
    for broadcast in broadcasts {
        print_broadcast(broadcast);
    }
    if broadcast.is_none() && broadcasts.is_empty() {
        println!("no broadcasts found");
    }
}

fn print_broadcast(broadcast: &BroadcastRecord) {
    println!("message: {}", broadcast.message_id);
    println!("sender: {}", broadcast.sender_member_id);
    println!("scope: {}", broadcast.scope);
    println!("created_at: {}", broadcast.created_at);
    println!("{}", broadcast.message);
}

fn storage_error(error: crate::team_storage::StorageError) -> std::io::Error {
    std::io::Error::other(format!("broadcast storage error: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::DeviceIdentity;
    use crate::team::{Device, Member, Role, Team};
    use crate::team_storage::TeamStore;
    use std::fs;

    fn test_path(label: &str, extension: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "team-assistant-cli-broadcast-{label}-{}-{nanos}.{extension}",
            std::process::id()
        ))
    }

    #[test]
    fn parses_and_rejects_broadcast_scopes() {
        assert!(parse_scope("team").is_ok());
        assert!(parse_scope("group:platform").is_ok());
        assert!(parse_scope("group:").is_err());
        assert!(parse_scope("repository:secret").is_err());
    }

    #[test]
    fn broadcast_commands_create_list_and_show() {
        let database = test_path("workflow", "sqlite");
        let identity_path = test_path("workflow", "json");
        let identity = DeviceIdentity::load_or_create(&identity_path).unwrap();
        let member = Member {
            member_id: "member-1".into(),
            display_name: "Local member".into(),
            role: Role::Implementer,
            active: true,
        };
        let team = Team::create("CLI broadcast team", member.member_id.clone(), 1).unwrap();
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

        let create_team = vec![
            "create".into(),
            "--db".into(),
            database.to_string_lossy().into_owned(),
            "--team".into(),
            team.team_id.clone(),
            "--actor".into(),
            member.member_id.clone(),
            "--identity".into(),
            identity_path.to_string_lossy().into_owned(),
            "--scope".into(),
            "team".into(),
            "--message".into(),
            "The release is ready.".into(),
            "--at".into(),
            "10".into(),
        ];
        assert_eq!(run_broadcast_command(&create_team).unwrap(), 0);

        let first_message_id = {
            let connection = rusqlite::Connection::open(&database).unwrap();
            connection
                .query_row(
                    "SELECT message_id FROM broadcasts ORDER BY created_at",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap()
        };

        let create_group = [
            "create",
            "--db",
            database.to_string_lossy().as_ref(),
            "--team",
            team.team_id.as_str(),
            "--actor",
            member.member_id.as_str(),
            "--identity",
            identity_path.to_string_lossy().as_ref(),
            "--scope",
            "group:platform",
            "--message",
            "Platform deploy starts at 18:00.",
            "--at",
            "11",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
        assert_eq!(run_broadcast_command(&create_group).unwrap(), 0);

        let list_group = vec![
            "list".into(),
            "--db".into(),
            database.to_string_lossy().into_owned(),
            "--scope".into(),
            "group:platform".into(),
            "--json".into(),
        ];
        assert_eq!(run_broadcast_command(&list_group).unwrap(), 0);

        let show = vec![
            "show".into(),
            "--db".into(),
            database.to_string_lossy().into_owned(),
            "--message-id".into(),
            first_message_id,
            "--json".into(),
        ];
        assert_eq!(run_broadcast_command(&show).unwrap(), 0);

        let store = TeamStore::open(&database).unwrap();
        assert_eq!(store.broadcasts(None).unwrap().len(), 2);
        assert_eq!(store.broadcasts(Some("group:platform")).unwrap().len(), 1);
        assert_eq!(
            store.broadcasts(Some("group:platform")).unwrap()[0].message,
            "Platform deploy starts at 18:00."
        );

        drop((store, identity));
        let _ = fs::remove_file(database);
        let _ = fs::remove_file(identity_path);
    }
}
