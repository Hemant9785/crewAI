use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::identity::DeviceIdentity;
use crate::team::{Device, Event, EventType, Member, Role, Team, TeamInvitation};
use crate::team_storage::TeamStore;

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct InviteBundle {
    pub(crate) invitation: TeamInvitation,
    pub(crate) pairing_code: String,
    pub(crate) relay_url: String,
    pub(crate) relay_token: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct JoinRequest {
    pub(crate) invitation_id: String,
    pub(crate) team_id: String,
    pub(crate) member: Member,
    pub(crate) device: Device,
    pub(crate) created_at: u64,
    pub(crate) signature: String,
}

impl JoinRequest {
    pub(crate) fn signing_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(&(
            "team-assistant/join-request/v1",
            &self.invitation_id,
            &self.team_id,
            &self.member,
            &self.device,
            self.created_at,
        ))
        .expect("join request signing fields serialize")
    }
}

pub(crate) fn run_team_command(args: &[String]) -> std::io::Result<i32> {
    match args.first().map(String::as_str) {
        Some("create") => create_team(&args[1..]),
        Some("invite") => create_invite(&args[1..]),
        Some("join") => accept_invite(&args[1..]),
        Some("approve") => approve_member(&args[1..]),
        Some("requests") => super::join_inbox::run(&args[1..]),
        Some("help" | "--help" | "-h") | None => {
            print_help();
            Ok(0)
        }
        Some(command) => {
            eprintln!("team: unknown subcommand {command}");
            print_help();
            Ok(2)
        }
    }
}

const INVITE_CODE_PREFIX: &str = "HERDR-INVITE-1:";
const JOIN_CODE_PREFIX: &str = "HERDR-JOIN-1:";
const MAX_PAIRING_CODE_BYTES: usize = 16 * 1024;

pub(crate) fn create_invite_bundle(
    database: &Path,
    team_id: &str,
    identity_path: &Path,
) -> std::io::Result<InviteBundle> {
    let endpoint = std::env::var("HERDR_RELAY_URL")
        .map_err(|_| io_error("set HERDR_RELAY_URL before inviting a teammate"))?;
    let token = std::env::var("HERDR_RELAY_TOKEN")
        .map_err(|_| io_error("set HERDR_RELAY_TOKEN before inviting a teammate"))?;
    create_invite_bundle_with_credentials(database, team_id, identity_path, &endpoint, &token)
}

fn create_invite_bundle_with_credentials(
    database: &Path,
    team_id: &str,
    identity_path: &Path,
    endpoint: &str,
    token: &str,
) -> std::io::Result<InviteBundle> {
    let endpoint = super::relay::validate_endpoint(endpoint).map_err(io_error)?;
    validate_relay_token(token)?;
    let identity = DeviceIdentity::load_existing(identity_path).map_err(io_error)?;
    let store = TeamStore::open(database).map_err(io_error)?;
    let profile = store
        .load_profile()
        .map_err(io_error)?
        .ok_or_else(|| io_error("local profile is missing"))?;
    let team = store
        .team(team_id)
        .map_err(io_error)?
        .ok_or_else(|| io_error("team is not configured locally"))?;
    if team.authority_member_id != profile.member.member_id {
        return Err(io_error("only the team authority can create invitations"));
    }
    let device = Device {
        device_id: identity.device_id().into(),
        member_id: profile.member.member_id.clone(),
        public_key: identity.public_key(),
        active: true,
    };
    if store
        .device_public_key(&device.device_id)
        .map_err(io_error)?
        .as_deref()
        != Some(device.public_key.as_str())
    {
        return Err(io_error(
            "authority device is not approved in the team database",
        ));
    }
    let pairing_code = generate_token();
    let mut invitation = TeamInvitation::new_unsigned(
        &team,
        &device,
        &pairing_code,
        now(),
        now() + 7 * 24 * 60 * 60,
    )
    .map_err(io_error)?;
    let signature = identity.sign(&invitation.signing_bytes());
    invitation = invitation.with_signature(signature).map_err(io_error)?;
    store
        .save_invitation(&invitation, now())
        .map_err(io_error)?;
    Ok(InviteBundle {
        invitation,
        pairing_code,
        relay_url: endpoint,
        relay_token: token.to_owned(),
    })
}

