use serde::Serialize;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::identity::DeviceIdentity;
use crate::team::{Device, Member, RepositoryApproval, Role};
use crate::team_storage::TeamStore;

const ONBOARD_USAGE: &str = concat!("usage: herdr onboard <setup|show> ", "--db PATH [options]");

#[derive(Clone, Debug, Eq, PartialEq)]
enum OnboardCommand {
    Setup {
        database: PathBuf,
        identity: PathBuf,
        member_id: String,
        display_name: String,
        role: Role,
        repository_id: String,
        repository_path: PathBuf,
        at: u64,
        json: bool,
    },
    Show {
        database: PathBuf,
        json: bool,
    },
}

#[derive(Serialize)]
struct OnboardOutput<'a> {
    member_id: &'a str,
    display_name: &'a str,
    role: &'a Role,
    active: bool,
    device_id: Option<&'a str>,
    repositories: &'a [RepositoryApproval],
}

pub(crate) fn run_onboard_command(args: &[String]) -> std::io::Result<i32> {
    if args.is_empty()
        || matches!(
            args.first().map(String::as_str),
            Some("help" | "--help" | "-h")
        )
    {
        print_help();
        return Ok(0);
    }

    let command = match parse_onboard_command(args) {
        Ok(command) => command,
        Err(message) => {
            eprintln!("onboard: {message}");
            eprintln!("{ONBOARD_USAGE}");
            return Ok(2);
        }
    };

    match command {
        OnboardCommand::Setup {
            database,
            identity: identity_path,
            member_id,
            display_name,
            role,
            repository_id,
            repository_path,
            at,
            json,
        } => {
            validate_repository_path(&repository_path).map_err(std::io::Error::other)?;
            let identity = DeviceIdentity::load_or_create(&identity_path).map_err(|error| {
                std::io::Error::other(format!("could not create or load device identity: {error}"))
            })?;
            let member = Member {
                member_id,
                display_name,
                role,
                active: true,
            };
            let device = Device {
                device_id: identity.device_id().into(),
                member_id: member.member_id.clone(),
                public_key: identity.public_key(),
                active: true,
            };
            let approval = RepositoryApproval {
                repository_id,
                owner_member_id: member.member_id.clone(),
                local_path: repository_path.to_string_lossy().into_owned(),
                approved_at: at,
            };
            let store = TeamStore::open(&database).map_err(storage_error)?;
            store.save_profile(&member, at, at).map_err(storage_error)?;
            store.save_device(&device, at).map_err(storage_error)?;
            store
                .save_repository_approval(&approval)
                .map_err(storage_error)?;
            let repositories = vec![approval];
            print_output(&member, Some(identity.device_id()), &repositories, json);
            Ok(0)
        }
        OnboardCommand::Show { database, json } => {
            let store = TeamStore::open_read_only(&database).map_err(|error| {
                std::io::Error::other(format!(
                    "could not open onboarding database read-only: {error}"
                ))
            })?;
            let Some(profile) = store.load_profile().map_err(storage_error)? else {
                eprintln!("onboard: no local profile has been configured");
                return Ok(1);
            };
            let repositories = store.repository_approvals().map_err(storage_error)?;
            print_output(&profile.member, None, &repositories, json);
            Ok(0)
        }
    }
}

