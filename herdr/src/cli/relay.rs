use crate::identity::DeviceIdentity;
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::sync::{apply_remote_events, SyncMessage};
use crate::team::{Event, MAX_EVENT_PAYLOAD_BYTES};
use crate::team_storage::TeamStore;

const MAX_UPLOAD_EVENTS: usize = 1;
const MAX_PAGE_EVENTS: usize = 128;
const MAX_RESPONSE_BYTES: u64 = 1024 * 1024;

pub(crate) const DEFAULT_RELAY_URL: &str =
    "https://team-ai-relay.spicygames-hemant9785.workers.dev";

pub(crate) fn publish_invite_code(
    endpoint: &str,
    team_id: &str,
    token: &str,
    invite_code: &str,
) -> Result<String, String> {
    let endpoint = validate_endpoint(endpoint).map_err(str::to_owned)?;
    if team_id.is_empty()
        || !team_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err("Invalid team ID".into());
    }
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .build()
        .new_agent();
    let response = agent
        .post(format!("{endpoint}/v1/teams/{team_id}/invite-codes"))
        .header("Authorization", &format!("Bearer {token}"))
        .send_json(serde_json::json!({"invite_code": invite_code}))
        .map_err(|_| {
            "Could not create short invite. Check relay connection and version.".to_string()
        })?;
    let body: serde_json::Value = response
        .into_body()
        .with_config()
        .limit(32 * 1024)
        .read_json()
        .map_err(|_| "Invalid short invite response".to_string())?;
    let code = body["code"].as_str().ok_or("Missing invite code")?;
    if !is_short_invite_code(code) {
        return Err("Invalid short invite code".into());
    }
    Ok(code.to_owned())
}

pub(crate) fn is_short_invite_code(code: &str) -> bool {
    code.len() == 6 && code.bytes().all(|b| b.is_ascii_digit())
}

pub(crate) fn claim_invite_code(endpoint: &str, code: &str) -> Result<String, String> {
    if !is_short_invite_code(code) {
        return Err("Enter a six-digit invite code".into());
    }
    let endpoint = validate_endpoint(endpoint).map_err(str::to_owned)?;
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .build()
        .new_agent();
    let response = agent
        .get(format!("{endpoint}/v1/invites/{code}"))
        .call()
        .map_err(|error| match error {
            ureq::Error::StatusCode(404) => {
                "Invite expired, already used, or incorrect. Ask the manager for a new code."
                    .to_string()
            }
            ureq::Error::StatusCode(429) => {
                "Too many invite attempts. Try again in ten minutes.".to_string()
            }
            _ => "Could not retrieve invite. Check relay connection.".to_string(),
        })?;
    let body: serde_json::Value = response
        .into_body()
        .with_config()
        .limit(32 * 1024)
        .read_json()
        .map_err(|_| "Invalid invite response".to_string())?;
    body["invite_code"]
        .as_str()
        .map(str::to_owned)
        .ok_or("Missing signed invite".into())
}

#[derive(Serialize)]
struct Upload<'a> {
    events: Vec<Envelope<'a>>,
}

#[derive(Serialize)]
struct Envelope<'a> {
    event: &'a Event,
    public_key: String,
}

#[derive(Deserialize)]
struct UploadResponse {
    cursor: u64,
    accepted: usize,
    duplicates: usize,
}

#[derive(Deserialize)]
struct EventPage {
    events: Vec<RemoteEvent>,
    cursor: u64,
    has_more: bool,
}

#[derive(Deserialize)]
struct RemoteEvent {
    event: Event,
    sequence: u64,
}

