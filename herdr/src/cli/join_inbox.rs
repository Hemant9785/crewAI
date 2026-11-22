//! Signed pairing mailbox; team membership is still approved through team commands.
use super::team::JoinRequest;
use crate::identity::DeviceIdentity;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct Entry {
    pub request: JoinRequest,
    pub status: String,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct Inbox {
    #[serde(default)]
    pub requests: Vec<Entry>,
    pub status: Option<String>,
}

fn post(
    endpoint: &str,
    team_id: &str,
    token: &str,
    body: serde_json::Value,
) -> Result<Inbox, String> {
    let endpoint = super::relay::validate_endpoint(endpoint).map_err(str::to_owned)?;
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
        .post(format!("{endpoint}/v1/teams/{team_id}/join-requests"))
        .header("Authorization", &format!("Bearer {token}"))
        .send_json(body)
        .map_err(|error| match error {
            ureq::Error::StatusCode(code) => format!("Join inbox rejected request (HTTP {code})"),
            _ => "Join inbox unavailable. Check connection and retry with a fresh invite.".into(),
        })?;
    response
        .into_body()
        .with_config()
        .limit(1024 * 1024)
        .read_json::<Inbox>()
        .map_err(|_| "Invalid join inbox response".into())
}

pub(crate) fn submit(endpoint: &str, token: &str, request: &JoinRequest) -> Result<Inbox, String> {
    post(
        endpoint,
        &request.team_id,
        token,
        serde_json::json!({"action":"submit", "request":request}),
    )
}

pub(crate) fn decide(
    database: &Path,
    identity: &Path,
    endpoint: &str,
    token: &str,
    request: &JoinRequest,
    approve: bool,
) -> Result<(), String> {
    let current = command(endpoint, &request.team_id, token, identity, "list", "", "")?;
    if !current.requests.iter().any(|entry| {
        entry.request.signature == request.signature
            && entry.request.device.device_id == request.device.device_id
    }) {
        return Err("This join request is no longer pending. Refresh Requests.".into());
    }
    if approve {
        super::team::approve_join_request(
            database,
            &request.team_id,
            identity,
            request,
            Some((endpoint, token)),
        )
        .map_err(|e| e.to_string())?;
    }
    command(
        endpoint,
        &request.team_id,
        token,
        identity,
        if approve { "approved" } else { "rejected" },
        &request.device.device_id,
        &request.signature,
    )?;
    Ok(())
}

pub(crate) fn run(args: &[String]) -> std::io::Result<i32> {
    let get = |key: &str| {
        args.windows(2)
            .find(|pair| pair[0] == key)
            .map(|pair| pair[1].clone())
            .ok_or_else(|| std::io::Error::other(format!("Missing {key}")))
    };
    let db = std::path::PathBuf::from(get("--db")?);
    let identity = std::path::PathBuf::from(get("--identity")?);
    let team = get("--team")?;
    let action = get("--action").unwrap_or_else(|_| "list".into());
    let url = std::env::var("HERDR_RELAY_URL").map_err(std::io::Error::other)?;
    let token = std::env::var("HERDR_RELAY_TOKEN").map_err(std::io::Error::other)?;
    if action == "status" {
        let key = DeviceIdentity::load_existing(&identity).map_err(std::io::Error::other)?;
        let inbox = command(
            &url,
            &team,
            &token,
            &identity,
            "status",
            key.device_id(),
            "",
        )
        .map_err(std::io::Error::other)?;
        println!("{}", inbox.status.unwrap_or_default());
        return Ok(0);
    }
    if action == "submit" {
        let request: JoinRequest = serde_json::from_slice(&std::fs::read(get("--request")?)?)
            .map_err(std::io::Error::other)?;
        if request.team_id != team {
            return Err(std::io::Error::other("Request belongs to another team"));
        }
        let inbox = submit(&url, &token, &request).map_err(std::io::Error::other)?;
        println!("{}", inbox.status.unwrap_or_default());
        return Ok(0);
    }
    let inbox =
        command(&url, &team, &token, &identity, "list", "", "").map_err(std::io::Error::other)?;
    if action == "list" {
        println!(
            "{}",
            serde_json::to_string(&inbox.requests).map_err(std::io::Error::other)?
        );
    } else if action == "approve" || action == "reject" {
        let request_id = get("--request-id")?;
        let entry = inbox
            .requests
            .iter()
            .find(|entry| entry.request.device.device_id == request_id)
            .ok_or_else(|| std::io::Error::other("Pending join request not found"))?;
        decide(
            &db,
            &identity,
            &url,
            &token,
            &entry.request,
            action == "approve",
        )
        .map_err(std::io::Error::other)?;
        println!(
            "{}",
            if action == "approve" {
                "Approved"
            } else {
                "Rejected"
            }
        );
    } else {
        return Err(std::io::Error::other("Expected list, approve, or reject"));
    }
    Ok(0)
}

pub(crate) fn command(
    endpoint: &str,
    team: &str,
    token: &str,
    identity_path: &Path,
    action: &str,
    request_id: &str,
    request_signature: &str,
) -> Result<Inbox, String> {
    let identity = DeviceIdentity::load_existing(identity_path).map_err(|e| e.to_string())?;
    let created_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let bytes = serde_json::to_vec(&(
        "team-assistant/join-inbox/v1",
        team,
        action,
        request_id,
        created_at,
        request_signature,
    ))
    .map_err(|e| e.to_string())?;
    post(
        endpoint,
        team,
        token,
        serde_json::json!({
            "action":action, "request_id":request_id, "created_at":created_at, "signature":identity.sign(&bytes), "request_signature": request_signature,
        }),
    )
}

pub(crate) fn identity_path(database: &Path) -> std::path::PathBuf {
    database.with_file_name(format!(
        "{}.identity.json",
        database
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("team")
    ))
}

pub(crate) fn reconnect_request(
    database: &Path,
    identity_path: &Path,
    bundle: &super::team::InviteBundle,
) -> Result<Option<JoinRequest>, String> {
    let identity = DeviceIdentity::load_existing(identity_path).map_err(|e| e.to_string())?;
    let store = crate::team_storage::TeamStore::open(database).map_err(|e| e.to_string())?;
    if store
        .team_events(&bundle.invitation.team_id, 20_000)
        .map_err(|e| e.to_string())?
        .iter()
        .any(|event| {
            event.event_type == crate::team::EventType::DeviceAdded
                && event.payload["device_id"].as_str() == Some(identity.device_id())
        })
    {
        return Ok(None);
    }
    let profile = store
        .load_profile()
        .map_err(|e| e.to_string())?
        .ok_or("Set up your profile first")?;
    if profile.member.member_id == bundle.invitation.authority_member_id {
        return Ok(None);
    }
    let mut request = JoinRequest {
        invitation_id: bundle.invitation.invitation_id.clone(),
        team_id: bundle.invitation.team_id.clone(),
        device: crate::team::Device {
            device_id: identity.device_id().into(),
            member_id: profile.member.member_id.clone(),
            public_key: identity.public_key(),
            active: true,
        },
        member: profile.member,
        created_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        signature: String::new(),
    };
    request.signature = identity.sign(&request.signing_bytes());
    Ok(Some(request))
}
