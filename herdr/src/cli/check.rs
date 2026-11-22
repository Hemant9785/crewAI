//! `/check` request creation, execution, and bounded result inspection.
//!
//! Requests are signed team events and are queued in the persistent outbox.
//! Execution is deliberately confirmation-gated on platforms where Herdr has
//! not installed a verified read-only filesystem boundary.

use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::agent_runner::{self, AgentRequest, AgentRunState, CancellationToken};
use crate::execution_boundary::{self, CheckExecutionPolicy};
use crate::identity::DeviceIdentity;
use crate::outbox;
use crate::team::{
    DeliveryState, Event, EventType, RequestKind,
    RequestRecord, RequestResult,
};
use crate::team_storage::TeamStore;

const CHECK_USAGE: &str = concat!(
    "usage: herdr check <create|run|show> ",
    "--db PATH --team TEAM_ID [options]"
);
const MAX_CHECK_QUESTION_BYTES: usize = 64 * 1024;
const MAX_RESULT_ERROR_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
enum CheckCommand {
    Create {
        database: PathBuf,
        team_id: String,
        actor_member_id: String,
        identity: PathBuf,
        target_member_id: String,
        repository_id: String,
        question: String,
        at: u64,
        json: bool,
    },
    Run {
        database: PathBuf,
        team_id: String,
        identity: PathBuf,
        request_id: String,
        program: PathBuf,
        args: Vec<String>,
        timeout: Duration,
        output_limit_bytes: usize,
        confirm_unisolated: bool,
        at: u64,
        json: bool,
    },
    Show {
        database: PathBuf,
        request_id: String,
        json: bool,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct CheckRequestPayload {
    repository_id: String,
    question: String,
}

#[derive(Serialize)]
struct CheckOutput<'a> {
    event_id: Option<&'a str>,
    request: Option<&'a RequestRecord>,
    policy: Option<CheckPolicyOutput>,
}

#[derive(Serialize)]
struct CheckPolicyOutput {
    isolation: &'static str,
    network: &'static str,
    confirmation_required: bool,
}

pub(crate) fn run_check_command(args: &[String]) -> std::io::Result<i32> {
    if args.is_empty()
        || matches!(
            args.first().map(String::as_str),
            Some("help" | "--help" | "-h")
        )
    {
        print_help();
        return Ok(0);
    }

    let command = match parse_check_command(args) {
        Ok(command) => command,
        Err(message) => {
            eprintln!("check: {message}");
            eprintln!("{CHECK_USAGE}");
            return Ok(2);
        }
    };

    match command {
        CheckCommand::Create {
            database,
            team_id,
            actor_member_id,
            identity: identity_path,
            target_member_id,
            repository_id,
            question,
            at,
            json,
        } => create_request(
            &database,
            &team_id,
            &actor_member_id,
            &identity_path,
            &target_member_id,
            &repository_id,
            &question,
            at,
            json,
        ),
        CheckCommand::Run {
            database,
            team_id,
            identity: identity_path,
            request_id,
            program,
            args,
            timeout,
            output_limit_bytes,
            confirm_unisolated,
            at,
            json,
        } => run_request(
            &database,
            &team_id,
            &identity_path,
            &request_id,
            &program,
            &args,
            timeout,
            output_limit_bytes,
            confirm_unisolated,
            at,
            json,
        ),
        CheckCommand::Show {
            database,
            request_id,
            json,
        } => show_request(&database, &request_id, json),
    }
}

fn create_request(
    database: &PathBuf,
    team_id: &str,
    actor_member_id: &str,
    identity_path: &PathBuf,
    target_member_id: &str,
    repository_id: &str,
    question: &str,
    at: u64,
    json_output: bool,
) -> std::io::Result<i32> {
    validate_question(question).map_err(std::io::Error::other)?;
    let identity = DeviceIdentity::load_existing(identity_path).map_err(|error| {
        std::io::Error::other(format!("could not load device identity: {error}"))
    })?;
    let mut store = TeamStore::open(database).map_err(storage_error)?;
    // Asking never grants access. The project owner explicitly consents before execution.
    let request_id = format!("check-{}", unique_suffix());
    let request_payload = CheckRequestPayload {
        repository_id: repository_id.into(),
        question: question.into(),
    };
    let payload = json!({
        "request_id": request_id,
        "requester_member_id": actor_member_id,
        "target_member_id": target_member_id,
        "kind": RequestKind::Check,
        "payload": serde_json::to_string(&request_payload).map_err(std::io::Error::other)?,
        "created_at": at,
    });
    let mut event = Event::create(
        team_id,
        actor_member_id,
        identity.device_id(),
        EventType::RequestCreated,
        payload,
        at,
        "pending-signature",
    )
    .map_err(|error| std::io::Error::other(format!("invalid check event: {error}")))?;
    event.signature = Some(identity.sign(&event.signing_bytes()));
    store.insert_event(&event, at).map_err(storage_error)?;
    outbox::queue_event(&store, &event.event_id, at).map_err(|error| {
        std::io::Error::other(format!("could not queue check request: {error}"))
    })?;
    append_lifecycle_event(
        &mut store,
        &identity,
        team_id,
        actor_member_id,
        EventType::RequestQueued,
        &request_id,
        None,
        at,
    )?;
    let request = store
        .request(&request_id)
        .map_err(storage_error)?
        .ok_or_else(|| std::io::Error::other("created check request was not projected"))?;
    print_output(Some(&event.event_id), Some(&request), None, json_output)?;
    Ok(0)
}

fn run_request(
    database: &PathBuf,
    team_id: &str,
    identity_path: &PathBuf,
    request_id: &str,
    program: &PathBuf,
    args: &[String],
    timeout: Duration,
    output_limit_bytes: usize,
    confirm_unisolated: bool,
    at: u64,
    json_output: bool,
) -> std::io::Result<i32> {
    let identity = DeviceIdentity::load_existing(identity_path).map_err(|error| {
        std::io::Error::other(format!("could not load device identity: {error}"))
    })?;
    let mut store = TeamStore::open(database).map_err(storage_error)?;
    let request = store
        .request(request_id)
        .map_err(storage_error)?
        .ok_or_else(|| std::io::Error::other(format!("check request not found: {request_id}")))?;
    if request.kind != RequestKind::Check {
        return Err(std::io::Error::other("request is not a check request"));
    }
    let profile = store
        .load_profile()
        .map_err(storage_error)?
        .ok_or_else(|| std::io::Error::other("target database has no local profile"))?;
    if profile.member.member_id != request.target_member_id {
        return Err(std::io::Error::other(
            "request target does not match the local profile",
        ));
    }
    let request_payload = decode_payload(&request)?;
    let approval = store
        .repository_approval(&request_payload.repository_id)
        .map_err(storage_error)?
        .ok_or_else(|| {
            std::io::Error::other(format!(
                "approved repository not found: {}",
                request_payload.repository_id
            ))
        })?;
    let repository_path = PathBuf::from(&approval.local_path);
    if !fs::metadata(&repository_path)
        .map_err(|error| std::io::Error::other(format!("repository is not accessible: {error}")))?
        .is_dir()
    {
        return Err(std::io::Error::other(
            "approved repository path is not a directory",
        ));
    }

    let policy = execution_boundary::assess_check_policy(&repository_path);
    ensure_delivered(
        &mut store,
        &identity,
        team_id,
        request_id,
        &request.delivery,
        at,
    )?;
    if let Err(error) = outbox::authorize_request_execution(
        &store,
        team_id,
        request_id,
        &request_payload.repository_id,
        at,
    ) {
        return record_failure(
            &mut store,
            &identity,
            team_id,
            request_id,
            &policy,
            error.to_string(),
            at,
            json_output,
        );
    }
    if let Err(error) = execution_boundary::authorize_check(&policy, confirm_unisolated) {
        return record_failure(
            &mut store,
            &identity,
            team_id,
            request_id,
            &policy,
            error.to_string(),
            at,
            json_output,
        );
    }

    append_lifecycle_event(
        &mut store,
        &identity,
        team_id,
        &profile.member.member_id,
        EventType::RequestStarted,
        request_id,
        None,
        at,
    )?;
    let agent_request = AgentRequest {
        request_id: request_id.into(),
        repository_id: request_payload.repository_id.clone(),
        program: program.clone(),
        args: args.to_vec(),
        working_directory: Some(repository_path),
        timeout,
        output_limit_bytes,
    };
    let result = match agent_runner::run_agent(agent_request, &CancellationToken::default()) {
        Ok(result) => result,
        Err(error) => {
            return record_failure(
                &mut store,
                &identity,
                team_id,
                request_id,
                &policy,
                error.to_string(),
                at,
                json_output,
            )
        }
    };
    let request_result = request_result_from_agent(&policy, result);
    let event_type = if request_result.error.is_none() && request_result.exit_code == Some(0) {
        EventType::RequestCompleted
    } else {
        EventType::RequestFailed
    };
    append_lifecycle_event(
        &mut store,
        &identity,
        team_id,
        &profile.member.member_id,
        event_type,
        request_id,
        Some(request_result),
        at,
    )?;
    let request = store
        .request(request_id)
        .map_err(storage_error)?
        .ok_or_else(|| std::io::Error::other("completed check request disappeared"))?;
    print_output(
        None,
        Some(&request),
        Some(policy_output(&policy)),
        json_output,
    )?;
    Ok(i32::from(request.delivery != DeliveryState::Completed))
}

fn show_request(database: &PathBuf, request_id: &str, json_output: bool) -> std::io::Result<i32> {
    let store = TeamStore::open_read_only(database).map_err(|error| {
        std::io::Error::other(format!("could not open check database read-only: {error}"))
    })?;
    let request = store
        .request(request_id)
        .map_err(storage_error)?
        .ok_or_else(|| std::io::Error::other(format!("check request not found: {request_id}")))?;
    print_output(None, Some(&request), None, json_output)?;
    Ok(0)
}

fn ensure_delivered(
    store: &mut TeamStore,
    identity: &DeviceIdentity,
    team_id: &str,
    request_id: &str,
    delivery: &DeliveryState,
    at: u64,
) -> std::io::Result<()> {
    let member_id = store
        .load_profile()
        .map_err(storage_error)?
        .ok_or_else(|| std::io::Error::other("target database has no local profile"))?
        .member
        .member_id;
    if matches!(delivery, DeliveryState::Created | DeliveryState::Failed) {
        append_lifecycle_event(
            store,
            identity,
            team_id,
            &member_id,
            EventType::RequestQueued,
            request_id,
            None,
            at,
        )?;
    }
    if matches!(
        delivery,
        DeliveryState::Created | DeliveryState::Queued | DeliveryState::Failed
    ) {
        append_lifecycle_event(
            store,
            identity,
            team_id,
            &member_id,
            EventType::RequestDelivered,
            request_id,
            None,
            at,
        )?;
    }
    Ok(())
}

fn record_failure(
    store: &mut TeamStore,
    identity: &DeviceIdentity,
    team_id: &str,
    request_id: &str,
    policy: &CheckExecutionPolicy,
    error: String,
    at: u64,
    json_output: bool,
) -> std::io::Result<i32> {
    let request_result = RequestResult {
        isolation: policy.isolation.label().into(),
        network: policy.network.label().into(),
        exit_code: None,
        stdout: String::new(),
        stderr: String::new(),
        output_truncated: false,
        started_at: current_unix_seconds(),
        duration_ms: 0,
        error: Some(bound_text(&error, MAX_RESULT_ERROR_BYTES)),
    };
    let profile = store
        .load_profile()
        .map_err(storage_error)?
        .ok_or_else(|| std::io::Error::other("target database has no local profile"))?;
    append_lifecycle_event(
        store,
        identity,
        team_id,
        &profile.member.member_id,
        EventType::RequestFailed,
        request_id,
        Some(request_result),
        at,
    )?;
    let request = store
        .request(request_id)
        .map_err(storage_error)?
        .ok_or_else(|| std::io::Error::other("failed check request disappeared"))?;
    print_output(
        None,
        Some(&request),
        Some(policy_output(policy)),
        json_output,
    )?;
    Ok(1)
}

fn append_lifecycle_event(
    store: &mut TeamStore,
    identity: &DeviceIdentity,
    team_id: &str,
    actor_member_id: &str,
    event_type: EventType,
    request_id: &str,
    result: Option<RequestResult>,
    at: u64,
) -> std::io::Result<String> {
    let mut event = Event::create(
        team_id,
        actor_member_id,
        identity.device_id(),
        event_type,
        json!({ "request_id": request_id, "result": result }),
        at,
        "pending-signature",
    )
    .map_err(|error| std::io::Error::other(format!("invalid request lifecycle event: {error}")))?;
    event.signature = Some(identity.sign(&event.signing_bytes()));
    let event_id = event.event_id.clone();
    store
        .insert_verified_event(&event, at)
        .map_err(storage_error)?;
    Ok(event_id)
}

fn request_result_from_agent(
    policy: &CheckExecutionPolicy,
    result: agent_runner::AgentResult,
) -> RequestResult {
    let error = match result.state {
        AgentRunState::Completed => None,
        AgentRunState::Failed => Some("agent process exited unsuccessfully".into()),
        AgentRunState::TimedOut => Some("agent process timed out".into()),
        AgentRunState::Cancelled => Some("agent process was cancelled".into()),
    };
    RequestResult {
        isolation: policy.isolation.label().into(),
        network: policy.network.label().into(),
        exit_code: result.exit_code,
        stdout: result.stdout,
        stderr: result.stderr,
        output_truncated: result.output_truncated,
        started_at: result.started_at,
        duration_ms: result.duration_ms,
        error,
    }
}

fn decode_payload(request: &RequestRecord) -> std::io::Result<CheckRequestPayload> {
    serde_json::from_str(&request.payload)
        .map_err(|error| std::io::Error::other(format!("invalid check request payload: {error}")))
}

fn policy_output(policy: &CheckExecutionPolicy) -> CheckPolicyOutput {
    CheckPolicyOutput {
        isolation: policy.isolation.label(),
        network: policy.network.label(),
        confirmation_required: policy.confirmation_required,
    }
}

fn print_output(
    event_id: Option<&str>,
    request: Option<&RequestRecord>,
    policy: Option<CheckPolicyOutput>,
    json_output: bool,
) -> std::io::Result<()> {
    if json_output {
        let output = CheckOutput {
            event_id,
            request,
            policy,
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&output).map_err(std::io::Error::other)?
        );
        return Ok(());
    }
    if let Some(event_id) = event_id {
        println!("event: {event_id}");
    }
    let Some(request) = request else {
        return Ok(());
    };
    println!("request: {}", request.request_id);
    println!("requester: {}", request.requester_member_id);
    println!("target: {}", request.target_member_id);
    println!("kind: {:?}", request.kind);
    println!("delivery: {:?}", request.delivery);
    if let Ok(payload) = decode_payload(request) {
        println!("repository: {}", payload.repository_id);
        println!("question: {}", payload.question);
    }
    if let Some(result) = &request.result {
        println!("isolation: {}", result.isolation);
        println!("network: {}", result.network);
        println!("exit code: {:?}", result.exit_code);
        println!("output truncated: {}", result.output_truncated);
        if let Some(error) = &result.error {
            println!("error: {error}");
        }
        if !result.stdout.is_empty() {
            println!("stdout:\n{}", result.stdout);
        }
        if !result.stderr.is_empty() {
            println!("stderr:\n{}", result.stderr);
        }
    }
    if let Some(policy) = policy {
        println!("execution isolation: {}", policy.isolation);
        println!("network policy: {}", policy.network);
        println!("confirmation required: {}", policy.confirmation_required);
    }
    Ok(())
}