pub(crate) fn run_relay_command(args: &[String]) -> std::io::Result<i32> {
    if args
        .first()
        .is_none_or(|arg| matches!(arg.as_str(), "help" | "--help" | "-h"))
    {
        print_help();
        return Ok(0);
    }
    if args.first().map(String::as_str) != Some("sync") {
        eprintln!("relay: expected `sync`");
        print_help();
        return Ok(2);
    }
    let mut database = None;
    let mut team_id = None;
    let mut endpoint = None;
    let mut token = None;
    let mut i = 1;
    while i < args.len() {
        let flag = args[i].as_str();
        if flag == "--help" || flag == "-h" {
            print_help();
            return Ok(0);
        }
        let Some(value) = args.get(i + 1) else {
            eprintln!("relay: missing value for {flag}");
            return Ok(2);
        };
        match flag {
            "--db" => database = Some(PathBuf::from(value)),
            "--team" => team_id = Some(value.clone()),
            "--url" => endpoint = Some(value.clone()),
            "--token" => token = Some(value.clone()),
            _ => {
                eprintln!("relay: unknown option: {flag}");
                return Ok(2);
            }
        }
        i += 2;
    }
    let database = database.unwrap_or_else(crate::team_view::default_database_path);
    let team_id = match team_id.or_else(|| {
        crate::team_view::load_from_path(&database)
            .ok()?
            .teams
            .first()
            .map(|t| t.team_id.clone())
    }) {
        Some(team) => team,
        None => {
            eprintln!("relay: create or join a team first, or provide --db and --team");
            return Ok(2);
        }
    };
    if endpoint.is_none() || token.is_none() {
        let saved = match crate::team_session::load_connection(&database) {
            Ok(saved) => saved,
            Err(error) => {
                eprintln!("relay: {error}");
                return Ok(2);
            }
        };
        if let Some(saved) = saved {
            if token.is_none()
                && endpoint.as_ref().is_some_and(|url| {
                    url.trim_end_matches('/') != saved.endpoint.trim_end_matches('/')
                })
            {
                eprintln!("relay: a different --url requires an explicit --token");
                return Ok(2);
            }
            endpoint = endpoint.or(Some(saved.endpoint));
            token = token.or(Some(saved.token));
        } else {
            endpoint = endpoint.or_else(|| std::env::var("HERDR_RELAY_URL").ok());
            token = token.or_else(|| std::env::var("HERDR_RELAY_TOKEN").ok());
        }
    }
    let (Some(endpoint), Some(token)) = (endpoint, token) else {
        eprintln!("relay: reconnect using an invite code, or provide --url and --token");
        return Ok(2);
    };
    if token.len() != 43
        || !token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        eprintln!("relay: token must be a 32-byte base64url bearer token");
        return Ok(2);
    }
    let endpoint = match validate_endpoint(&endpoint) {
        Ok(value) => value,
        Err(reason) => {
            eprintln!("relay: {reason}");
            return Ok(2);
        }
    };
    match sync_team(&database, &team_id, &endpoint, &token) {
        Ok((uploaded, downloaded)) => {
            println!(
                "relay sync complete: uploaded {uploaded} event(s), applied {downloaded} event(s)"
            );
            Ok(0)
        }
        Err(error) => {
            eprintln!("relay: {error}");
            Ok(1)
        }
    }
}