pub(crate) fn encode_invite_code(bundle: &InviteBundle) -> std::io::Result<String> {
    let bytes = serde_json::to_vec(bundle).map_err(io_error)?;
    Ok(format!(
        "{INVITE_CODE_PREFIX}{}",
        URL_SAFE_NO_PAD.encode(bytes)
    ))
}

pub(crate) fn decode_invite_code(code: &str) -> std::io::Result<InviteBundle> {
    let encoded = code
        .trim()
        .strip_prefix(INVITE_CODE_PREFIX)
        .ok_or_else(|| io_error("invite code must start with HERDR-INVITE-1:"))?;
    decode_code_payload(encoded)
}

pub(crate) fn encode_join_code(request: &JoinRequest) -> std::io::Result<String> {
    let bytes = serde_json::to_vec(request).map_err(io_error)?;
    Ok(format!("{JOIN_CODE_PREFIX}{}", URL_SAFE_NO_PAD.encode(bytes)))
}

pub(crate) fn decode_join_code(code: &str) -> std::io::Result<JoinRequest> {
    let encoded = code
        .trim()
        .strip_prefix(JOIN_CODE_PREFIX)
        .ok_or_else(|| io_error("join code must start with HERDR-JOIN-1:"))?;
    decode_code_payload(encoded)
}

fn decode_code_payload<T: for<'de> Deserialize<'de>>(encoded: &str) -> std::io::Result<T> {
    if encoded.len() > MAX_PAIRING_CODE_BYTES * 2 {
        return Err(io_error("pairing code is too large"));
    }
    let bytes = URL_SAFE_NO_PAD.decode(encoded).map_err(io_error)?;
    if bytes.len() > MAX_PAIRING_CODE_BYTES {
        return Err(io_error("pairing code is too large"));
    }
    serde_json::from_slice(&bytes).map_err(io_error)
}

pub(crate) fn validate_invite_for_existing_team(
    bundle: &InviteBundle,
    team_id: &str,
    authority_member_id: &str,
) -> std::io::Result<()> {
    let invitation = &bundle.invitation;
    invitation.validate().map_err(io_error)?;
    if invitation.is_expired(now()) {
        return Err(io_error("invitation has expired"));
    }
    if invitation.team_id != team_id || invitation.authority_member_id != authority_member_id {
        return Err(io_error("invitation belongs to a different team"));
    }
    if DeviceIdentity::device_id_for_public_key(&invitation.authority_public_key)
        .map_err(io_error)?
        != invitation.authority_device_id
    {
        return Err(io_error("invitation authority device ID is invalid"));
    }
    DeviceIdentity::verify(
        &invitation.authority_public_key,
        &invitation.signing_bytes(),
        &invitation.signature,
    )
    .map_err(io_error)?;
    super::relay::validate_endpoint(&bundle.relay_url).map_err(io_error)?;
    validate_relay_token(&bundle.relay_token)
}

pub(crate) fn accept_invite_bundle(
    database: &Path,
    identity_path: &Path,
    bundle: &InviteBundle,
) -> std::io::Result<JoinRequest> {
    super::relay::validate_endpoint(&bundle.relay_url).map_err(io_error)?;
    validate_relay_token(&bundle.relay_token)?;
    let identity = DeviceIdentity::load_existing(identity_path).map_err(io_error)?;
    let mut store = TeamStore::open(database).map_err(io_error)?;
    let profile = store
        .load_profile()
        .map_err(io_error)?
        .ok_or_else(|| io_error("set up this device's profile first"))?;
    let joining_device = Device {
        device_id: identity.device_id().into(),
        member_id: profile.member.member_id.clone(),
        public_key: identity.public_key(),
        active: true,
    };
    let authority_member = Member {
        member_id: bundle.invitation.authority_member_id.clone(),
        display_name: bundle.invitation.team_name.clone(),
        role: Role::Manager,
        active: true,
    };
    let authority_device = Device {
        device_id: bundle.invitation.authority_device_id.clone(),
        member_id: bundle.invitation.authority_member_id.clone(),
        public_key: bundle.invitation.authority_public_key.clone(),
        active: true,
    };
    store
        .accept_invitation(
            &bundle.invitation,
            &bundle.pairing_code,
            &authority_member,
            &authority_device,
            &profile.member,
            &joining_device,
            &identity,
            now(),
        )
        .map_err(io_error)?;
    let mut request = JoinRequest {
        invitation_id: bundle.invitation.invitation_id.clone(),
        team_id: bundle.invitation.team_id.clone(),
        member: profile.member,
        device: joining_device,
        created_at: now(),
        signature: String::new(),
    };
    request.signature = identity.sign(&request.signing_bytes());
    Ok(request)
}

