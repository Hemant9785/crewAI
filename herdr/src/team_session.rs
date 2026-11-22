//! Device-local onboarding, remembered connections, and non-destructive team selection.
//! Secrets are authenticated with the protected device identity and never enter team events.

use crate::cli::team::InviteBundle;
use crate::identity::DeviceIdentity;
use crate::team::{Device, Member, RepositoryApproval, Role, TeamInvitation};
use crate::team_storage::TeamStore;
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Serialize, Deserialize, Eq, PartialEq)]
pub(crate) struct RelayConnection {
    pub endpoint: String,
    pub token: String,
}

impl std::fmt::Debug for RelayConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RelayConnection")
            .field("credentials", &"[protected]")
            .finish()
    }
}

fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn digest(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

pub(crate) fn identity_path(db: &Path) -> PathBuf {
    db.with_file_name(format!(
        "{}.identity.json",
        db.file_stem().and_then(|s| s.to_str()).unwrap_or("team")
    ))
}

fn read_bounded(path: &Path, limit: usize) -> Result<Option<Vec<u8>>, String> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(err(error)),
    };
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(err)?;
    if bytes.len() > limit {
        return Err("Saved local setting exceeds its size limit".into());
    }
    Ok(Some(bytes))
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(err)?;
    let temporary = parent.join(format!(".local-state-{:016x}.tmp", OsRng.next_u64()));
    let result = (|| {
        let mut file = crate::platform::create_private_state_file(&temporary).map_err(err)?;
        file.write_all(bytes).map_err(err)?;
        file.sync_all().map_err(err)?;
        drop(file);
        crate::platform::replace_file(&temporary, path).map_err(err)?;
        crate::platform::sync_parent_directory(parent).map_err(err)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn save_protected(db: &Path, path: &Path, context: &str, bytes: &[u8]) -> Result<(), String> {
    let identity = DeviceIdentity::load_existing(identity_path(db)).map_err(err)?;
    atomic_write(
        path,
        identity.seal_local(context, bytes).map_err(err)?.as_bytes(),
    )
}

fn load_protected(db: &Path, path: &Path, context: &str) -> Result<Option<Vec<u8>>, String> {
    let Some(bytes) = read_bounded(path, 192 * 1024)? else {
        return Ok(None);
    };
    let identity = DeviceIdentity::load_existing(identity_path(db)).map_err(err)?;
    let sealed = std::str::from_utf8(&bytes).map_err(|_| "Invalid saved local setting")?;
    identity.open_local(context, sealed).map(Some).map_err(err)
}

fn validate_connection(endpoint: &str, token: &str) -> Result<(), String> {
    crate::cli::relay::validate_endpoint(endpoint).map_err(err)?;
    if token.trim().is_empty() || token.len() > 4096 || token.chars().any(char::is_control) {
        return Err("Relay access token is missing or invalid".into());
    }
    Ok(())
}

pub(crate) fn save_connection(db: &Path, endpoint: &str, token: &str) -> Result<(), String> {
    validate_connection(endpoint, token)?;
    let connection = RelayConnection {
        endpoint: endpoint.into(),
        token: token.into(),
    };
    save_protected(
        db,
        &db.with_extension("connection.json"),
        "relay-connection/v1",
        &serde_json::to_vec(&connection).map_err(err)?,
    )
}

pub(crate) fn load_connection(db: &Path) -> Result<Option<RelayConnection>, String> {
    let Some(bytes) = load_protected(
        db,
        &db.with_extension("connection.json"),
        "relay-connection/v1",
    )?
    else {
        return Ok(None);
    };
    let value: RelayConnection =
        serde_json::from_slice(&bytes).map_err(|_| "Invalid saved connection")?;
    validate_connection(&value.endpoint, &value.token)?;
    Ok(Some(value))
}

pub(crate) fn connection_from_env_or_store(db: &Path) -> Result<Option<RelayConnection>, String> {
    connection_override(
        db,
        std::env::var("HERDR_RELAY_URL").ok(),
        std::env::var("HERDR_RELAY_TOKEN").ok(),
    )
}

fn connection_override(
    db: &Path,
    endpoint: Option<String>,
    token: Option<String>,
) -> Result<Option<RelayConnection>, String> {
    match (endpoint, token) {
        (None, None) => load_connection(db),
        (Some(endpoint), Some(token)) => {
            validate_connection(&endpoint, &token)?;
            Ok(Some(RelayConnection { endpoint, token }))
        }
        _ => Err("Set both HERDR_RELAY_URL and HERDR_RELAY_TOKEN for a custom relay".into()),
    }
}

pub(crate) fn setup_profile(db: &Path, name: &str, role: Role) -> Result<Member, String> {
    let name = name.trim();
    if name.is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
        return Err("Enter a name between 1 and 256 characters".into());
    }
    let identity = DeviceIdentity::load_or_create(identity_path(db)).map_err(err)?;
    let store = TeamStore::open(db).map_err(err)?;
    let previous = store.load_profile().map_err(err)?;
    let created_at = previous.as_ref().map(|p| p.created_at).unwrap_or_else(now);
    let member = match previous {
        Some(profile) => Member {
            display_name: name.into(),
            role: if store.teams().map_err(err)?.is_empty() {
                role
            } else {
                profile.member.role
            },
            ..profile.member
        },
        None => Member {
            member_id: format!("member-{}", &digest(identity.device_id())[..24]),
            display_name: name.into(),
            role,
            active: true,
        },
    };
    store
        .save_profile(&member, created_at, now())
        .map_err(err)?;
    store
        .save_device(
            &Device {
                device_id: identity.device_id().into(),
                member_id: member.member_id.clone(),
                public_key: identity.public_key(),
                active: true,
            },
            now(),
        )
        .map_err(err)?;
    Ok(member)
}

pub(crate) fn link_project(db: &Path, folder: &Path) -> Result<RepositoryApproval, String> {
    let folder = folder
        .canonicalize()
        .map_err(|_| "Choose an existing project folder")?;
    if !folder.is_dir() {
        return Err("Choose a project folder".into());
    }
    let store = TeamStore::open(db).map_err(err)?;
    let profile = store
        .load_profile()
        .map_err(err)?
        .ok_or("Set up your name first")?;
    let local_path = folder.to_string_lossy().into_owned();
    let approval = RepositoryApproval {
        repository_id: format!(
            "repo-{}",
            &digest(&format!("{}\0{}", profile.member.member_id, local_path))[..24]
        ),
        owner_member_id: profile.member.member_id,
        local_path,
        approved_at: now(),
    };
    store.save_repository_approval(&approval).map_err(err)?;
    Ok(approval)
}

fn draft_path(db: &Path, kind: &str) -> Result<PathBuf, String> {
    if kind.is_empty()
        || kind.len() > 40
        || !kind.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
    {
        return Err("Invalid draft kind".into());
    }
    Ok(db.with_extension(format!("{kind}.draft.json")))
}

pub(crate) fn save_draft(db: &Path, kind: &str, text: &str) -> Result<(), String> {
    if text.len() > 64 * 1024 {
        return Err("Draft exceeds 64 KiB".into());
    }
    save_protected(
        db,
        &draft_path(db, kind)?,
        &format!("draft/{kind}/v1"),
        text.as_bytes(),
    )
}

pub(crate) fn load_draft(db: &Path, kind: &str) -> Result<Option<String>, String> {
    let bytes = load_protected(db, &draft_path(db, kind)?, &format!("draft/{kind}/v1"))?;
    match bytes {
        None => Ok(None),
        Some(bytes) if bytes.is_empty() => Ok(None),
        Some(bytes) if bytes.len() <= 64 * 1024 => String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| "Invalid saved draft".into()),
        _ => Err("Saved draft exceeds 64 KiB".into()),
    }
}

