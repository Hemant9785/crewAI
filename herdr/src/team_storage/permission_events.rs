//! Signed owner-scoped project announcements and consent, projected from the event log.
use super::*;
use crate::team::PermissionDecisionReason;
use serde::Deserialize;
use sha2::{Digest, Sha256};
#[derive(Clone, Debug, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PublishedProject {
    pub repository_id: String,
    pub owner_member_id: String,
    pub label: String,
}
#[derive(Clone, Debug, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct QuestionGrant {
    pub grant_id: String,
    pub subject_member_id: String,
    pub repository_id: String,
    pub owner_member_id: String,
    pub request_id: Option<String>,
    pub expires_at: Option<u64>,
}
pub(crate) fn owner_prefix(owner: &str) -> String {
    format!("project-{:x}-", Sha256::digest(owner.as_bytes()))
}
fn invalid() -> StorageError {
    StorageError::MembershipConflict("project owner or question consent")
}
fn missing_dependency() -> StorageError {
    StorageError::Sql(rusqlite::Error::QueryReturnedNoRows)
}
fn active(c: &Connection, t: &str, m: &str) -> Result<bool, StorageError> {
    Ok(c.query_row(
        "SELECT COUNT(*) FROM team_members WHERE team_id=?1 AND member_id=?2 AND active=1",
        params![t, m],
        |r| r.get::<_, i64>(0),
    )? == 1)
}
fn request_team(c: &Connection, team: &str, request: &str) -> Result<bool, StorageError> {
    Ok(c.query_row("SELECT COUNT(*) FROM event_log WHERE team_id=?1 AND event_type=?2 AND json_extract(payload_json,'$.request_id')=?3",params![team,EventType::RequestCreated.as_str(),request],|r|r.get::<_,i64>(0))?==1)
}
fn values(c: &Connection, t: &str, k: EventType) -> Result<Vec<(String, Value)>, StorageError> {
    let mut q=c.prepare("SELECT event_id,payload_json FROM event_log WHERE team_id=?1 AND event_type=?2 ORDER BY logical_time,event_id")?;
    let rows = q.query_map(params![t, k.as_str()], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?;
    rows.map(|r| {
        let (id, json) = r?;
        Ok((id, serde_json::from_str(&json)?))
    })
    .collect()
}
pub(crate) fn projects(c: &Connection, t: &str) -> Result<Vec<PublishedProject>, StorageError> {
    let mut all = std::collections::BTreeMap::new();
    for (_, p) in values(c, t, EventType::RepositoryApproved)? {
        let p: PublishedProject = serde_json::from_value(p)?;
        all.insert(p.repository_id.clone(), p);
    }
    Ok(all.into_values().collect())
}
pub(super) fn apply(tx: &Transaction<'_>, e: &Event, _at: u64) -> Result<bool, StorageError> {
    match e.event_type {
        EventType::RepositoryApproved => {
            let p: PublishedProject = serde_json::from_value(e.payload.clone())?;
            if p.owner_member_id != e.actor_member_id
                || !p
                    .repository_id
                    .starts_with(&owner_prefix(&p.owner_member_id))
                || p.label.trim().is_empty()
                || p.label.len() > 120
                || p.label.contains(['\n', '\r'])
            {
                return Err(invalid());
            }
            Ok(true)
        }
        EventType::PermissionGranted => {
            let g: QuestionGrant = serde_json::from_value(e.payload.clone())?;
            let owner = projects(tx, &e.team_id)?
                .into_iter()
                .find(|p| p.repository_id == g.repository_id)
                .ok_or_else(missing_dependency)?
                .owner_member_id;
            if owner != e.actor_member_id
                || g.owner_member_id != owner
                || g.grant_id.trim().is_empty()
                || !active(tx, &e.team_id, &g.subject_member_id)?
            {
                return Err(invalid());
            }
            for (id, p) in values(tx, &e.team_id, EventType::PermissionGranted)? {
                if id != e.event_id
                    && p.get("grant_id").and_then(Value::as_str) == Some(g.grant_id.as_str())
                {
                    return Err(invalid());
                }
            }
            if let Some(id) = &g.request_id {
                if !request_team(tx, &e.team_id, id)? {
                    return Err(missing_dependency());
                }
                let(a,b,k,p):(String,String,String,String)=tx.query_row("SELECT requester_member_id,target_member_id,kind_json,payload FROM requests WHERE request_id=?1",params![id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
                let p: Value = serde_json::from_str(&p)?;
                if a != g.subject_member_id
                    || b != owner
                    || !matches!(
                        decode::<RequestKind>(&k)?,
                        RequestKind::Check | RequestKind::Read
                    )
                    || p.get("repository_id").and_then(Value::as_str)
                        != Some(g.repository_id.as_str())
                {
                    return Err(invalid());
                }
            }
            Ok(true)
        }
        EventType::PermissionRevoked => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Revoke {
                grant_id: String,
            }
            let r: Revoke = serde_json::from_value(e.payload.clone())?;
            let g = values(tx, &e.team_id, EventType::PermissionGranted)?
                .into_iter()
                .find_map(|(_, p)| {
                    serde_json::from_value::<QuestionGrant>(p)
                        .ok()
                        .filter(|g| g.grant_id == r.grant_id)
                })
                .ok_or_else(missing_dependency)?;
            if e.actor_member_id != g.owner_member_id {
                return Err(invalid());
            }
            Ok(true)
        }
        EventType::RequestCreated => {
            let p: RequestCreatedPayload = serde_json::from_value(e.payload.clone())?;
            if matches!(p.kind, RequestKind::Check | RequestKind::Read)
                && (p.requester_member_id != e.actor_member_id
                    || !active(tx, &e.team_id, &p.target_member_id)?)
            {
                return Err(invalid());
            }
            Ok(false)
        }
        EventType::RequestQueued
        | EventType::RequestDelivered
        | EventType::RequestStarted
        | EventType::RequestCompleted
        | EventType::RequestFailed
        | EventType::RequestCancelled => {
            let id = e
                .payload
                .get("request_id")
                .and_then(Value::as_str)
                .ok_or_else(invalid)?;
            if !request_team(tx, &e.team_id, id)? {
                return Err(missing_dependency());
            }
            let(a,b,k):(String,String,String)=tx.query_row("SELECT requester_member_id,target_member_id,kind_json FROM requests WHERE request_id=?1",params![id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
            if matches!(
                decode::<RequestKind>(&k)?,
                RequestKind::Check | RequestKind::Read
            ) {
                let allowed = match e.event_type {
                    EventType::RequestQueued | EventType::RequestCancelled => {
                        e.actor_member_id == a || e.actor_member_id == b
                    }
                    _ => e.actor_member_id == b,
                };
                if !allowed {
                    return Err(invalid());
                }
                if let Some(r) = e.payload.get("result").filter(|v| !v.is_null()) {
                    let r: RequestResult = serde_json::from_value(r.clone())?;
                    if r.stdout.len() + r.stderr.len() > 128 * 1024
                        || r.error.as_ref().is_some_and(|s| s.len() > 16 * 1024)
                    {
                        return Err(invalid());
                    }
                }
            }
            Ok(false)
        }
        _ => Ok(false),
    }
}
impl TeamStore {
    pub(crate) fn published_projects(
        &self,
        t: &str,
    ) -> Result<Vec<PublishedProject>, StorageError> {
        projects(&self.connection, t)
    }
    pub(crate) fn question_permission_decision(
        &self,
        t: &str,
        s: &str,
        repo: &str,
        request: Option<&str>,
        at: u64,
    ) -> Result<PermissionDecision, StorageError> {
        let revoked = values(&self.connection, t, EventType::PermissionRevoked)?
            .into_iter()
            .filter_map(|(_, p)| p.get("grant_id").and_then(Value::as_str).map(str::to_owned))
            .collect::<std::collections::HashSet<_>>();
        let mut found = false;
        for (_, p) in values(&self.connection, t, EventType::PermissionGranted)? {
            let g: QuestionGrant = serde_json::from_value(p)?;
            if g.subject_member_id == s
                && g.repository_id == repo
                && (g.request_id.is_none() || g.request_id.as_deref() == request)
            {
                found = true;
                if !revoked.contains(&g.grant_id)
                    && g.expires_at.is_none_or(|e| at < e)
                    && active(&self.connection, t, s)?
                    && active(&self.connection, t, &g.owner_member_id)?
                {
                    return Ok(PermissionDecision {
                        allowed: true,
                        chain: vec![g.grant_id],
                        reason: PermissionDecisionReason::Allowed,
                    });
                }
            }
        }
        if found {
            return Ok(PermissionDecision {
                allowed: false,
                chain: Vec::new(),
                reason: PermissionDecisionReason::GrantRevoked("owner consent".into()),
            });
        }
        let authority = self.team_authority(t)?;
        Ok(self.load_permission_evaluator(t, &authority)?.check(
            s,
            &PermissionResource::Repository(repo.into()),
            &PermissionAction::ReadRepository,
            at,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::team::{Device, Role, Team};
    use serde_json::json;
    fn event(id: &DeviceIdentity, actor: &str, kind: EventType, payload: Value) -> Event {
        let mut e =
            Event::create("team", actor, id.device_id(), kind, payload, 10, "pending").unwrap();
        e.signature = Some(id.sign(&e.signing_bytes()));
        e
    }
    fn setup() -> (TeamStore, DeviceIdentity, DeviceIdentity, String) {
        let root = std::env::temp_dir().join(format!(
            "herdr-consent-{}",
            crate::cli::team::generate_token()
        ));
        std::fs::create_dir(&root).unwrap();
        let owner = DeviceIdentity::load_or_create(root.join("owner.json")).unwrap();
        let asker = DeviceIdentity::load_or_create(root.join("asker.json")).unwrap();
        let s = TeamStore::open(root.join("team.sqlite")).unwrap();
        s.save_team(&Team {
            team_id: "team".into(),
            name: "Test".into(),
            authority_member_id: "asker".into(),
            created_at: 1,
        })
        .unwrap();
        for (member, id) in [("owner", &owner), ("asker", &asker)] {
            s.save_member(
                "team",
                &Member {
                    member_id: member.into(),
                    display_name: member.into(),
                    role: Role::Implementer,
                    active: true,
                },
                1,
            )
            .unwrap();
            s.save_device(
                &Device {
                    device_id: id.device_id().into(),
                    member_id: member.into(),
                    public_key: id.public_key(),
                    active: true,
                },
                1,
            )
            .unwrap();
        }
        let repo = format!("{}demo", owner_prefix("owner"));
        (s, owner, asker, repo)
    }
    fn publish(s: &mut TeamStore, o: &DeviceIdentity, repo: &str) {
        s.insert_verified_event(
            &event(
                o,
                "owner",
                EventType::RepositoryApproved,
                json!({"repository_id":repo,"owner_member_id":"owner","label":"Demo"}),
            ),
            10,
        )
        .unwrap();
    }
    fn request(s: &mut TeamStore, a: &DeviceIdentity, repo: &str, id: &str) {
        s.insert_verified_event(&event(a,"asker",EventType::RequestCreated,json!({"request_id":id,"requester_member_id":"asker","target_member_id":"owner","kind":RequestKind::Check,"payload":json!({"repository_id":repo,"question":"Explain this project"}).to_string(),"created_at":10})),10).unwrap();
    }
    #[test]
    fn linked_projects_are_announced_once_after_join_without_local_paths() {
        let (s, _owner, _asker, _repo) = setup();
        s.save_profile(
            &Member {
                member_id: "owner".into(),
                display_name: "Owner".into(),
                role: Role::Implementer,
                active: true,
            },
            1,
            1,
        )
        .unwrap();
        let root = s.path().parent().unwrap().to_path_buf();
        s.save_repository_approval(&RepositoryApproval {
            repository_id: "local-project".into(),
            owner_member_id: "owner".into(),
            local_path: root.to_string_lossy().into_owned(),
            approved_at: 1,
        })
        .unwrap();
        let identity = root.join("owner.json");
        assert_eq!(
            crate::cli::questions::publish_local_projects(s.path(), &identity).unwrap(),
            1
        );
        assert_eq!(
            crate::cli::questions::publish_local_projects(s.path(), &identity).unwrap(),
            0
        );
        let announcements = values(&s.connection, "team", EventType::RepositoryApproved).unwrap();
        assert_eq!(announcements.len(), 1);
        assert!(announcements[0].1.get("local_path").is_none());
        assert!(!announcements[0]
            .1
            .to_string()
            .contains(&root.to_string_lossy().to_string()));
        assert!(s
            .repository_approval(announcements[0].1["repository_id"].as_str().unwrap())
            .unwrap()
            .is_some());
    }
    #[test]
    fn owner_must_confirm_limited_isolation_and_can_decline_without_codex() {
        let (mut s, owner, asker, repo) = setup();
        publish(&mut s, &owner, &repo);
        request(&mut s, &asker, &repo, "needs-consent");
        s.save_profile(
            &Member {
                member_id: "owner".into(),
                display_name: "Owner".into(),
                role: Role::Implementer,
                active: true,
            },
            1,
            1,
        )
        .unwrap();
        let root = s.path().parent().unwrap().to_path_buf();
        s.save_repository_approval(&RepositoryApproval {
            repository_id: repo,
            owner_member_id: "owner".into(),
            local_path: root.to_string_lossy().into_owned(),
            approved_at: 1,
        })
        .unwrap();
        let db = s.path().to_path_buf();
        let identity = root.join("owner.json");
        let error = crate::cli::questions::decide_and_run(
            &db,
            &identity,
            "team",
            "needs-consent",
            crate::cli::questions::QuestionDecision::AllowOnce,
            false,
        )
        .unwrap_err();
        assert!(error.contains("limited isolation"));
        assert!(values(&s.connection, "team", EventType::PermissionGranted)
            .unwrap()
            .is_empty());
        assert_eq!(
            s.request("needs-consent").unwrap().unwrap().delivery,
            DeliveryState::Created
        );
        crate::cli::questions::decide_and_run(
            &db,
            &identity,
            "team",
            "needs-consent",
            crate::cli::questions::QuestionDecision::Deny,
            false,
        )
        .unwrap();
        assert_eq!(
            s.request("needs-consent").unwrap().unwrap().delivery,
            DeliveryState::Cancelled
        );
        assert!(values(&s.connection, "team", EventType::PermissionGranted)
            .unwrap()
            .is_empty());
    }
    #[test]
    #[ignore = "requires HERDR_CODEX_PATH pointing to the release smoke assistant"]
    fn question_answer_roundtrip_with_native_smoke_assistant() {
        let (mut owner_store, owner, asker, repo) = setup();
        publish(&mut owner_store, &owner, &repo);
        let root = owner_store.path().parent().unwrap().to_path_buf();
        owner_store
            .save_profile(
                &Member {
                    member_id: "owner".into(),
                    display_name: "Owner".into(),
                    role: Role::Implementer,
                    active: true,
                },
                1,
                1,
            )
            .unwrap();
        owner_store
            .save_repository_approval(&RepositoryApproval {
                repository_id: repo.clone(),
                owner_member_id: "owner".into(),
                local_path: root.to_string_lossy().into_owned(),
                approved_at: 1,
            })
            .unwrap();
        let mut peer = TeamStore::open(root.join("peer.sqlite")).unwrap();
        peer.save_team(&owner_store.team("team").unwrap().unwrap())
            .unwrap();
        for (name, key) in [("owner", &owner), ("asker", &asker)] {
            let member = Member {
                member_id: name.into(),
                display_name: name.into(),
                role: Role::Implementer,
                active: true,
            };
            peer.save_member("team", &member, 1).unwrap();
            peer.save_device(
                &Device {
                    device_id: key.device_id().into(),
                    member_id: name.into(),
                    public_key: key.public_key(),
                    active: true,
                },
                1,
            )
            .unwrap();
            if name == "asker" {
                peer.save_profile(&member, 1, 1).unwrap();
            }
        }
        for e in owner_store.team_events("team", 100).unwrap() {
            peer.insert_verified_event(&e, 10).unwrap();
        }
        let request = crate::cli::questions::ask(
            peer.path(),
            &root.join("asker.json"),
            "team",
            "owner",
            &repo,
            "Explain the smoke project",
        )
        .unwrap();
        for e in peer.team_events("team", 100).unwrap() {
            owner_store.insert_verified_event(&e, 11).unwrap();
        }
        let answer = crate::cli::questions::decide_and_run(
            owner_store.path(),
            &root.join("owner.json"),
            "team",
            &request,
            crate::cli::questions::QuestionDecision::AllowOnce,
            true,
        )
        .unwrap();
        assert!(answer.contains("Smoke project answer"));
        for e in owner_store.team_events("team", 100).unwrap() {
            peer.insert_verified_event(&e, 12).unwrap();
        }
        let questions = crate::cli::questions::pending_questions(peer.path()).unwrap();
        let completed = questions.iter().find(|q| q.request_id == request).unwrap();
        assert!(completed
            .result
            .as_ref()
            .unwrap()
            .contains("Smoke project answer"));
        assert_eq!(
            peer.request(&request).unwrap().unwrap().delivery,
            DeliveryState::Completed
        );
    }
    #[test]
    fn consent_is_request_scoped_signed_and_revocable() {
        let (mut s, o, a, repo) = setup();
        publish(&mut s, &o, &repo);
        request(&mut s, &a, &repo, "one");
        request(&mut s, &a, &repo, "two");
        let grant = json!({"grant_id":"grant","subject_member_id":"asker","repository_id":repo,"owner_member_id":"owner","request_id":"one","expires_at":null});
        let wrong = event(&a, "asker", EventType::PermissionGranted, grant.clone());
        assert!(s.insert_verified_event(&wrong, 11).is_err());
        let e = event(&o, "owner", EventType::PermissionGranted, grant);
        assert!(s.insert_verified_event(&e, 11).unwrap());
        assert!(!s.insert_verified_event(&e, 11).unwrap());
        assert!(
            s.question_permission_decision("team", "asker", &repo, Some("one"), 12)
                .unwrap()
                .allowed
        );
        assert!(
            !s.question_permission_decision("team", "asker", &repo, Some("two"), 12)
                .unwrap()
                .allowed
        );
        assert!(
            !s.permission_decision(
                "team",
                "asker",
                &PermissionResource::Repository(repo.clone()),
                &PermissionAction::ReadRepository,
                12
            )
            .unwrap()
            .allowed
        );
        let revoke = event(
            &o,
            "owner",
            EventType::PermissionRevoked,
            json!({"grant_id":"grant"}),
        );
        s.insert_verified_event(&revoke, 13).unwrap();
        assert!(
            !s.question_permission_decision("team", "asker", &repo, Some("one"), 14)
                .unwrap()
                .allowed
        );
    }
    #[test]
    fn malformed_owner_and_out_of_order_grant_are_rejected() {
        let (mut s, o, a, repo) = setup();
        let forged = event(
            &a,
            "asker",
            EventType::RepositoryApproved,
            json!({"repository_id":repo,"owner_member_id":"owner","label":"Demo"}),
        );
        assert!(s.insert_verified_event(&forged, 1).is_err());
        let early = event(
            &o,
            "owner",
            EventType::PermissionGranted,
            json!({"grant_id":"early","subject_member_id":"asker","repository_id":repo,"owner_member_id":"owner","request_id":null,"expires_at":null}),
        );
        assert!(s.insert_verified_event(&early, 2).is_err());
        publish(&mut s, &o, &repo);
        let mut bad = event(
            &o,
            "owner",
            EventType::PermissionRevoked,
            json!({"grant_id":"missing"}),
        );
        bad.signature = Some("forged".into());
        assert!(s.insert_event(&bad, 3).is_err());
        let lifecycle = event(
            &a,
            "asker",
            EventType::RequestCompleted,
            json!({"request_id":"missing","result":null}),
        );
        assert!(s.insert_verified_event(&lifecycle, 3).is_err());
    }
    #[test]
    fn project_consent_expires_and_cannot_be_changed_by_replay() {
        let (mut s, o, _a, repo) = setup();
        publish(&mut s, &o, &repo);
        let payload = json!({"grant_id":"project","subject_member_id":"asker","repository_id":repo,"owner_member_id":"owner","request_id":null,"expires_at":20});
        s.insert_verified_event(
            &event(&o, "owner", EventType::PermissionGranted, payload.clone()),
            11,
        )
        .unwrap();
        assert!(
            s.question_permission_decision("team", "asker", &repo, Some("any"), 19)
                .unwrap()
                .allowed
        );
        assert!(
            !s.question_permission_decision("team", "asker", &repo, Some("any"), 20)
                .unwrap()
                .allowed
        );
        assert!(s
            .insert_verified_event(
                &event(&o, "owner", EventType::PermissionGranted, payload),
                12
            )
            .is_err());
    }
}