pub(crate) fn approve_join_request(
    database: &Path,
    team_id: &str,
    identity_path: &Path,
    request: &JoinRequest,
    relay_credentials: Option<(&str, &str)>,
) -> std::io::Result<()> {
    if request.team_id != team_id
        || !request.member.active
        || !request.device.active
        || request.device.member_id != request.member.member_id
    {
        return Err(io_error("join request team/member/device fields are inconsistent"));
    }
    if DeviceIdentity::device_id_for_public_key(&request.device.public_key).map_err(io_error)?
        != request.device.device_id
    {
        return Err(io_error("join request device ID does not match its public key"));
    }
    DeviceIdentity::verify(
        &request.device.public_key,
        &request.signing_bytes(),
        &request.signature,
    )
    .map_err(io_error)?;
    let identity = DeviceIdentity::load_existing(identity_path).map_err(io_error)?;
    let mut store = TeamStore::open(database).map_err(io_error)?;
    let profile = store
        .load_profile()
        .map_err(io_error)?
        .ok_or_else(|| io_error("local profile is missing"))?;
    if store.team_authority(team_id).map_err(io_error)? != profile.member.member_id
        || identity.device_id().is_empty()
    {
        return Err(io_error("only the configured team authority can approve a teammate"));
    }
    if !store
        .validate_approval_invitation(&request.invitation_id, team_id, request.created_at, now())
        .map_err(io_error)?
    {
        return Err(io_error("join request does not match an active invitation from this team"));
    }
    store.save_member(team_id, &request.member, now()).map_err(io_error)?;
    store.save_device(&request.device, now()).map_err(io_error)?;
    let mut member_event = Event::create(
        team_id.to_owned(), profile.member.member_id.clone(), identity.device_id(), EventType::MemberAdded,
        json!({"member_id": request.member.member_id, "display_name": request.member.display_name, "role": request.member.role, "active": true, "invitation_id": request.invitation_id}),
        now(), "pending",
    ).map_err(io_error)?;
    member_event.signature = Some(identity.sign(&member_event.signing_bytes()));
    store.insert_event(&member_event, now()).map_err(io_error)?;
    let mut device_event = Event::create(
        team_id.to_owned(), profile.member.member_id.clone(), identity.device_id(), EventType::DeviceAdded,
        json!({"member_id": request.member.member_id, "device_id": request.device.device_id, "public_key": request.device.public_key, "active": true}),
        now(), "pending",
    ).map_err(io_error)?;
    device_event.signature = Some(identity.sign(&device_event.signing_bytes()));
    store.insert_event(&device_event, now()).map_err(io_error)?;
    if let Some((endpoint, token)) = relay_credentials.map(|(url, token)| (url.to_owned(), token.to_owned())).or_else(|| {
        Some((std::env::var("HERDR_RELAY_URL").ok()?, std::env::var("HERDR_RELAY_TOKEN").ok()?))
    }) {
        super::relay::sync_team(database, team_id, &endpoint, &token).map_err(io_error)?;
    }
    Ok(())
}

fn validate_relay_token(token: &str) -> std::io::Result<()> {
    if token.len() == 43
        && token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        Ok(())
    } else {
        Err(io_error("relay token must be a 32-byte base64url bearer token"))
    }
}