pub(crate) fn sync_team(
    database: &std::path::Path,
    team_id: &str,
    endpoint: &str,
    token: &str,
) -> Result<(usize, usize), String> {
    if team_id.is_empty()
        || team_id.len() > 256
        || !team_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err("team ID contains unsupported characters".into());
    }
    if token.len() != 43
        || !token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err("relay token must be a 32-byte base64url bearer token".into());
    }
    let endpoint = validate_endpoint(endpoint).map_err(str::to_owned)?;
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .http_status_as_error(false)
        .build()
        .new_agent();
    let identity = DeviceIdentity::load_existing(crate::cli::join_inbox::identity_path(database))
        .map_err(|error| format!("relay device identity unavailable: {error}"))?;
    crate::cli::questions::publish_local_projects(
        database,
        &crate::cli::join_inbox::identity_path(database),
    )?;
    let mut store = TeamStore::open(database).map_err(|error| error.to_string())?;
    let mut uploaded = 0;
    let pending = store
        .team_events(team_id, 20_000)
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter_map(
            |event| match store.relay_knows_event(team_id, &endpoint, &event.event_id) {
                Ok(false) => Some(Ok(event)),
                Ok(true) => None,
                Err(error) => Some(Err(error.to_string())),
            },
        )
        .collect::<Result<Vec<_>, _>>()?;
    for events in pending.chunks(MAX_UPLOAD_EVENTS) {
        let mut envelopes = Vec::with_capacity(events.len());
        for event in events {
            let public_key = store
                .event_device_public_key(&event.device_id)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| format!("no public key for event device {}", event.device_id))?;
            envelopes.push(Envelope { event, public_key });
        }
        if envelopes.is_empty() {
            continue;
        }
        let url = format!("{endpoint}/v1/teams/{team_id}/events");
        let upload = Upload { events: envelopes };
        let result = post_upload(&agent, &url, token, &upload, team_id, &identity)?;
        let _ = result.cursor;
        if result.accepted.checked_add(result.duplicates) != Some(events.len()) {
            return Err("relay did not acknowledge every uploaded event".into());
        }
        for event in events {
            store
                .remember_relay_event(team_id, &endpoint, &event.event_id)
                .map_err(|error| error.to_string())?;
        }
        uploaded += events.len();
    }

    let mut applied = 0;
    let mut cursor = store
        .relay_cursor(team_id)
        .map_err(|error| error.to_string())?;
    loop {
        let url =
            format!("{endpoint}/v1/teams/{team_id}/events?after={cursor}&limit={MAX_PAGE_EVENTS}");
        let previous_cursor = cursor;
        let mut response = get_page(&agent, &url, token, team_id, &identity)?;
        let page = response
            .body_mut()
            .with_config()
            .limit(MAX_RESPONSE_BYTES)
            .read_json::<EventPage>()
            .map_err(|_| "relay returned an invalid or oversized event page".to_string())?;
        if page.events.len() > MAX_PAGE_EVENTS || page.cursor < cursor {
            return Err("relay returned an invalid page cursor".into());
        }
        let mut events = Vec::with_capacity(page.events.len());
        let mut next_cursor = cursor;
        for remote in page.events {
            if remote.event.team_id != team_id || remote.sequence <= next_cursor {
                return Err("relay returned an out-of-order or wrong-team event".into());
            }
            if serde_json::to_vec(&remote.event.payload)
                .map_or(true, |payload| payload.len() > MAX_EVENT_PAYLOAD_BYTES)
            {
                return Err("relay event payload exceeds the local limit".into());
            }
            next_cursor = remote.sequence;
            events.push(remote.event);
        }
        if !events.is_empty() {
            let received_ids = events
                .iter()
                .map(|event| event.event_id.clone())
                .collect::<Vec<_>>();
            let message = SyncMessage::Events {
                team_id: team_id.to_string(),
                events,
            };
            let report = apply_remote_events(&mut store, team_id, &message, now())
                .map_err(|error| format!("event verification/apply failed: {error}"))?;
            for event_id in received_ids {
                store
                    .remember_relay_event(team_id, &endpoint, &event_id)
                    .map_err(|error| error.to_string())?;
            }
            applied += report.inserted;
            cursor = next_cursor;
            store
                .save_relay_cursor(team_id, cursor, now())
                .map_err(|error| error.to_string())?;
        } else if page.cursor > cursor {
            return Err("relay page advanced without returning events".into());
        }
        if !page.has_more {
            break;
        }
        if events_is_empty_at_cursor(previous_cursor, next_cursor) {
            return Err("relay pagination made no progress".into());
        }
    }
    let (direct_uploaded, direct_applied) =
        sync_direct_messages(&agent, &store, team_id, &endpoint, token, &identity)?;
    Ok((uploaded + direct_uploaded, applied + direct_applied))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DirectPage {
    messages: Vec<DirectRemote>,
    cursor: u64,
    has_more: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DirectRemote {
    sequence: u64,
    message: crate::crewai_message::DirectMessage,
}

fn sync_direct_messages(
    agent: &ureq::Agent,
    store: &TeamStore,
    team_id: &str,
    endpoint: &str,
    token: &str,
    identity: &DeviceIdentity,
) -> Result<(usize, usize), String> {
    let target = format!("{endpoint}/v1/teams/{team_id}/direct-messages");
    let mut uploaded = 0;
    for message in store
        .queued_direct_messages(team_id)
        .map_err(|e| e.to_string())?
    {
        let body = serde_json::to_vec(&message).map_err(|e| e.to_string())?;
        let (timestamp, nonce, signature) =
            access_proof(identity, team_id, "POST", &target, &body)?;
        let response = agent
            .post(&target)
            .header("Authorization", &format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .header("x-relay-device", identity.device_id())
            .header("x-relay-time", &timestamp.to_string())
            .header("x-relay-nonce", &nonce)
            .header("x-relay-signature", &signature)
            .send(body.as_slice())
            .map_err(|_| "Could not send direct message; it remains queued".to_string())?;
        if !response.status().is_success() {
            return Err(relay_rejection(response, "direct message upload"));
        }
        store
            .mark_direct_synced(team_id, &message.message_id)
            .map_err(|e| e.to_string())?;
        uploaded += 1;
    }
    let mut cursor = store.direct_cursor(team_id).map_err(|e| e.to_string())?;
    let mut applied = 0;
    loop {
        let url = format!("{target}?after={cursor}&limit=16");
        let response = match get_page(agent, &url, token, team_id, identity) {
            Ok(response) => response,
            // Older relays have no private mailbox. Existing team sync must still work;
            // queued direct messages are never marked delivered by this fallback.
            Err(error)
                if uploaded == 0 && error.starts_with("relay rejected download with HTTP 404") =>
            {
                return Ok((0, 0))
            }
            Err(error) => return Err(error),
        };
        let page = response
            .into_body()
            .with_config()
            .limit(1024 * 1024)
            .read_json::<DirectPage>()
            .map_err(|_| "Invalid direct-message response".to_string())?;
        if page.messages.len() > 16 || page.cursor < cursor {
            return Err("Invalid direct-message cursor".into());
        }
        let mut next = cursor;
        for entry in page.messages {
            if entry.sequence <= next || entry.message.team_id != team_id {
                return Err("Invalid direct-message order or team".into());
            }
            if store
                .insert_direct_message(&entry.message)
                .map_err(|e| format!("Direct message rejected: {e}"))?
            {
                applied += 1;
            }
            next = entry.sequence;
        }
        if page.cursor != next || (page.has_more && next == cursor) {
            return Err("Invalid direct-message page".into());
        }
        if next > cursor {
            store
                .advance_direct_cursor(team_id, next)
                .map_err(|e| e.to_string())?;
            cursor = next;
        }
        if !page.has_more {
            break;
        }
    }
    Ok((uploaded, applied))
}

fn post_upload(
    agent: &ureq::Agent,
    url: &str,
    token: &str,
    upload: &Upload<'_>,
    team_id: &str,
    identity: &DeviceIdentity,
) -> Result<UploadResponse, String> {
    let body = serde_json::to_vec(upload).map_err(|error| error.to_string())?;
    for attempt in 0..3 {
        let (timestamp, nonce, signature) = access_proof(identity, team_id, "POST", url, &body)?;
        match agent
            .post(url)
            .header("Authorization", &format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .header("x-relay-device", identity.device_id())
            .header("x-relay-time", &timestamp.to_string())
            .header("x-relay-nonce", &nonce)
            .header("x-relay-signature", &signature)
            .send(body.as_slice())
        {
            Ok(response) => {
                if !response.status().is_success() {
                    return Err(relay_rejection(response, "upload"));
                }
                return response
                    .into_body()
                    .with_config()
                    .limit(64 * 1024)
                    .read_json::<UploadResponse>()
                    .map_err(|_| "relay returned an invalid upload response".to_string());
            }
            Err(ureq::Error::StatusCode(code)) => {
                return Err(format!("relay rejected upload with HTTP {code}"))
            }
            Err(_) if attempt < 2 => {
                std::thread::sleep(Duration::from_millis(250 * (1 << attempt)))
            }
            Err(_) => {
                return Err(
                    "relay upload failed after retries (check URL, token, and network)".into(),
                )
            }
        }
    }
    Err("relay upload failed".into())
}

fn get_page(
    agent: &ureq::Agent,
    url: &str,
    token: &str,
    team_id: &str,
    identity: &DeviceIdentity,
) -> Result<ureq::http::Response<ureq::Body>, String> {
    for attempt in 0..3 {
        let (timestamp, nonce, signature) = access_proof(identity, team_id, "GET", url, &[])?;
        match agent
            .get(url)
            .header("Authorization", &format!("Bearer {token}"))
            .header("x-relay-device", identity.device_id())
            .header("x-relay-time", &timestamp.to_string())
            .header("x-relay-nonce", &nonce)
            .header("x-relay-signature", &signature)
            .call()
        {
            Ok(response) => {
                if !response.status().is_success() {
                    return Err(relay_rejection(response, "download"));
                }
                return Ok(response);
            }
            Err(ureq::Error::StatusCode(code)) => {
                return Err(format!("relay rejected download with HTTP {code}"))
            }
            Err(_) if attempt < 2 => {
                std::thread::sleep(Duration::from_millis(250 * (1 << attempt)))
            }
            Err(_) => {
                return Err(
                    "relay download failed after retries (check URL, token, and network)".into(),
                )
            }
        }
    }
    Err("relay download failed".into())
}

pub(crate) fn auto_sync_after_write(
    database: &std::path::Path,
    team_id: &str,
) -> Result<bool, String> {
    let Some(connection) = crate::team_session::connection_from_env_or_store(database)? else {
        return Ok(false);
    };
    sync_team(database, team_id, &connection.endpoint, &connection.token)?;
    Ok(true)
}

fn relay_rejection(response: ureq::http::Response<ureq::Body>, operation: &str) -> String {
    let status = response.status().as_u16();
    let code = response
        .into_body()
        .with_config()
        .limit(4096)
        .read_json::<serde_json::Value>()
        .ok()
        .and_then(|v| v.get("error").and_then(|s| s.as_str()).map(str::to_owned));
    rejection_message(status, operation, code.as_deref())
}
fn rejection_message(status: u16, operation: &str, code: Option<&str>) -> String {
    let hint = match code {
        Some("invalid_team_token" | "authorization_required") => {
            "The connection does not match this team. Reconnect with a fresh manager invite."
        }
        Some("invalid_signature" | "device_identity_mismatch" | "invalid_access_signature") => {
            "The signed device or event could not be verified."
        }
        Some("device_not_approved" | "event_device_not_approved") => {
            "Waiting for the manager to approve this device and sync membership."
        }
        Some("access_replay" | "stale_access_proof") => {
            "Retry sync and check that this computer's clock is correct."
        }
        _ => "Check your connection and retry sync.",
    };
    format!("relay rejected {operation} with HTTP {status}. {hint}")
}

fn access_proof(
    identity: &DeviceIdentity,
    team_id: &str,
    method: &str,
    url: &str,
    body: &[u8],
) -> Result<(u64, String, String), String> {
    let uri: ureq::http::Uri = url.parse().map_err(|_| "invalid relay URL".to_string())?;
    let target = uri
        .path_and_query()
        .map(|value| value.as_str())
        .unwrap_or("/");
    let timestamp = now();
    let mut random = [0_u8; 24];
    OsRng.fill_bytes(&mut random);
    let nonce = random
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let digest = Sha256::digest(body)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let message = serde_json::to_vec(&(
        "team-assistant/relay-access/v1",
        team_id,
        identity.device_id(),
        method,
        target,
        digest,
        timestamp,
        &nonce,
    ))
    .map_err(|error| error.to_string())?;
    let signature = identity.sign(&message);
    Ok((timestamp, nonce, signature))
}

fn events_is_empty_at_cursor(current: u64, previous: u64) -> bool {
    current == previous
}

pub(crate) fn validate_endpoint(endpoint: &str) -> Result<String, &'static str> {
    let value = endpoint.trim().trim_end_matches('/');
    let local_http = ["http://localhost", "http://127.0.0.1", "http://[::1]"]
        .iter()
        .any(|prefix| {
            value == *prefix
                || value.starts_with(&format!("{prefix}:"))
                || value.starts_with(&format!("{prefix}/"))
        });
    if !(value.starts_with("https://") || local_http)
        || value.len() > 2048
        || value.contains(['?', '#'])
    {
        return Err("URL must be HTTPS (HTTP is allowed only for localhost development)");
    }
    Ok(value.to_string())
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn print_help() {
    println!("usage: herdr relay sync [--db PATH] [--team TEAM_ID] [--url HTTPS_URL --token TOKEN]\n\nUses the active team's protected saved connection; environment settings bootstrap unsaved connections. Explicit URL/token flags override it. Synchronizes bounded signed team records.");
}

#[cfg(test)]
mod connection_error_tests {
    use super::*;
    #[test]
    fn token_signature_and_approval_errors_have_distinct_safe_guidance() {
        assert!(rejection_message(401, "upload", Some("invalid_team_token"))
            .contains("fresh manager invite"));
        assert!(rejection_message(401, "upload", Some("invalid_signature"))
            .contains("could not be verified"));
        assert!(
            rejection_message(403, "download", Some("device_not_approved"))
                .contains("approve this device")
        );
        assert!(
            !rejection_message(401, "upload", Some("untrusted-secret-response"))
                .contains("untrusted-secret-response")
        );
    }
}