fn parse_onboard_command(args: &[String]) -> Result<OnboardCommand, String> {
    let kind = args
        .first()
        .map(String::as_str)
        .ok_or_else(|| "missing onboard subcommand".to_string())?;
    let mut database = None;
    let mut identity = None;
    let mut member_id = None;
    let mut display_name = None;
    let mut role = None;
    let mut repository_id = None;
    let mut repository_path = None;
    let mut at = None;
    let mut json_output = false;

    parse_options(&args[1..], |flag, value| match flag {
        "--db" => {
            database = Some(value);
            Ok(())
        }
        "--identity" => {
            identity = Some(value);
            Ok(())
        }
        "--member" => {
            member_id = Some(value);
            Ok(())
        }
        "--display-name" => {
            display_name = Some(value);
            Ok(())
        }
        "--role" => {
            role = Some(parse_role(&value)?);
            Ok(())
        }
        "--repository-id" => {
            repository_id = Some(value);
            Ok(())
        }
        "--repository-path" => {
            repository_path = Some(PathBuf::from(value));
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
        "setup" => {
            let repository_path =
                repository_path.ok_or_else(|| "missing --repository-path".to_string())?;
            validate_repository_path(&repository_path)?;
            Ok(OnboardCommand::Setup {
                database,
                identity: required_path(identity, "--identity")?,
                member_id: required_text(member_id, "--member")?,
                display_name: required_text(display_name, "--display-name")?,
                role: role.ok_or_else(|| "missing --role".to_string())?,
                repository_id: required_text(repository_id, "--repository-id")?,
                repository_path,
                at: at.unwrap_or_else(current_unix_seconds),
                json: json_output,
            })
        }
        "show" => Ok(OnboardCommand::Show {
            database,
            json: json_output,
        }),
        _ => Err("expected setup or show".into()),
    }
}

fn parse_role(value: &str) -> Result<Role, String> {
    match value {
        "manager" => Ok(Role::Manager),
        "group-head" | "group_head" => Ok(Role::GroupHead),
        "part-head" | "part_head" => Ok(Role::PartHead),
        "implementer" => Ok(Role::Implementer),
        _ => Err("invalid --role; expected manager, group-head, part-head, or implementer".into()),
    }
}

fn validate_repository_path(path: &PathBuf) -> Result<(), String> {
    let metadata = fs::metadata(path)
        .map_err(|error| format!("repository path is not accessible: {error}"))?;
    if !metadata.is_dir() {
        return Err("repository path must be a directory".into());
    }
    Ok(())
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

fn print_help() {
    eprintln!("herdr onboard commands:");
    eprintln!("  {ONBOARD_USAGE}");
    eprintln!("  setup stores a local display name, role label, identity, and approved repository metadata.");
    eprintln!("  show restores the saved profile and repository metadata using a read-only database connection.");
}

fn print_output(
    member: &Member,
    device_id: Option<&str>,
    repositories: &[RepositoryApproval],
    json_output: bool,
) {
    if json_output {
        let output = OnboardOutput {
            member_id: &member.member_id,
            display_name: &member.display_name,
            role: &member.role,
            active: member.active,
            device_id,
            repositories,
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&output).expect("onboarding output must be serializable")
        );
        return;
    }
    println!("member: {}", member.member_id);
    println!("display name: {}", member.display_name);
    println!("role: {:?}", member.role);
    println!("active: {}", member.active);
    if let Some(device_id) = device_id {
        println!("device: {device_id}");
    }
    for repository in repositories {
        println!("repository: {}", repository.repository_id);
        println!("repository path: {}", repository.local_path);
        println!("approved at: {}", repository.approved_at);
    }
}

fn storage_error(error: crate::team_storage::StorageError) -> std::io::Error {
    std::io::Error::other(format!("onboarding storage error: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::team_storage::TeamStore;

    fn test_path(label: &str, extension: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "team-assistant-onboard-{label}-{}-{nanos}.{extension}",
            std::process::id()
        ))
    }

    #[test]
    fn parses_role_labels_and_rejects_unknown_role() {
        assert_eq!(parse_role("manager"), Ok(Role::Manager));
        assert_eq!(parse_role("group_head"), Ok(Role::GroupHead));
        assert_eq!(parse_role("part-head"), Ok(Role::PartHead));
        assert_eq!(parse_role("implementer"), Ok(Role::Implementer));
        assert!(parse_role("owner").is_err());
    }

    #[test]
    fn onboarding_setup_persists_profile_and_repository_metadata_across_restart() {
        let database = test_path("profile", "sqlite");
        let identity_path = test_path("profile", "json");
        let repository_path = test_path("approved-repository", "dir");
        fs::create_dir_all(&repository_path).unwrap();

        let args = vec![
            "setup".into(),
            "--db".into(),
            database.to_string_lossy().into_owned(),
            "--identity".into(),
            identity_path.to_string_lossy().into_owned(),
            "--member".into(),
            "implementer-1".into(),
            "--display-name".into(),
            "Ravi".into(),
            "--role".into(),
            "implementer".into(),
            "--repository-id".into(),
            "repo-local-1".into(),
            "--repository-path".into(),
            repository_path.to_string_lossy().into_owned(),
            "--at".into(),
            "10".into(),
            "--json".into(),
        ];
        assert_eq!(run_onboard_command(&args).unwrap(), 0);

        let store = TeamStore::open(&database).unwrap();
        let profile = store.load_profile().unwrap().unwrap();
        assert_eq!(profile.member.member_id, "implementer-1");
        assert_eq!(profile.member.display_name, "Ravi");
        assert_eq!(profile.member.role, Role::Implementer);
        let repositories = store.repository_approvals().unwrap();
        assert_eq!(repositories.len(), 1);
        assert_eq!(repositories[0].repository_id, "repo-local-1");
        assert_eq!(
            repositories[0].local_path,
            repository_path.to_string_lossy().as_ref()
        );
        drop(store);

        let show = vec![
            "show".into(),
            "--db".into(),
            database.to_string_lossy().into_owned(),
            "--json".into(),
        ];
        assert_eq!(run_onboard_command(&show).unwrap(), 0);
        assert!(DeviceIdentity::load_existing(&identity_path).is_ok());

        let _ = fs::remove_file(database);
        let _ = fs::remove_file(identity_path);
        let _ = fs::remove_dir_all(repository_path);
    }
}