fn create_team(args: &[String]) -> std::io::Result<i32> {
    let options = parse(args)?;
    let database = required_path(&options, "db")?;
    let endpoint = options
        .get("url")
        .cloned()
        .or_else(|| std::env::var("HERDR_RELAY_URL").ok())
        .ok_or_else(|| io_error("provide --url or set HERDR_RELAY_URL"))?;
    let endpoint = super::relay::validate_endpoint(&endpoint).map_err(io_error)?;
    let token = options.get("token").cloned().or_else(|| std::env::var("HERDR_RELAY_TOKEN").ok())
        .ok_or_else(|| io_error("set HERDR_RELAY_TOKEN or pass --token before creating the team so the relay credential can be retained for retry"))?;
    if token.len() != 43
        || !token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(io_error(
            "relay token must be a 32-byte base64url bearer token",
        ));
    }
    let identity =
        DeviceIdentity::load_existing(required_path(&options, "identity")?).map_err(io_error)?;
    let mut store = TeamStore::open(&database).map_err(io_error)?;
    let profile = store
        .load_profile()
        .map_err(io_error)?
        .ok_or_else(|| io_error("run `herdr onboard setup` before creating a team"))?;
    if !profile.member.active {
        return Err(io_error("inactive local member cannot create a team"));
    }
    let team = Team::create(
        required(&options, "name")?,
        profile.member.member_id.clone(),
        now(),
    )
    .map_err(io_error)?;
    let device = Device {
        device_id: identity.device_id().into(),
        member_id: profile.member.member_id.clone(),
        public_key: identity.public_key(),
        active: true,
    };
    store.save_team(&team).map_err(io_error)?;
    store
        .save_member(&team.team_id, &profile.member, now())
        .map_err(io_error)?;
    store.save_device(&device, now()).map_err(io_error)?;
    let mut event = Event::create(team.team_id.clone(), profile.member.member_id.clone(), identity.device_id(), EventType::TeamCreated,
        json!({"team_id": team.team_id, "name": team.name, "authority_member_id": team.authority_member_id, "created_at": team.created_at}), now(), "pending")
        .map_err(io_error)?;
    event.signature = Some(identity.sign(&event.signing_bytes()));
    store.insert_event(&event, now()).map_err(io_error)?;

    match super::relay::sync_team(&database, &team.team_id, &endpoint, &token) {
        Ok(_) => {
            println!("team_id: {}", team.team_id);
            println!("relay_url: {}", endpoint.trim_end_matches('/'));
            println!("Set HERDR_RELAY_URL={} and keep HERDR_RELAY_TOKEN set in this shell. Use `herdr team invite` to create a private invitation.", endpoint.trim_end_matches('/'));
            Ok(0)
        }
        Err(error) => Err(io_error(format!("team created locally but relay bootstrap failed: {error}; retry `herdr relay sync` with the same token supplied through your shell environment"))),
    }
}

fn create_invite(args: &[String]) -> std::io::Result<i32> {
    let options = parse(args)?;
    let database = required_path(&options, "db")?;
    let team_id = required(&options, "team")?;
    let endpoint = options
        .get("url")
        .cloned()
        .or_else(|| std::env::var("HERDR_RELAY_URL").ok())
        .ok_or_else(|| io_error("provide --url or set HERDR_RELAY_URL"))?;
    let endpoint = super::relay::validate_endpoint(&endpoint).map_err(io_error)?;
    let token = options
        .get("token")
        .cloned()
        .or_else(|| std::env::var("HERDR_RELAY_TOKEN").ok())
        .ok_or_else(|| io_error("provide --token or set HERDR_RELAY_TOKEN"))?;
    if token.len() != 43
        || !token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(io_error(
            "relay token must be a 32-byte base64url bearer token",
        ));
    }
    let identity =
        DeviceIdentity::load_existing(required_path(&options, "identity")?).map_err(io_error)?;
    let store = TeamStore::open(&database).map_err(io_error)?;
    let profile = store
        .load_profile()
        .map_err(io_error)?
        .ok_or_else(|| io_error("local profile is missing"))?;
    let team = store
        .team(&team_id)
        .map_err(io_error)?
        .ok_or_else(|| io_error("team is not configured locally"))?;
    if team.authority_member_id != profile.member.member_id {
        return Err(io_error("only the team authority can create invitations"));
    }
    let device = Device {
        device_id: identity.device_id().into(),
        member_id: profile.member.member_id.clone(),
        public_key: identity.public_key(),
        active: true,
    };
    if store
        .device_public_key(&device.device_id)
        .map_err(io_error)?
        .as_deref()
        != Some(device.public_key.as_str())
    {
        return Err(io_error(
            "authority device is not approved in the team database",
        ));
    }
    let pairing_code = generate_token();
    let mut invitation = TeamInvitation::new_unsigned(
        &team,
        &device,
        &pairing_code,
        now(),
        now() + 7 * 24 * 60 * 60,
    )
    .map_err(io_error)?;
    let signature = identity.sign(&invitation.signing_bytes());
    invitation = invitation.with_signature(signature).map_err(io_error)?;
    store
        .save_invitation(&invitation, now())
        .map_err(io_error)?;
    let bundle = InviteBundle {
        invitation,
        pairing_code,
        relay_url: endpoint,
        relay_token: token,
    };
    let output = required_path(&options, "out")?;
    write_json_file(&output, &bundle)?;
    println!("Signed invite created at {}. Share this file with the intended teammate through a private channel; it contains the shared relay credential and expires in seven days.", output.display());
    Ok(0)
}

