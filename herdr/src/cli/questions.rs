//! Guided question workflow. Only bounded answers and project labels cross devices.
use crate::identity::DeviceIdentity;
use crate::team::{
    DeliveryState, Event, EventType, RepositoryApproval, RequestKind, RequestResult,
};
use crate::team_storage::{
    permission_events::{owner_prefix, PublishedProject, QuestionGrant},
    TeamStore,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) type Project = PublishedProject;
#[derive(Clone, Debug)]
pub(crate) struct Question {
    pub request_id: String,
    pub requester_member_id: String,
    pub target_member_id: String,
    pub repository_id: String,
    pub question: String,
    pub delivery: String,
    pub result: Option<String>,
    pub consent: String,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum QuestionDecision {
    AllowOnce,
    AllowProject,
    Deny,
}
#[derive(Clone, Debug)]
pub(crate) struct QuestionPolicy {
    pub warning: String,
    pub confirmation_required: bool,
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn profile(store: &TeamStore) -> Result<String, String> {
    Ok(store
        .load_profile()
        .map_err(|e| e.to_string())?
        .ok_or("Create your profile first")?
        .member
        .member_id)
}
fn append(
    store: &mut TeamStore,
    id: &DeviceIdentity,
    team: &str,
    actor: &str,
    kind: EventType,
    payload: Value,
) -> Result<String, String> {
    let at = now();
    let mut event = Event::create(team, actor, id.device_id(), kind, payload, at, "pending")
        .map_err(|e| e.to_string())?;
    event.signature = Some(id.sign(&event.signing_bytes()));
    store
        .insert_verified_event(&event, at)
        .map_err(|e| e.to_string())?;
    crate::outbox::queue_event(store, &event.event_id, at).map_err(|e| e.to_string())?;
    Ok(event.event_id)
}
fn lifecycle(
    store: &mut TeamStore,
    id: &DeviceIdentity,
    team: &str,
    actor: &str,
    kind: EventType,
    request: &str,
    result: Option<RequestResult>,
) -> Result<(), String> {
    append(
        store,
        id,
        team,
        actor,
        kind,
        json!({"request_id":request,"result":result}),
    )
    .map(|_| ())
}
fn payload(request: &crate::team::RequestRecord) -> Result<(String, String), String> {
    let p: Value =
        serde_json::from_str(&request.payload).map_err(|_| "Invalid question payload")?;
    Ok((
        p["repository_id"].as_str().ok_or("Missing project")?.into(),
        p["question"].as_str().ok_or("Missing question")?.into(),
    ))
}
fn valid_question(question: &str) -> Result<(), String> {
    if question.trim().is_empty() || question.len() > 16 * 1024 {
        Err("Enter a question of at most 16 KiB".into())
    } else {
        Ok(())
    }
}
pub(crate) fn publish_project(
    database: &Path,
    identity: &Path,
    repository_id: &str,
    label: &str,
) -> Result<Project, String> {
    if label.trim().is_empty() || label.len() > 120 || label.contains(['\n', '\r']) {
        return Err("Enter a project name of at most 120 characters".into());
    }
    let id = DeviceIdentity::load_existing(identity).map_err(|e| e.to_string())?;
    let mut store = TeamStore::open(database).map_err(|e| e.to_string())?;
    let owner = profile(&store)?;
    let approval = store
        .repository_approval(repository_id)
        .map_err(|e| e.to_string())?
        .ok_or("Connect and approve the project folder first")?;
    if approval.owner_member_id != owner {
        return Err("Only this project's owner can share its label".into());
    }
    let public_id = if repository_id.starts_with(&owner_prefix(&owner)) {
        repository_id.into()
    } else {
        format!(
            "{}{:x}",
            owner_prefix(&owner),
            Sha256::digest(repository_id.as_bytes())
        )
    };
    store
        .save_repository_approval(&RepositoryApproval {
            repository_id: public_id.clone(),
            owner_member_id: owner.clone(),
            local_path: approval.local_path,
            approved_at: approval.approved_at,
        })
        .map_err(|e| e.to_string())?;
    let p = Project {
        repository_id: public_id,
        owner_member_id: owner.clone(),
        label: label.trim().into(),
    };
    for team in store.teams().map_err(|e| e.to_string())? {
        append(
            &mut store,
            &id,
            &team.team_id,
            &owner,
            EventType::RepositoryApproved,
            serde_json::to_value(&p).map_err(|e| e.to_string())?,
        )?;
    }
    Ok(p)
}
/// Idempotently announce approved local projects after a device joins a team.
/// Call before uploading team events and again after applying membership approval.
/// Only labels and owner-scoped IDs leave this device.
pub(crate) fn publish_local_projects(database: &Path, identity: &Path) -> Result<usize, String> {
    let id = DeviceIdentity::load_existing(identity).map_err(|e| e.to_string())?;
    let mut store = TeamStore::open(database).map_err(|e| e.to_string())?;
    let owner = profile(&store)?;
    if store
        .device_public_key(id.device_id())
        .map_err(|e| e.to_string())?
        .as_deref()
        != Some(id.public_key().as_str())
    {
        return Ok(0);
    }
    let teams = store.teams().map_err(|e| e.to_string())?;
    let approvals = store.repository_approvals().map_err(|e| e.to_string())?;
    let mut announced = 0;
    for approval in approvals.into_iter().filter(|a| a.owner_member_id == owner) {
        let repository_id = if approval.repository_id.starts_with(&owner_prefix(&owner)) {
            approval.repository_id.clone()
        } else {
            format!(
                "{}{:x}",
                owner_prefix(&owner),
                Sha256::digest(approval.repository_id.as_bytes())
            )
        };
        let label = Path::new(&approval.local_path)
            .file_name()
            .and_then(|n| n.to_str())
            .filter(|n| !n.trim().is_empty())
            .unwrap_or("My project")
            .chars()
            .take(120)
            .collect::<String>()
            .replace(['\n', '\r'], " ");
        store
            .save_repository_approval(&RepositoryApproval {
                repository_id: repository_id.clone(),
                owner_member_id: owner.clone(),
                local_path: approval.local_path,
                approved_at: approval.approved_at,
            })
            .map_err(|e| e.to_string())?;
        for team in &teams {
            if store
                .published_projects(&team.team_id)
                .map_err(|e| e.to_string())?
                .iter()
                .any(|p| p.repository_id == repository_id)
            {
                continue;
            }
            let metadata = Project {
                repository_id: repository_id.clone(),
                owner_member_id: owner.clone(),
                label: label.clone(),
            };
            append(
                &mut store,
                &id,
                &team.team_id,
                &owner,
                EventType::RepositoryApproved,
                serde_json::to_value(metadata).map_err(|e| e.to_string())?,
            )?;
            announced += 1;
        }
    }
    Ok(announced)
}
pub(crate) fn list_projects(
    database: &Path,
    team: &str,
    target: &str,
) -> Result<Vec<Project>, String> {
    let s = TeamStore::open_read_only(database).map_err(|e| e.to_string())?;
    Ok(s.published_projects(team)
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|p| p.owner_member_id == target)
        .collect())
}
pub(crate) fn ask(
    database: &Path,
    identity: &Path,
    team: &str,
    target: &str,
    repo: &str,
    question: &str,
) -> Result<String, String> {
    valid_question(question)?;
    let id = DeviceIdentity::load_existing(identity).map_err(|e| e.to_string())?;
    let mut s = TeamStore::open(database).map_err(|e| e.to_string())?;
    let actor = profile(&s)?;
    if !s
        .published_projects(team)
        .map_err(|e| e.to_string())?
        .iter()
        .any(|p| p.repository_id == repo && p.owner_member_id == target)
    {
        return Err("Your teammate has not shared this project's name yet. Ask them to connect a project first.".into());
    }
    let request = format!("question-{}", super::team::generate_token());
    append(
        &mut s,
        &id,
        team,
        &actor,
        EventType::RequestCreated,
        json!({"request_id":request,"requester_member_id":actor,"target_member_id":target,"kind":RequestKind::Check,"payload":json!({"repository_id":repo,"question":question}).to_string(),"created_at":now()}),
    )?;
    lifecycle(
        &mut s,
        &id,
        team,
        &actor,
        EventType::RequestQueued,
        &request,
        None,
    )?;
    Ok(request)
}
pub(crate) fn pending_questions(database: &Path) -> Result<Vec<Question>, String> {
    let s = TeamStore::open_read_only(database).map_err(|e| e.to_string())?;
    let actor = profile(&s)?;
    s.requests()
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|r| {
            matches!(r.kind, RequestKind::Check | RequestKind::Read)
                && (r.requester_member_id == actor || r.target_member_id == actor)
        })
        .map(|r| {
            let (repo, question) = payload(&r)?;
            Ok(Question {
                request_id: r.request_id,
                requester_member_id: r.requester_member_id,
                target_member_id: r.target_member_id,
                repository_id: repo,
                question,
                delivery: format!("{:?}", r.delivery),
                result: r.result.map(|x| x.error.unwrap_or(x.stdout)),
                consent: if matches!(
                    r.delivery,
                    DeliveryState::Created | DeliveryState::Queued | DeliveryState::Delivered
                ) {
                    "Waiting for project owner's approval".into()
                } else {
                    format!("{:?}", r.delivery)
                },
            })
        })
        .collect()
}
pub(crate) fn policy(database: &Path, repo: &str) -> Result<QuestionPolicy, String> {
    let s = TeamStore::open_read_only(database).map_err(|e| e.to_string())?;
    let a = s
        .repository_approval(repo)
        .map_err(|e| e.to_string())?
        .ok_or("Connect and approve this project folder first")?;
    let p = crate::execution_boundary::assess_check_policy(Path::new(&a.local_path));
    Ok(QuestionPolicy{warning:"Codex will run locally with read-only sandbox flags, but Herdr cannot verify operating-system isolation in this build. This is limited isolation: the process may access files or network outside the project. Only continue if you trust this question. Only its bounded final answer is shared.".into(),confirmation_required:p.confirmation_required})
}
pub(crate) fn revoke(
    database: &Path,
    identity: &Path,
    team: &str,
    grant: &str,
) -> Result<(), String> {
    let id = DeviceIdentity::load_existing(identity).map_err(|e| e.to_string())?;
    let mut s = TeamStore::open(database).map_err(|e| e.to_string())?;
    let actor = profile(&s)?;
    append(
        &mut s,
        &id,
        team,
        &actor,
        EventType::PermissionRevoked,
        json!({"grant_id":grant}),
    )
    .map(|_| ())
}
pub(crate) fn decide_and_run(
    database: &Path,
    identity: &Path,
    team: &str,
    request_id: &str,
    decision: QuestionDecision,
    confirm_limited_isolation: bool,
) -> Result<String, String> {
    let id = DeviceIdentity::load_existing(identity).map_err(|e| e.to_string())?;
    let mut s = TeamStore::open(database).map_err(|e| e.to_string())?;
    let actor = profile(&s)?;
    let request = s
        .request(request_id)
        .map_err(|e| e.to_string())?
        .ok_or("Question not found")?;
    if request.target_member_id != actor
        || !matches!(request.kind, RequestKind::Check | RequestKind::Read)
    {
        return Err("Only the selected project owner can answer this question".into());
    }
    if !matches!(
        request.delivery,
        DeliveryState::Created | DeliveryState::Queued | DeliveryState::Delivered
    ) {
        return Err("This question has already been handled".into());
    }
    let (repo, question) = payload(&request)?;
    if !s
        .published_projects(team)
        .map_err(|e| e.to_string())?
        .iter()
        .any(|p| p.repository_id == repo && p.owner_member_id == actor)
    {
        return Err("This question does not belong to one of your shared projects".into());
    }
    if decision == QuestionDecision::Deny {
        lifecycle(
            &mut s,
            &id,
            team,
            &actor,
            EventType::RequestCancelled,
            request_id,
            None,
        )?;
        return Ok("Question declined".into());
    }
    let approval = s
        .repository_approval(&repo)
        .map_err(|e| e.to_string())?
        .ok_or("Project folder is not connected on this computer")?;
    if approval.owner_member_id != actor {
        return Err("Project owner does not match this profile".into());
    }
    let folder = std::fs::canonicalize(&approval.local_path)
        .map_err(|_| "The project folder is unavailable")?;
    if !folder.is_dir() {
        return Err("The approved project is not a directory".into());
    }
    valid_question(&question)?;
    let boundary = crate::execution_boundary::assess_check_policy(&folder);
    crate::execution_boundary::authorize_check(&boundary,confirm_limited_isolation).map_err(|_|"Review the limited-isolation warning, then explicitly choose Allow with limited isolation".to_string())?;
    // Verify availability before recording consent or starting a request.
    super::assistant::resolve_codex()?;
    let grant = QuestionGrant {
        grant_id: format!("consent-{}", super::team::generate_token()),
        subject_member_id: request.requester_member_id.clone(),
        repository_id: repo.clone(),
        owner_member_id: actor.clone(),
        request_id: (decision == QuestionDecision::AllowOnce).then(|| request_id.into()),
        expires_at: None,
    };
    append(
        &mut s,
        &id,
        team,
        &actor,
        EventType::PermissionGranted,
        serde_json::to_value(grant).map_err(|e| e.to_string())?,
    )?;
    if request.delivery == DeliveryState::Created {
        lifecycle(
            &mut s,
            &id,
            team,
            &actor,
            EventType::RequestQueued,
            request_id,
            None,
        )?;
    }
    if request.delivery != DeliveryState::Delivered {
        lifecycle(
            &mut s,
            &id,
            team,
            &actor,
            EventType::RequestDelivered,
            request_id,
            None,
        )?;
    }
    // Reload/re-evaluate immediately before execution; expired or revoked consent fails closed.
    crate::outbox::authorize_request_execution(&s, team, request_id, &repo, now())
        .map_err(|e| e.to_string())?;
    lifecycle(
        &mut s,
        &id,
        team,
        &actor,
        EventType::RequestStarted,
        request_id,
        None,
    )?;
    let started = std::time::Instant::now();
    let answer = super::assistant::answer(&folder, &repo, &question);
    let (result, kind) = match &answer {
        Ok(text) => (
            RequestResult {
                isolation: "limited isolation; OS boundary not verified".into(),
                network: boundary.network.label().into(),
                exit_code: Some(0),
                stdout: text.clone(),
                stderr: String::new(),
                output_truncated: false,
                started_at: now(),
                duration_ms: started.elapsed().as_millis(),
                error: None,
            },
            EventType::RequestCompleted,
        ),
        Err(error) => (
            RequestResult {
                isolation: "limited isolation; OS boundary not verified".into(),
                network: boundary.network.label().into(),
                exit_code: None,
                stdout: String::new(),
                stderr: String::new(),
                output_truncated: false,
                started_at: now(),
                duration_ms: started.elapsed().as_millis(),
                error: Some(error.clone()),
            },
            EventType::RequestFailed,
        ),
    };
    lifecycle(&mut s, &id, team, &actor, kind, request_id, Some(result))?;
    answer
}