fn parse_check_command(args: &[String]) -> Result<CheckCommand, String> {
    match args.first().map(String::as_str) {
        Some("create") => parse_create(&args[1..]),
        Some("run") => parse_run(&args[1..]),
        Some("show") => parse_show(&args[1..]),
        _ => Err("expected create, run, or show".into()),
    }
}

fn parse_create(args: &[String]) -> Result<CheckCommand, String> {
    let mut database = None;
    let mut team_id = None;
    let mut actor = None;
    let mut identity = None;
    let mut target = None;
    let mut repository = None;
    let mut question = None;
    let mut at = None;
    let mut json_output = false;
    parse_options(args, |flag, value| {
        match flag {
            "--db" => database = Some(value),
            "--team" => team_id = Some(value),
            "--actor" => actor = Some(value),
            "--identity" => identity = Some(value),
            "--target" => target = Some(value),
            "--repository" => repository = Some(value),
            "--question" => question = Some(value),
            "--at" => at = Some(parse_u64(&value, "--at")?),
            "--json" => json_output = true,
            _ => return Err(format!("unknown option: {flag}")),
        }
        Ok(())
    })?;
    Ok(CheckCommand::Create {
        database: required_path(database, "--db")?,
        team_id: required_text(team_id, "--team")?,
        actor_member_id: required_text(actor, "--actor")?,
        identity: required_path(identity, "--identity")?,
        target_member_id: required_text(target, "--target")?,
        repository_id: required_text(repository, "--repository")?,
        question: required_text(question, "--question")?,
        at: at.unwrap_or_else(current_unix_seconds),
        json: json_output,
    })
}