fn accept_invite(args: &[String]) -> std::io::Result<i32> {
    let options = parse(args)?;
    let database = required_path(&options, "db")?;
    let identity =
        DeviceIdentity::load_existing(required_path(&options, "identity")?).map_err(io_error)?;
    let bundle: InviteBundle = read_json_file(&required_path(&options, "invite")?)?;
    let mut store = TeamStore::open(&database).map_err(io_error)?;
    let profile = store
        .load_profile()
        .map_err(io_error)?
        .ok_or_else(|| io_error("run `herdr onboard setup` before joining a team"))?;
    let joining_device = Device {
        device_id: identity.device_id().into(),
        member_id: profile.member.member_id.clone(),
        public_key: identity.public_key(),
        active: true,
    };
    let authority_member = Member {
        member_id: bundle.invitation.authority_member_id.clone(),
        display_name: bundle.invitation.team_name.clone(),
        role: Role::Manager,
        active: true,
    };
    let authority_device = Device {
        device_id: bundle.invitation.authority_device_id.clone(),
        member_id: bundle.invitation.authority_member_id.clone(),
        public_key: bundle.invitation.authority_public_key.clone(),
        active: true,
    };
    store
        .accept_invitation(
            &bundle.invitation,
            &bundle.pairing_code,
            &authority_member,
            &authority_device,
            &profile.member,
            &joining_device,
            &identity,
            now(),
        )
        .map_err(io_error)?;
    let mut request = JoinRequest {
        invitation_id: bundle.invitation.invitation_id.clone(),
        team_id: bundle.invitation.team_id.clone(),
        member: profile.member,
        device: joining_device,
        created_at: now(),
        signature: String::new(),
    };
    request.signature = identity.sign(&request.signing_bytes());
    let request_path = required_path(&options, "request-out")?;
    write_json_file(&request_path, &request)?;
    println!("Invitation accepted locally. Share {} with the team authority for device approval, then set HERDR_RELAY_URL={} and HERDR_RELAY_TOKEN privately before syncing.", request_path.display(), bundle.relay_url);
    println!("The relay credential is in the invite bundle; copy it to HERDR_RELAY_TOKEN in your shell without committing it to the repository.");
    Ok(0)
}