#[derive(Serialize, Deserialize)]
struct Selection {
    version: u32,
    context: String,
}

pub(crate) fn contexts_root(base: &Path) -> PathBuf {
    base.with_extension("teams")
}
fn valid_context(key: &str) -> bool {
    key.len() == 64
        && key
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn context_database(base: &Path, key: &str) -> PathBuf {
    contexts_root(base).join(key).join("team.sqlite")
}

fn contained_context(base: &Path, path: &Path) -> bool {
    match (contexts_root(base).canonicalize(), path.canonicalize()) {
        (Ok(root), Ok(path)) => path.starts_with(root),
        _ => false,
    }
}

pub(crate) fn resolve_active_database(base: &Path) -> PathBuf {
    let selected = (|| {
        let bytes = read_bounded(&base.with_extension("active.json"), 1024).ok()??;
        let selection: Selection = serde_json::from_slice(&bytes).ok()?;
        if selection.version != 1 || !valid_context(&selection.context) {
            return None;
        }
        let path = context_database(base, &selection.context);
        (path.is_file() && contained_context(base, &path)).then_some(path)
    })();
    selected.unwrap_or_else(|| base.to_path_buf())
}

pub(crate) fn activate_database(base: &Path, destination: &Path) -> Result<(), String> {
    let context = if destination == base {
        "base".to_string()
    } else {
        let key = destination
            .parent()
            .and_then(Path::file_name)
            .and_then(|s| s.to_str())
            .ok_or("Invalid team selection")?;
        if !valid_context(key)
            || destination != context_database(base, key)
            || !contained_context(base, destination)
        {
            return Err("Invalid team selection".into());
        }
        key.into()
    };
    let store = TeamStore::open_read_only(destination).map_err(err)?;
    if store.load_profile().map_err(err)?.is_none() {
        return Err("Set up your name first".into());
    }
    atomic_write(
        &base.with_extension("active.json"),
        &serde_json::to_vec(&Selection {
            version: 1,
            context,
        })
        .map_err(err)?,
    )
}

pub(crate) fn list_contexts(base: &Path) -> Result<Vec<(PathBuf, String)>, String> {
    let mut paths = vec![base.to_path_buf()];
    match fs::read_dir(contexts_root(base)) {
        Ok(entries) => {
            for entry in entries.take(100).flatten() {
                if let Some(key) = entry.file_name().to_str().filter(|key| valid_context(key)) {
                    let path = context_database(base, key);
                    if contained_context(base, &path) {
                        paths.push(path);
                    }
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(err(error)),
    }
    let mut contexts = Vec::new();
    for path in paths {
        if !path.is_file() {
            continue;
        }
        if let Ok(store) = TeamStore::open_read_only(&path) {
            if let Ok(teams) = store.teams() {
                if let Some(team) = teams.first() {
                    contexts.push((path, team.name.clone()));
                }
            }
        }
    }
    contexts.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    Ok(contexts)
}

/// Validate first, then prepare a separate profile without changing active selection.
/// Caller accepts the invite, saves credentials, and only then activates the database.
pub(crate) fn prepare_join_database(
    base: &Path,
    current: &Path,
    bundle: &InviteBundle,
    name: &str,
) -> Result<PathBuf, String> {
    let invitation = &bundle.invitation;
    crate::cli::team::validate_invite_for_existing_team(
        bundle,
        &invitation.team_id,
        &invitation.authority_member_id,
    )
    .map_err(err)?;
    if TeamInvitation::pairing_code_hash(&bundle.pairing_code) != invitation.pairing_code_hash {
        return Err("Invalid invitation code".into());
    }
    let mut paths: Vec<_> = list_contexts(base)?.into_iter().map(|(p, _)| p).collect();
    if !paths.iter().any(|p| p == current) {
        paths.push(current.to_path_buf());
    }
    for path in paths {
        if !path.is_file() {
            continue;
        }
        let store = TeamStore::open_read_only(&path).map_err(err)?;
        if let Some(team) = store.team(&invitation.team_id).map_err(err)? {
            if team.authority_member_id != invitation.authority_member_id
                || store
                    .device_public_key(&invitation.authority_device_id)
                    .map_err(err)?
                    .as_deref()
                    != Some(&invitation.authority_public_key)
            {
                return Err("Invitation does not match this team's trusted authority".into());
            }
            return Ok(path);
        }
    }
    let reuse_current = if current.is_file() {
        let store = TeamStore::open_read_only(current).map_err(err)?;
        store.teams().map_err(err)?.is_empty()
            && store
                .load_profile()
                .map_err(err)?
                .map(|p| p.member.member_id != invitation.authority_member_id)
                .unwrap_or(true)
    } else {
        true
    };
    let destination = if reuse_current {
        current.to_path_buf()
    } else {
        context_database(base, &digest(&invitation.team_id))
    };
    setup_profile(&destination, name, Role::Implementer)?;
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::team::Team;

    struct Sandbox(PathBuf);
    impl Sandbox {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!("herdr-session-{:016x}", OsRng.next_u64())))
        }
        fn db(&self) -> PathBuf {
            self.0.join("team.sqlite")
        }
    }
    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn bundle(root: &Sandbox) -> InviteBundle {
        let identity = DeviceIdentity::load_or_create(root.0.join("authority.json")).unwrap();
        let team = Team::create("Friends", "manager", now()).unwrap();
        let device = Device {
            device_id: identity.device_id().into(),
            member_id: "manager".into(),
            public_key: identity.public_key(),
            active: true,
        };
        let pairing_code = "private-pairing-test-code".to_string();
        let invitation =
            TeamInvitation::new_unsigned(&team, &device, &pairing_code, now(), now() + 600)
                .unwrap();
        let signature = identity.sign(&invitation.signing_bytes());
        InviteBundle {
            invitation: invitation.with_signature(signature).unwrap(),
            pairing_code,
            relay_url: "https://example.com".into(),
            relay_token: "test-access-token".into(),
        }
    }

    #[test]
    fn encrypted_connection_survives_restart_without_plaintext() {
        let root = Sandbox::new();
        let db = root.db();
        let original = setup_profile(&db, "Hemant", Role::Implementer).unwrap();
        save_connection(&db, "https://example.com", "private-token").unwrap();
        let bytes = fs::read(db.with_extension("connection.json")).unwrap();
        assert!(!String::from_utf8(bytes).unwrap().contains("private-token"));
        assert_eq!(
            load_connection(&db).unwrap().unwrap().token,
            "private-token"
        );
        let edited = setup_profile(&db, "New name", Role::Manager).unwrap();
        assert_eq!(original.member_id, edited.member_id);
        assert!(TeamStore::open_read_only(&db)
            .unwrap()
            .repository_approvals()
            .unwrap()
            .is_empty());
        assert!(connection_override(&db, Some("https://example.com".into()), None).is_err());
        assert_eq!(
            connection_override(
                &db,
                Some("https://custom.example.com".into()),
                Some("override".into())
            )
            .unwrap()
            .unwrap()
            .token,
            "override"
        );
        assert_eq!(
            load_connection(&db).unwrap().unwrap().token,
            "private-token"
        );
    }

    #[test]
    fn authentication_rejects_wrong_identity_context_and_tampering() {
        let root = Sandbox::new();
        let a = DeviceIdentity::load_or_create(root.0.join("a.json")).unwrap();
        let b = DeviceIdentity::load_or_create(root.0.join("b.json")).unwrap();
        let sealed = a.seal_local("connection", b"private").unwrap();
        assert_eq!(a.open_local("connection", &sealed).unwrap(), b"private");
        assert_ne!(sealed, a.seal_local("connection", b"private").unwrap());
        assert!(b.open_local("connection", &sealed).is_err());
        assert!(a.open_local("draft", &sealed).is_err());
        let mut damaged = sealed.into_bytes();
        damaged[20] = if damaged[20] == b'A' { b'B' } else { b'A' };
        assert!(a
            .open_local("connection", std::str::from_utf8(&damaged).unwrap())
            .is_err());
        assert!(a.open_local("connection", "local-v1:AA==").is_err());
    }

    #[test]
    fn drafts_are_bounded_private_and_clearable() {
        let root = Sandbox::new();
        let db = root.db();
        setup_profile(&db, "Teammate", Role::Implementer).unwrap();
        save_draft(&db, "message", "hello friends").unwrap();
        assert_eq!(
            load_draft(&db, "message").unwrap().as_deref(),
            Some("hello friends")
        );
        assert!(!fs::read_to_string(draft_path(&db, "message").unwrap())
            .unwrap()
            .contains("hello friends"));
        assert!(save_draft(&db, "message", &"x".repeat(65537)).is_err());
        assert!(save_draft(&db, "../bad", "x").is_err());
        save_draft(&db, "message", "").unwrap();
        assert_eq!(load_draft(&db, "message").unwrap(), None);
    }

    #[test]
    fn joining_prepares_without_switching_and_preserves_prior_team() {
        let root = Sandbox::new();
        let db = root.db();
        let original = setup_profile(&db, "Owner", Role::Manager).unwrap();
        let old_team = Team::create("Original", &original.member_id, now()).unwrap();
        TeamStore::open(&db).unwrap().save_team(&old_team).unwrap();
        let invite = bundle(&root);
        let destination = prepare_join_database(&db, &db, &invite, "Owner").unwrap();
        assert_ne!(destination, db);
        assert_eq!(resolve_active_database(&db), db);
        assert_eq!(
            TeamStore::open_read_only(&db)
                .unwrap()
                .load_profile()
                .unwrap()
                .unwrap()
                .member,
            original
        );
        assert_eq!(
            TeamStore::open_read_only(&db).unwrap().teams().unwrap(),
            vec![old_team]
        );
        let new_profile = TeamStore::open_read_only(&destination)
            .unwrap()
            .load_profile()
            .unwrap()
            .unwrap();
        assert_ne!(new_profile.member.member_id, original.member_id);
        activate_database(&db, &destination).unwrap();
        assert_eq!(resolve_active_database(&db), destination);
        activate_database(&db, &db).unwrap();
        assert_eq!(resolve_active_database(&db), db);
    }

    #[test]
    fn invalid_invites_and_selection_do_not_change_data() {
        let root = Sandbox::new();
        let db = root.db();
        let mut invite = bundle(&root);
        invite.pairing_code = "incorrect-code".into();
        assert!(prepare_join_database(&db, &db, &invite, "Teammate").is_err());
        assert!(!db.exists());
        assert!(!contexts_root(&db).exists());
        atomic_write(
            &db.with_extension("active.json"),
            br#"{"version":1,"context":"../../elsewhere"}"#,
        )
        .unwrap();
        assert_eq!(resolve_active_database(&db), db);
        assert!(activate_database(&db, &root.0.join("untrusted.sqlite")).is_err());
    }
}