fn parse_run(args: &[String]) -> Result<CheckCommand, String> {
    let mut database = None;
    let mut team_id = None;
    let mut identity = None;
    let mut request_id = None;
    let mut program = None;
    let mut command_args = Vec::new();
    let mut timeout = agent_runner::DEFAULT_TIMEOUT;
    let mut output_limit_bytes = agent_runner::DEFAULT_OUTPUT_LIMIT_BYTES;
    let mut confirm_unisolated = false;
    let mut at = None;
    let mut json_output = false;
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        match flag {
            "--json" => {
                json_output = true;
                index += 1;
            }
            "--confirm-unisolated" => {
                confirm_unisolated = true;
                index += 1;
            }
            "--arg" => {
                command_args.push(value_at(args, &mut index, flag)?);
            }
            "--db" => database = Some(value_at(args, &mut index, flag)?),
            "--team" => team_id = Some(value_at(args, &mut index, flag)?),
            "--identity" => identity = Some(value_at(args, &mut index, flag)?),
            "--request" => request_id = Some(value_at(args, &mut index, flag)?),
            "--program" => program = Some(value_at(args, &mut index, flag)?),
            "--timeout-sec" => {
                let value = value_at(args, &mut index, flag)?;
                timeout = Duration::from_secs(parse_u64(&value, flag)?);
            }
            "--output-limit" => {
                let value = value_at(args, &mut index, flag)?;
                output_limit_bytes = value
                    .parse::<usize>()
                    .map_err(|_| format!("{flag} must be a positive byte count"))?;
            }
            "--at" => {
                let value = value_at(args, &mut index, flag)?;
                at = Some(parse_u64(&value, flag)?);
            }
            _ => return Err(format!("unknown option: {flag}")),
        }
    }
    Ok(CheckCommand::Run {
        database: required_path(database, "--db")?,
        team_id: required_text(team_id, "--team")?,
        identity: required_path(identity, "--identity")?,
        request_id: required_text(request_id, "--request")?,
        program: required_path(program, "--program")?,
        args: command_args,
        timeout,
        output_limit_bytes,
        confirm_unisolated,
        at: at.unwrap_or_else(current_unix_seconds),
        json: json_output,
    })
}