fn approve_member(args: &[String]) -> std::io::Result<i32> {
    let options = parse(args)?;
    let database = required_path(&options, "db")?;
    let team_id = required(&options, "team")?;
    let identity =
        DeviceIdentity::load_existing(required_path(&options, "identity")?).map_err(io_error)?;
    let request: JoinRequest = read_json_file(&required_path(&options, "request")?)?;
    if request.team_id != team_id
        || !request.member.active
        || !request.device.active
        || request.device.member_id != request.member.member_id
    {
        return Err(io_error(
            "join request team/member/device fields are inconsistent",
        ));
    }
    if DeviceIdentity::device_id_for_public_key(&request.device.public_key).map_err(io_error)?
        != request.device.device_id
    {
        return Err(io_error(
            "join request device ID does not match its public key",
        ));
    }
    DeviceIdentity::verify(
        &request.device.public_key,
        &request.signing_bytes(),
        &request.signature,
    )
    .map_err(io_error)?;
    let mut store = TeamStore::open(&database).map_err(io_error)?;
    let profile = store
        .load_profile()
        .map_err(io_error)?
        .ok_or_else(|| io_error("local profile is missing"))?;
    if store.team_authority(&team_id).map_err(io_error)? != profile.member.member_id
        || identity.device_id().is_empty()
    {
        return Err(io_error(
            "only the configured team authority can approve a teammate",
        ));
    }
    if !store
        .validate_approval_invitation(&request.invitation_id, &team_id, request.created_at, now())
        .map_err(io_error)?
    {
        return Err(io_error(
            "join request does not match an active invitation from this team",
        ));
    }
    store
        .save_member(&team_id, &request.member, now())
        .map_err(io_error)?;
    store
        .save_device(&request.device, now())
        .map_err(io_error)?;
    let mut member_event = Event::create(team_id.clone(), profile.member.member_id.clone(), identity.device_id(), EventType::MemberAdded,
        json!({"member_id": request.member.member_id, "display_name": request.member.display_name, "role": request.member.role, "active": true, "invitation_id": request.invitation_id}), now(), "pending").map_err(io_error)?;
    member_event.signature = Some(identity.sign(&member_event.signing_bytes()));
    store.insert_event(&member_event, now()).map_err(io_error)?;
    let mut device_event = Event::create(team_id.clone(), profile.member.member_id.clone(), identity.device_id(), EventType::DeviceAdded,
        json!({"member_id": request.member.member_id, "device_id": request.device.device_id, "public_key": request.device.public_key, "active": true}), now(), "pending").map_err(io_error)?;
    device_event.signature = Some(identity.sign(&device_event.signing_bytes()));
    store.insert_event(&device_event, now()).map_err(io_error)?;
    let endpoint = options
        .get("url")
        .cloned()
        .or_else(|| std::env::var("HERDR_RELAY_URL").ok());
    let token = options
        .get("token")
        .cloned()
        .or_else(|| std::env::var("HERDR_RELAY_TOKEN").ok());
    if let (Some(endpoint), Some(token)) = (endpoint, token) {
        super::relay::sync_team(&database, &team_id, &endpoint, &token).map_err(io_error)?;
    } else {
        eprintln!("approval is saved locally; configure HERDR_RELAY_URL and HERDR_RELAY_TOKEN then run `herdr relay sync` to publish it");
    }
    println!(
        "approved member {} with device {}",
        request.member.member_id, request.device.device_id
    );
    Ok(0)
}

fn parse(args: &[String]) -> std::io::Result<HashMap<String, String>> {
    let mut map = HashMap::new();
    let mut index = 0;
    while index < args.len() {
        let key = args[index]
            .strip_prefix("--")
            .ok_or_else(|| io_error("team options must use --name value"))?;
        let value = args
            .get(index + 1)
            .ok_or_else(|| io_error(format!("missing value for --{key}")))?;
        if map.insert(key.to_string(), value.clone()).is_some() {
            return Err(io_error(format!("duplicate --{key}")));
        }
        index += 2;
    }
    Ok(map)
}

fn required(options: &HashMap<String, String>, key: &str) -> std::io::Result<String> {
    options
        .get(key)
        .filter(|value| !value.trim().is_empty())
        .cloned()
        .ok_or_else(|| io_error(format!("--{key} is required")))
}
fn required_path(options: &HashMap<String, String>, key: &str) -> std::io::Result<PathBuf> {
    Ok(PathBuf::from(required(options, key)?))
}
fn generate_token() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn io_error(error: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(error.to_string())
}
fn write_json_file(path: &Path, value: &impl Serialize) -> std::io::Result<()> {
    std::fs::write(path, serde_json::to_vec_pretty(value).map_err(io_error)?)
}
fn read_json_file<T: for<'de> Deserialize<'de>>(path: &Path) -> std::io::Result<T> {
    let metadata = std::fs::metadata(path)?;
    if metadata.len() > 64 * 1024 {
        return Err(io_error("team invite/request file exceeds 64 KiB"));
    }
    serde_json::from_slice(&std::fs::read(path)?).map_err(io_error)
}
fn print_help() {
    println!("usage:\n  herdr team create --db PATH --identity PATH --name NAME --url HTTPS_URL [--token TOKEN]\n  herdr team invite --db PATH --team ID --identity PATH --out FILE\n  herdr team join --db PATH --identity PATH --invite FILE --request-out FILE\n  herdr team approve --db PATH --team ID --identity PATH --request FILE\n\nRun onboard setup on each device first. Set HERDR_RELAY_TOKEN before creating a team. Invite files contain a shared relay token; transfer them privately and never commit them. Joiners must send a signed device request to the authority, who approves it before the joiner's events are accepted.");
}
