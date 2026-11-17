use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::sync::{apply_remote_events, SyncMessage};
use crate::team::{Event, MAX_EVENT_PAYLOAD_BYTES};
use crate::team_storage::TeamStore;

const MAX_UPLOAD_EVENTS: usize = 1;
const MAX_PAGE_EVENTS: usize = 128;
const MAX_RESPONSE_BYTES: u64 = 1024 * 1024;

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
    let mut token = std::env::var("HERDR_RELAY_TOKEN").ok();
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
    let (Some(database), Some(team_id), Some(endpoint), Some(token)) =
        (database, team_id, endpoint, token)
    else {
        eprintln!("relay: --db, --team, --url, and --token (or HERDR_RELAY_TOKEN) are required");
        print_help();
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

pub(super) fn sync_team(
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
        .build()
        .new_agent();
    let mut store = TeamStore::open(database).map_err(|error| error.to_string())?;
    let mut uploaded = 0;
    for events in store
        .team_events(team_id, 20_000)
        .map_err(|error| error.to_string())?
        .chunks(MAX_UPLOAD_EVENTS)
    {
        let mut envelopes = Vec::with_capacity(events.len());
        for event in events {
            let public_key = store
                .device_public_key(&event.device_id)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| {
                    format!("no active public key for event device {}", event.device_id)
                })?;
            envelopes.push(Envelope { event, public_key });
        }
        if envelopes.is_empty() {
            continue;
        }
        let url = format!("{endpoint}/v1/teams/{team_id}/events");
        let upload = Upload { events: envelopes };
        let result = post_upload(&agent, &url, token, &upload)?;
        let _ = result.cursor;
        uploaded += events.len();
    }

    let mut applied = 0;
    let mut cursor = store
        .relay_cursor(team_id)
        .map_err(|error| error.to_string())?;
    loop {
        let url =
            format!("{endpoint}/v1/teams/{team_id}/events?after={cursor}&limit={MAX_PAGE_EVENTS}");
        let mut response = get_page(&agent, &url, token)?;
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
            let message = SyncMessage::Events {
                team_id: team_id.to_string(),
                events,
            };
            let report = apply_remote_events(&mut store, team_id, &message, now())
                .map_err(|error| format!("event verification/apply failed: {error}"))?;
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
        if events_is_empty_at_cursor(cursor, next_cursor) {
            return Err("relay pagination made no progress".into());
        }
    }
    Ok((uploaded, applied))
}

fn post_upload(
    agent: &ureq::Agent,
    url: &str,
    token: &str,
    upload: &Upload<'_>,
) -> Result<UploadResponse, String> {
    for attempt in 0..3 {
        match agent
            .post(url)
            .header("Authorization", &format!("Bearer {token}"))
            .send_json(upload)
        {
            Ok(response) => {
                return response
                    .into_body()
                    .with_config()
                    .limit(64 * 1024)
                    .read_json::<UploadResponse>()
                    .map_err(|_| "relay returned an invalid upload response".to_string())
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
) -> Result<ureq::http::Response<ureq::Body>, String> {
    for attempt in 0..3 {
        match agent
            .get(url)
            .header("Authorization", &format!("Bearer {token}"))
            .call()
        {
            Ok(response) => return Ok(response),
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

pub(super) fn auto_sync_after_write(
    database: &std::path::Path,
    team_id: &str,
) -> Result<bool, String> {
    let (Ok(endpoint), Ok(token)) = (
        std::env::var("HERDR_RELAY_URL"),
        std::env::var("HERDR_RELAY_TOKEN"),
    ) else {
        return Ok(false);
    };
    let endpoint = validate_endpoint(&endpoint).map_err(str::to_owned)?;
    sync_team(database, team_id, &endpoint, &token)?;
    Ok(true)
}

fn events_is_empty_at_cursor(current: u64, previous: u64) -> bool {
    current == previous
}

pub(super) fn validate_endpoint(endpoint: &str) -> Result<String, &'static str> {
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
    println!("usage: herdr relay sync --db PATH --team TEAM_ID --url HTTPS_URL --token TOKEN\n       herdr relay sync --db PATH --team TEAM_ID --url HTTPS_URL (uses HERDR_RELAY_TOKEN)\n\nSynchronizes bounded signed team events. Repository files and paths are never sent. The token is not stored locally.");
}