fn parse_show(args: &[String]) -> Result<CheckCommand, String> {
    let mut database = None;
    let mut request_id = None;
    let mut json_output = false;
    parse_options(args, |flag, value| {
        match flag {
            "--db" => database = Some(value),
            "--request" => request_id = Some(value),
            "--json" => json_output = true,
            _ => return Err(format!("unknown option: {flag}")),
        }
        Ok(())
    })?;
    Ok(CheckCommand::Show {
        database: required_path(database, "--db")?,
        request_id: required_text(request_id, "--request")?,
        json: json_output,
    })
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
        handler(flag, value_at(args, &mut index, flag)?)?;
    }
    Ok(())
}

fn value_at(args: &[String], index: &mut usize, flag: &str) -> Result<String, String> {
    let value = args
        .get(*index + 1)
        .ok_or_else(|| format!("{flag} requires a value"))?;
    *index += 2;
    Ok(value.clone())
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

fn parse_u64(value: &str, flag: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .map_err(|_| format!("{flag} must be an unsigned integer"))
}

fn validate_question(question: &str) -> Result<(), String> {
    if question.trim().is_empty() {
        return Err("question must not be empty".into());
    }
    if question.len() > MAX_CHECK_QUESTION_BYTES {
        return Err(format!("question exceeds {MAX_CHECK_QUESTION_BYTES} bytes"));
    }
    Ok(())
}

fn bound_text(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.into();
    }
    let mut end = limit;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].into()
}

fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("{nanos}-{}", std::process::id())
}

fn current_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn print_help() {
    eprintln!("herdr check commands:");
    eprintln!("  {CHECK_USAGE}");
    eprintln!("  create queues a signed question; its owner must consent before execution.");
    eprintln!("  run executes a target request; this Windows build requires --confirm-unisolated.");
    eprintln!(
        "  show reads request status and bounded output using a read-only database connection."
    );
}

fn storage_error(error: crate::team_storage::StorageError) -> std::io::Error {
    std::io::Error::other(format!("check storage error: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::team::{Device, Member, PermissionAction, PermissionResource, PermissionGrant, Role, Team};
    use std::fs;
    use std::path::Path;

    fn test_path(label: &str, extension: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "herdr-check-{label}-{}-{nanos}.{extension}",
            std::process::id()
        ))
    }

    fn setup() -> (PathBuf, PathBuf, PathBuf, String) {
        let database = test_path("team", "sqlite");
        let identity_path = test_path("identity", "json");
        let repository_path = test_path("repository", "dir");
        fs::create_dir_all(&repository_path).unwrap();
        let identity = DeviceIdentity::load_or_create(&identity_path).unwrap();
        let store = TeamStore::open(&database).unwrap();
        let member = Member {
            member_id: "member-1".into(),
            display_name: "Implementer".into(),
            role: Role::Implementer,
            active: true,
        };
        let team = Team::create("Check team", "manager-1", 1).unwrap();
        store.save_profile(&member, 1, 1).unwrap();
        store.save_team(&team).unwrap();
        store
            .save_member(
                &team.team_id,
                &Member {
                    member_id: "manager-1".into(),
                    display_name: "Manager".into(),
                    role: Role::Manager,
                    active: true,
                },
                1,
            )
            .unwrap();
        store.save_member(&team.team_id, &member, 1).unwrap();
        store
            .save_device(
                &Device {
                    device_id: identity.device_id().into(),
                    member_id: member.member_id,
                    public_key: identity.public_key(),
                    active: true,
                },
                1,
            )
            .unwrap();
        store
            .save_repository_approval(&crate::team::RepositoryApproval {
                repository_id: "repo-1".into(),
                owner_member_id: "member-1".into(),
                local_path: repository_path.to_string_lossy().into_owned(),
                approved_at: 1,
            })
            .unwrap();
        store
            .grant_permission(
                &team.team_id,
                &PermissionGrant {
                    grant_id: "grant-read".into(),
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
        (database, identity_path, repository_path, team.team_id)
    }

    fn create_args(database: &Path, identity: &Path, team_id: &str) -> Vec<String> {
        vec![
            "create".into(),
            "--db".into(),
            database.to_string_lossy().into_owned(),
            "--team".into(),
            team_id.into(),
            "--actor".into(),
            "member-1".into(),
            "--identity".into(),
            identity.to_string_lossy().into_owned(),
            "--target".into(),
            "member-1".into(),
            "--repository".into(),
            "repo-1".into(),
            "--question".into(),
            "what is the project entry point?".into(),
            "--at".into(),
            "10".into(),
            "--json".into(),
        ]
    }

    #[test]
    fn create_queues_signed_request_and_requires_execution_confirmation() {
        let (database, identity_path, repository_path, team_id) = setup();
        assert_eq!(
            run_check_command(&create_args(&database, &identity_path, &team_id)).unwrap(),
            0
        );
        let store = TeamStore::open(&database).unwrap();
        let request = store.requests().unwrap().pop().unwrap();
        assert_eq!(request.delivery, DeliveryState::Queued);
        assert!(request.result.is_none());
        assert_eq!(store.outbox().unwrap().len(), 1);
        drop(store);

        let run = vec![
            "run".into(),
            "--db".into(),
            database.to_string_lossy().into_owned(),
            "--team".into(),
            team_id.clone(),
            "--identity".into(),
            identity_path.to_string_lossy().into_owned(),
            "--request".into(),
            request.request_id.clone(),
            "--program".into(),
            std::env::current_exe()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            "--arg".into(),
            "--help".into(),
            "--at".into(),
            "11".into(),
            "--json".into(),
        ];
        assert_eq!(run_check_command(&run).unwrap(), 1);
        let store = TeamStore::open(&database).unwrap();
        let request = store.request(&request.request_id).unwrap().unwrap();
        assert_eq!(request.delivery, DeliveryState::Failed);
        assert_eq!(
            request.result.as_ref().unwrap().isolation,
            "failed to establish isolation"
        );
        drop(store);
        let retry = vec![
            "run".into(),
            "--db".into(),
            database.to_string_lossy().into_owned(),
            "--team".into(),
            team_id,
            "--identity".into(),
            identity_path.to_string_lossy().into_owned(),
            "--request".into(),
            request.request_id.clone(),
            "--program".into(),
            std::env::current_exe()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            "--arg".into(),
            "--help".into(),
            "--confirm-unisolated".into(),
            "--at".into(),
            "12".into(),
            "--json".into(),
        ];
        assert_eq!(run_check_command(&retry).unwrap(), 0);
        let store = TeamStore::open_read_only(&database).unwrap();
        assert_eq!(
            store
                .request(&request.request_id)
                .unwrap()
                .unwrap()
                .delivery,
            DeliveryState::Completed
        );
        drop(store);
        let _ = fs::remove_file(database);
        let _ = fs::remove_file(identity_path);
        let _ = fs::remove_dir_all(repository_path);
    }

    #[test]
    fn confirmed_unisolated_run_persists_bounded_result_and_status() {
        let (database, identity_path, repository_path, team_id) = setup();
        assert_eq!(
            run_check_command(&create_args(&database, &identity_path, &team_id)).unwrap(),
            0
        );
        let request_id = TeamStore::open(&database)
            .unwrap()
            .requests()
            .unwrap()
            .pop()
            .unwrap()
            .request_id;
        let run = vec![
            "run".into(),
            "--db".into(),
            database.to_string_lossy().into_owned(),
            "--team".into(),
            team_id,
            "--identity".into(),
            identity_path.to_string_lossy().into_owned(),
            "--request".into(),
            request_id.clone(),
            "--program".into(),
            std::env::current_exe()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            "--arg".into(),
            "--help".into(),
            "--confirm-unisolated".into(),
            "--at".into(),
            "11".into(),
            "--json".into(),
        ];
        assert_eq!(run_check_command(&run).unwrap(), 0);
        let store = TeamStore::open_read_only(&database).unwrap();
        let request = store.request(&request_id).unwrap().unwrap();
        assert_eq!(request.delivery, DeliveryState::Completed);
        let result = request.result.unwrap();
        assert_eq!(result.isolation, "failed to establish isolation");
        assert!(result.stdout.len() <= agent_runner::DEFAULT_OUTPUT_LIMIT_BYTES);
        let _ = fs::remove_file(database);
        let _ = fs::remove_file(identity_path);
        let _ = fs::remove_dir_all(repository_path);
    }
}
