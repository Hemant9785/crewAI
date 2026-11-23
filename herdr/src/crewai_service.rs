//! Application operations shared by terminal surfaces. Network and assistant calls
//! must be dispatched away from the terminal input/render thread.
use crate::identity::DeviceIdentity;
use crate::team::{Event, EventType, TaskStatus};
use crate::team_storage::TeamStore;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug)]
pub(crate) struct TeamService {
    database: PathBuf,
    team_id: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SavedOperation {
    pub event_id: String,
    pub item_id: String,
    /// False means the event was saved but its queue bookkeeping failed.
    pub queued: bool,
}

impl TeamService {
    pub(crate) fn setup_profile(
        database: &Path,
        name: &str,
        role: crate::team::Role,
    ) -> Result<crate::team::Member, String> {
        crate::team_session::setup_profile(database, name, role)
    }

    /// Prepare a team locally and save its relay credential before any network
    /// operation. The caller can safely retry sync if the relay is unavailable.
    pub(crate) fn create_team(database: &Path, name: &str) -> Result<Self, String> {
        let mut store = TeamStore::open(database).map_err(|e| e.to_string())?;
        if !store.teams().map_err(|e| e.to_string())?.is_empty() {
            return Err(
                "This workspace already has a team; switch teams or invite a teammate".into(),
            );
        }
        let profile = store
            .load_profile()
            .map_err(|e| e.to_string())?
            .ok_or("Enter your name first")?;
        if !profile.member.active {
            return Err("Inactive profiles cannot create a team".into());
        }
        let identity = DeviceIdentity::load_existing(crate::team_session::identity_path(database))
            .map_err(|e| e.to_string())?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_secs();
        let team = crate::team::Team::create(name, &profile.member.member_id, now)
            .map_err(|e| e.to_string())?;
        let token = crate::cli::team::generate_token();
        let endpoint = std::env::var("HERDR_RELAY_URL")
            .unwrap_or_else(|_| crate::cli::relay::DEFAULT_RELAY_URL.into());
        crate::team_session::save_connection(database, &endpoint, &token)?;
        store.save_team(&team).map_err(|e| e.to_string())?;
        store
            .save_member(&team.team_id, &profile.member, now)
            .map_err(|e| e.to_string())?;
        store
            .save_device(
                &crate::team::Device {
                    device_id: identity.device_id().into(),
                    member_id: profile.member.member_id.clone(),
                    public_key: identity.public_key(),
                    active: true,
                },
                now,
            )
            .map_err(|e| e.to_string())?;
        let mut event=Event::create(&team.team_id,&profile.member.member_id,identity.device_id(),EventType::TeamCreated,
            json!({"team_id":team.team_id,"name":team.name,"authority_member_id":team.authority_member_id,"created_at":team.created_at}),now,"pending").map_err(|e|e.to_string())?;
        event.signature = Some(identity.sign(&event.signing_bytes()));
        store.insert_event(&event, now).map_err(|e| e.to_string())?;
        Ok(Self {
            database: database.to_owned(),
            team_id: team.team_id,
        })
    }

    pub(crate) fn preview_invite(code: &str) -> Result<crate::cli::team::InviteBundle, String> {
        let code = code.trim();
        let endpoint = std::env::var("HERDR_RELAY_URL")
            .unwrap_or_else(|_| crate::cli::relay::DEFAULT_RELAY_URL.into());
        let signed = if crate::cli::relay::is_short_invite_code(code) {
            crate::cli::relay::claim_invite_code(&endpoint, code)?
        } else {
            code.to_owned()
        };
        let bundle = crate::cli::team::decode_invite_code(&signed).map_err(|e| e.to_string())?;
        crate::cli::team::validate_invite_for_existing_team(
            &bundle,
            &bundle.invitation.team_id,
            &bundle.invitation.authority_member_id,
        )
        .map_err(|e| e.to_string())?;
        Ok(bundle)
    }

    pub(crate) fn join(
        base: &Path,
        current: &Path,
        name: &str,
        bundle: &crate::cli::team::InviteBundle,
    ) -> Result<PathBuf, String> {
        let dest = crate::team_session::prepare_join_database(base, current, bundle, name)?;
        let identity = crate::team_session::identity_path(&dest);
        let existing = TeamStore::open(&dest)
            .map_err(|e| e.to_string())?
            .team(&bundle.invitation.team_id)
            .map_err(|e| e.to_string())?
            .is_some();
        let request = if existing {
            crate::cli::join_inbox::reconnect_request(&dest, &identity, bundle)?
        } else {
            Some(
                crate::cli::team::accept_invite_bundle(&dest, &identity, bundle)
                    .map_err(|e| e.to_string())?,
            )
        };
        if let Some(request) = request {
            crate::cli::join_inbox::submit(&bundle.relay_url, &bundle.relay_token, &request)?;
        }
        crate::team_session::save_connection(&dest, &bundle.relay_url, &bundle.relay_token)?;
        crate::team_session::activate_database(base, &dest)?;
        Ok(dest)
    }
    pub(crate) fn open(database: impl AsRef<Path>, team_id: &str) -> Result<Self, String> {
        let db = database.as_ref();
        if !db.is_file() {
            return Err("Create or join this team first".into());
        }
        let store = TeamStore::open(db).map_err(|e| e.to_string())?;
        if store.team(team_id).map_err(|e| e.to_string())?.is_none() {
            return Err("Create or join this team first".into());
        }
        if store.load_profile().map_err(|e| e.to_string())?.is_none() {
            return Err("Set up your profile first".into());
        }
        Ok(Self {
            database: db.to_owned(),
            team_id: team_id.into(),
        })
    }
    pub(crate) fn teammates(&self) -> Result<Vec<crate::team::Member>, String> {
        TeamStore::open_read_only(&self.database)
            .map_err(|e| e.to_string())?
            .members_for_team(&self.team_id)
            .map_err(|e| e.to_string())
    }
    fn append(
        &self,
        kind: EventType,
        payload: Value,
        item_id: String,
    ) -> Result<SavedOperation, String> {
        let mut store = TeamStore::open(&self.database).map_err(|e| e.to_string())?;
        let profile = store
            .load_profile()
            .map_err(|e| e.to_string())?
            .ok_or("Set up your profile first")?;
        let identity =
            DeviceIdentity::load_existing(crate::team_session::identity_path(&self.database))
                .map_err(|e| e.to_string())?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_secs();
        let mut event = Event::create(
            &self.team_id,
            &profile.member.member_id,
            identity.device_id(),
            kind,
            payload,
            now,
            "pending",
        )
        .map_err(|e| e.to_string())?;
        event.signature = Some(identity.sign(&event.signing_bytes()));
        store
            .validate_verified_event(&event)
            .map_err(|e| e.to_string())?;
        store.insert_event(&event, now).map_err(|e| e.to_string())?;
        // The signed event is already durable. A queue bookkeeping failure must not
        // encourage a user to retry and accidentally create a duplicate task.
        let queued = crate::outbox::queue_event(&store, &event.event_id, now).is_ok();
        Ok(SavedOperation {
            event_id: event.event_id,
            item_id,
            queued,
        })
    }
    pub(crate) fn assign_task(
        &self,
        recipient_id: &str,
        description: &str,
    ) -> Result<SavedOperation, String> {
        let store = TeamStore::open_read_only(&self.database).map_err(|e| e.to_string())?;
        let profile = store
            .load_profile()
            .map_err(|e| e.to_string())?
            .ok_or("Set up your profile first")?;
        let task_id = format!("task-{}", crate::cli::team::generate_token());
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_secs();
        self.append(EventType::TaskCreated,json!({"task_id":task_id,"assigner_member_id":profile.member.member_id,"assignee_member_id":recipient_id,"description":description,"created_at":now}),task_id)
    }
    pub(crate) fn update_task(
        &self,
        task_id: &str,
        status: TaskStatus,
    ) -> Result<SavedOperation, String> {
        self.append(
            EventType::TaskStatusChanged,
            json!({"task_id":task_id,"status":status}),
            task_id.into(),
        )
    }
    pub(crate) fn follow_up(&self, task_id: &str, message: &str) -> Result<SavedOperation, String> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_secs();
        self.append(
            EventType::TaskFollowUpAdded,
            json!({"task_id":task_id,"message":message,"created_at":now}),
            task_id.into(),
        )
    }
    pub(crate) fn broadcast(&self, message: &str) -> Result<SavedOperation, String> {
        crate::team::validate_broadcast_message(message).map_err(|e| e.to_string())?;
        let store = TeamStore::open_read_only(&self.database).map_err(|e| e.to_string())?;
        let profile = store
            .load_profile()
            .map_err(|e| e.to_string())?
            .ok_or("Set up your profile first")?;
        let message_id = format!("message-{}", crate::cli::team::generate_token());
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_secs();
        self.append(EventType::BroadcastCreated,json!({"message_id":message_id,"sender_member_id":profile.member.member_id,"scope":"team","message":message,"created_at":now}),message_id)
    }

    pub(crate) fn send_direct(
        &self,
        recipient_id: &str,
        reply_to: Option<String>,
        body: &str,
    ) -> Result<crate::crewai_message::DirectMessage, String> {
        let store = TeamStore::open(&self.database).map_err(|e| e.to_string())?;
        let profile = store
            .load_profile()
            .map_err(|e| e.to_string())?
            .ok_or("Set up your profile first")?;
        let identity =
            DeviceIdentity::load_existing(crate::team_session::identity_path(&self.database))
                .map_err(|e| e.to_string())?;
        let at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_secs();
        let message = crate::crewai_message::DirectMessage::create(
            &self.team_id,
            &profile.member.member_id,
            recipient_id,
            reply_to,
            body,
            at,
            &identity,
        )?;
        store
            .insert_direct_message(&message)
            .map_err(|e| e.to_string())?;
        Ok(message)
    }

    pub(crate) fn ask_remote(
        &self,
        recipient_id: &str,
        repository_id: &str,
        question: &str,
    ) -> Result<String, String> {
        crate::cli::questions::ask(
            &self.database,
            &crate::team_session::identity_path(&self.database),
            &self.team_id,
            recipient_id,
            repository_id,
            question,
        )
    }

    pub(crate) fn projects(
        &self,
        recipient_id: &str,
    ) -> Result<Vec<crate::cli::questions::Project>, String> {
        crate::cli::questions::list_projects(&self.database, &self.team_id, recipient_id)
    }

    pub(crate) fn questions(&self) -> Result<Vec<crate::cli::questions::Question>, String> {
        let store = TeamStore::open_read_only(&self.database).map_err(|e| e.to_string())?;
        let requests = store
            .team_events(&self.team_id, 20_000)
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|event| event.event_type == EventType::RequestCreated)
            .filter_map(|event| {
                event
                    .payload
                    .get("request_id")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned)
            })
            .collect::<std::collections::HashSet<_>>();
        Ok(crate::cli::questions::pending_questions(&self.database)?
            .into_iter()
            .filter(|q| requests.contains(&q.request_id))
            .collect())
    }

    pub(crate) fn conversation(
        &self,
    ) -> Result<Vec<crate::crewai_conversation::ConversationEntry>, String> {
        self.conversation_for_thread("main")
    }

    pub(crate) fn conversation_for_thread(
        &self,
        thread: &str,
    ) -> Result<Vec<crate::crewai_conversation::ConversationEntry>, String> {
        let store = TeamStore::open_read_only(&self.database).map_err(|e| e.to_string())?;
        let member = store
            .load_profile()
            .map_err(|e| e.to_string())?
            .ok_or("Set up your profile first")?
            .member
            .member_id;
        let events = store
            .team_events(&self.team_id, 20_000)
            .map_err(|e| e.to_string())?;
        let direct = store
            .direct_messages(&self.team_id)
            .map_err(|e| e.to_string())?;
        let mut reconstructed = crate::crewai_conversation::reconstruct(
            &self.team_id,
            &member,
            &events,
            &self.questions()?,
            &direct,
        );
        if thread != "main" {
            let mut related = std::collections::HashSet::from([thread.to_owned()]);
            loop {
                let before = related.len();
                for entry in &reconstructed {
                    if entry
                        .related_id
                        .as_ref()
                        .is_some_and(|id| related.contains(id))
                    {
                        related.insert(entry.entry_id.clone());
                    }
                }
                if related.len() == before {
                    break;
                }
            }
            reconstructed.retain(|entry| related.contains(&entry.entry_id));
            for entry in &mut reconstructed {
                entry.thread_id = thread.into();
            }
        }
        let mut entries = store
            .conversation_history(&self.team_id, thread)
            .map_err(|e| e.to_string())?;
        let rebuilt_ids = reconstructed
            .iter()
            .map(|entry| entry.entry_id.as_str())
            .collect::<std::collections::HashSet<_>>();
        entries.retain(|entry| !rebuilt_ids.contains(entry.entry_id.as_str()));
        entries.extend(reconstructed);
        crate::crewai_conversation::sort_entries(&mut entries);
        if entries.len() > 500 {
            entries.drain(..entries.len() - 500);
        }
        Ok(entries)
    }

    pub(crate) fn decide_question(
        &self,
        request_id: &str,
        decision: crate::cli::questions::QuestionDecision,
        confirm_limited_isolation: bool,
    ) -> Result<String, String> {
        crate::cli::questions::decide_and_run(
            &self.database,
            &crate::team_session::identity_path(&self.database),
            &self.team_id,
            request_id,
            decision,
            confirm_limited_isolation,
        )
    }

    pub(crate) fn join_requests(&self) -> Result<crate::cli::join_inbox::Inbox, String> {
        let connection = crate::team_session::connection_from_env_or_store(&self.database)?
            .ok_or("Reconnect this team before reviewing requests")?;
        crate::cli::join_inbox::command(
            &connection.endpoint,
            &self.team_id,
            &connection.token,
            &crate::team_session::identity_path(&self.database),
            "list",
            "",
            "",
        )
    }

    pub(crate) fn answer_local(
        &self,
        repository_id: &str,
        question: &str,
        confirm_limited_isolation: bool,
    ) -> Result<String, String> {
        let store = TeamStore::open_read_only(&self.database).map_err(|e| e.to_string())?;
        let profile = store
            .load_profile()
            .map_err(|e| e.to_string())?
            .ok_or("Set up your profile first")?;
        let project = store
            .repository_approval(repository_id)
            .map_err(|e| e.to_string())?
            .ok_or("Connect a project first")?;
        if project.owner_member_id != profile.member.member_id {
            return Err("Only the project owner can run a local question".into());
        }
        let policy = crate::execution_boundary::assess_check_policy(Path::new(&project.local_path));
        crate::execution_boundary::authorize_check(&policy, confirm_limited_isolation).map_err(
            |_| "Confirm the limited-isolation warning before starting Codex".to_string(),
        )?;
        crate::cli::read::answer_local(&self.database, repository_id, question)
    }

    pub(crate) fn connect_project(
        &self,
        folder: &Path,
    ) -> Result<crate::team::RepositoryApproval, String> {
        let approval = crate::team_session::link_project(&self.database, folder)?;
        crate::cli::questions::publish_local_projects(
            &self.database,
            &crate::team_session::identity_path(&self.database),
        )?;
        Ok(approval)
    }

    pub(crate) fn sync(&self) -> Result<bool, String> {
        crate::cli::relay::auto_sync_after_write(&self.database, &self.team_id)
    }

    pub(crate) fn start_sync(&self) -> BackgroundOperation<bool> {
        let service = self.clone();
        dispatch(move || service.sync())
    }

    pub(crate) fn start_local_answer(
        &self,
        repository_id: String,
        question: String,
        confirm_limited_isolation: bool,
    ) -> BackgroundOperation<String> {
        let service = self.clone();
        dispatch(move || service.answer_local(&repository_id, &question, confirm_limited_isolation))
    }

    pub(crate) fn start_question_decision(
        &self,
        request_id: String,
        decision: crate::cli::questions::QuestionDecision,
        confirm_limited_isolation: bool,
    ) -> BackgroundOperation<String> {
        let service = self.clone();
        dispatch(move || service.decide_question(&request_id, decision, confirm_limited_isolation))
    }

    /// Creates a one-use short invitation. The long invitation and relay token
    /// stay inside this operation and are never returned to the renderer.
    pub(crate) fn invite(&self) -> Result<String, String> {
        let connection = crate::team_session::connection_from_env_or_store(&self.database)?
            .ok_or("Reconnect this team before inviting someone")?;
        let bundle = crate::cli::team::create_invite_bundle_with_credentials(
            &self.database,
            &self.team_id,
            &crate::team_session::identity_path(&self.database),
            &connection.endpoint,
            &connection.token,
        )
        .map_err(|e| e.to_string())?;
        let signed = crate::cli::team::encode_invite_code(&bundle).map_err(|e| e.to_string())?;
        crate::cli::relay::publish_invite_code(
            &connection.endpoint,
            &self.team_id,
            &connection.token,
            &signed,
        )
    }

    pub(crate) fn decide_join(
        &self,
        request: &crate::cli::team::JoinRequest,
        approve: bool,
    ) -> Result<(), String> {
        if request.team_id != self.team_id {
            return Err("Join request belongs to another team".into());
        }
        let connection = crate::team_session::connection_from_env_or_store(&self.database)?
            .ok_or("Reconnect this team before reviewing requests")?;
        crate::cli::join_inbox::decide(
            &self.database,
            &crate::team_session::identity_path(&self.database),
            &connection.endpoint,
            &connection.token,
            request,
            approve,
        )
    }
}

/// A bounded, correlated result channel for work that can block on a network or assistant.
pub(crate) struct BackgroundOperation<T> {
    pub operation_id: String,
    pub updates: std::sync::mpsc::Receiver<OperationUpdate<T>>,
}

pub(crate) enum OperationUpdate<T> {
    Started,
    Finished(Result<T, String>),
}

pub(crate) fn dispatch<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> BackgroundOperation<T> {
    let operation_id = format!("operation-{}", crate::cli::team::generate_token());
    let (sender, updates) = std::sync::mpsc::sync_channel(2);
    std::thread::spawn(move || {
        let _ = sender.send(OperationUpdate::Started);
        let _ = sender.send(OperationUpdate::Finished(work()));
    });
    BackgroundOperation {
        operation_id,
        updates,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::team::{Device, Member, Role, Team};

    #[test]
    #[ignore = "requires Internet access to the configured prototype relay"]
    fn live_service_join_approval_and_team_delivery() {
        let root = std::env::temp_dir().join(format!(
            "crewai-live-service-{}",
            crate::cli::team::generate_token()
        ));
        let manager_db = root.join("manager/team.sqlite");
        let member_db = root.join("member/team.sqlite");
        TeamService::setup_profile(&manager_db, "Manager", Role::Manager).unwrap();
        TeamService::setup_profile(&member_db, "Teammate", Role::Implementer).unwrap();
        let manager = TeamService::create_team(&manager_db, "Service test").unwrap();
        assert!(manager.sync().unwrap());
        let endpoint = std::env::var("HERDR_RELAY_URL").unwrap();
        let first = TeamStore::open_read_only(&manager_db).unwrap();
        let initial_events = first.team_events(&manager.team_id, 100).unwrap();
        assert!(!initial_events.is_empty());
        let connection = crate::team_session::load_connection(&manager_db)
            .unwrap()
            .unwrap();
        let (resent, _) = crate::cli::relay::sync_team(
            &manager_db,
            &manager.team_id,
            &connection.endpoint,
            &connection.token,
        )
        .unwrap();
        assert_eq!(resent, 0, "an idle sync must not re-upload history");
        for event in &initial_events {
            assert!(first
                .relay_knows_event(&manager.team_id, &endpoint, &event.event_id)
                .unwrap());
            assert!(!first
                .relay_knows_event(
                    &manager.team_id,
                    "https://different.example",
                    &event.event_id
                )
                .unwrap());
        }
        let writable = TeamStore::open(&manager_db).unwrap();
        writable
            .remember_relay_event("wrong-team", &endpoint, &initial_events[0].event_id)
            .unwrap();
        assert!(!writable
            .relay_knows_event("wrong-team", &endpoint, &initial_events[0].event_id)
            .unwrap());
        let code = manager.invite().unwrap();
        let bundle = TeamService::preview_invite(&code).unwrap();
        let joined = TeamService::join(&member_db, &member_db, "Teammate", &bundle).unwrap();
        assert_eq!(joined, member_db);
        let requests = manager.join_requests().unwrap();
        assert_eq!(requests.requests.len(), 1);
        manager
            .decide_join(&requests.requests[0].request, true)
            .unwrap();
        assert!(manager.sync().unwrap());
        let member = TeamService::open(&member_db, &manager.team_id).unwrap();
        assert!(member.sync().unwrap());
        let recipient = member
            .teammates()
            .unwrap()
            .into_iter()
            .find(|p| p.display_name == "Teammate")
            .unwrap();
        let task = manager
            .assign_task(&recipient.member_id, "Test assignment")
            .unwrap();
        let message = manager.broadcast("Test message").unwrap();
        let unsynced = TeamStore::open_read_only(&manager_db)
            .unwrap()
            .team_events(&manager.team_id, 100)
            .unwrap();
        assert!(unsynced
            .iter()
            .any(|event| !TeamStore::open_read_only(&manager_db)
                .unwrap()
                .relay_knows_event(&manager.team_id, &endpoint, &event.event_id)
                .unwrap()));
        assert!(manager.sync().unwrap());
        assert!(member.sync().unwrap());
        let store = TeamStore::open_read_only(&member_db).unwrap();
        assert_eq!(
            store.task(&task.item_id).unwrap().unwrap().description,
            "Test assignment"
        );
        assert_eq!(
            store.broadcast(&message.item_id).unwrap().unwrap().message,
            "Test message"
        );
        let direct = manager
            .send_direct(&recipient.member_id, None, "Private hello")
            .unwrap();
        let queued = TeamStore::open_read_only(&manager_db)
            .unwrap()
            .direct_messages(&manager.team_id)
            .unwrap();
        assert!(queued
            .iter()
            .any(|(m, state)| m.message_id == direct.message_id && state == "queued"));
        let restarted = TeamService::open(&manager_db, &manager.team_id).unwrap();
        assert!(restarted.sync().unwrap());
        assert!(member.sync().unwrap());
        let received = TeamStore::open_read_only(&member_db)
            .unwrap()
            .direct_messages(&manager.team_id)
            .unwrap();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].0.body, "Private hello");
        let owner = manager
            .teammates()
            .unwrap()
            .into_iter()
            .find(|p| p.display_name == "Manager")
            .unwrap();
        member
            .send_direct(
                &owner.member_id,
                Some(direct.message_id.clone()),
                "Private reply",
            )
            .unwrap();
        assert!(member.sync().unwrap());
        assert!(manager.sync().unwrap());
        let messages = TeamStore::open_read_only(&manager_db)
            .unwrap()
            .direct_messages(&manager.team_id)
            .unwrap();
        assert_eq!(messages.len(), 2);
        assert!(messages.iter().any(|(m, _)| m.body == "Private reply"
            && m.reply_to.as_deref() == Some(direct.message_id.as_str())));
        let history = manager.conversation().unwrap();
        assert!(history.iter().any(|entry| entry.text == "Test assignment"
            && entry.kind == crate::crewai_conversation::ConversationKind::Task));
        assert!(history.iter().any(|entry| entry.text == "Test message"
            && entry.kind == crate::crewai_conversation::ConversationKind::Broadcast));
        assert!(history.iter().any(|entry| entry.text == "Private reply"
            && entry.kind == crate::crewai_conversation::ConversationKind::Message
            && entry.related_id.as_deref()
                == Some(format!("message:{}", direct.message_id).as_str())));
        let thread = manager
            .conversation_for_thread(&format!("message:{}", direct.message_id))
            .unwrap();
        assert!(thread.iter().any(|entry| entry.text == "Private hello"));
        assert!(thread.iter().any(|entry| entry.text == "Private reply"));
        assert!(!thread.iter().any(|entry| entry.text == "Test message"));
        let project_dir = root.join("member-project");
        std::fs::create_dir_all(&project_dir).unwrap();
        member.connect_project(&project_dir).unwrap();
        assert!(member.sync().unwrap());
        assert!(manager.sync().unwrap());
        let project = manager
            .projects(&recipient.member_id)
            .unwrap()
            .into_iter()
            .next()
            .unwrap();
        let first_question = manager
            .ask_remote(
                &recipient.member_id,
                &project.repository_id,
                "First question",
            )
            .unwrap();
        let second_question = manager
            .ask_remote(
                &recipient.member_id,
                &project.repository_id,
                "Second question",
            )
            .unwrap();
        assert!(manager.sync().unwrap());
        assert!(member.sync().unwrap());
        assert!(member
            .decide_question(
                &second_question,
                crate::cli::questions::QuestionDecision::Deny,
                false
            )
            .is_ok());
        assert!(member.sync().unwrap());
        assert!(manager.sync().unwrap());
        let questions = manager.questions().unwrap();
        assert_eq!(
            questions
                .iter()
                .find(|q| q.request_id == first_question)
                .unwrap()
                .delivery,
            "Queued"
        );
        assert_eq!(
            questions
                .iter()
                .find(|q| q.request_id == second_question)
                .unwrap()
                .delivery,
            "Cancelled"
        );
    }

    #[test]
    fn team_setup_keeps_credentials_and_rejects_accidental_replacement() {
        let root = std::env::temp_dir().join(format!(
            "crewai-create-{}",
            crate::cli::team::generate_token()
        ));
        let db = root.join("team.sqlite");
        TeamService::setup_profile(&db, "Manager", Role::Manager).unwrap();
        let service = TeamService::create_team(&db, "First").unwrap();
        let first = TeamStore::open_read_only(&db)
            .unwrap()
            .team(&service.team_id)
            .unwrap()
            .unwrap();
        assert_eq!(first.name, "First");
        assert!(crate::team_session::load_connection(&db).unwrap().is_some());
        let project_dir = root.join("project");
        std::fs::create_dir_all(&project_dir).unwrap();
        let project = crate::team_session::link_project(&db, &project_dir).unwrap();
        assert!(service
            .answer_local(&project.repository_id, "What is this?", false)
            .unwrap_err()
            .contains("Confirm the limited-isolation"));
        assert!(TeamService::create_team(&db, "Second").is_err());
        assert_eq!(
            TeamStore::open_read_only(&db)
                .unwrap()
                .teams()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn background_results_are_correlated_and_do_not_block_the_caller() {
        let first = dispatch(|| Ok::<_, String>("first"));
        let second = dispatch(|| Err::<&str, _>("failure".to_owned()));
        assert_ne!(first.operation_id, second.operation_id);
        assert!(matches!(
            first
                .updates
                .recv_timeout(std::time::Duration::from_secs(3))
                .unwrap(),
            OperationUpdate::Started
        ));
        assert!(matches!(
            first
                .updates
                .recv_timeout(std::time::Duration::from_secs(3))
                .unwrap(),
            OperationUpdate::Finished(Ok("first"))
        ));
        assert!(matches!(
            second
                .updates
                .recv_timeout(std::time::Duration::from_secs(3))
                .unwrap(),
            OperationUpdate::Started
        ));
        assert!(
            matches!(second.updates.recv_timeout(std::time::Duration::from_secs(3)).unwrap(),OperationUpdate::Finished(Err(error)) if error=="failure")
        );
    }
    #[test]
    fn signed_tasks_and_broadcasts_are_saved_and_enforce_authority() {
        let root = std::env::temp_dir().join(format!(
            "crewai-service-{}",
            crate::cli::team::generate_token()
        ));
        std::fs::create_dir(&root).unwrap();
        let db = root.join("team.sqlite");
        let owner = crate::team_session::setup_profile(&db, "Manager", Role::Manager).unwrap();
        let owner_key =
            DeviceIdentity::load_existing(crate::team_session::identity_path(&db)).unwrap();
        let peer_key = DeviceIdentity::load_or_create(root.join("peer.identity.json")).unwrap();
        let peer = Member {
            member_id: "peer".into(),
            display_name: "Peer".into(),
            role: Role::Implementer,
            active: true,
        };
        let team = Team::create("Team", &owner.member_id, 1).unwrap();
        let store = TeamStore::open(&db).unwrap();
        store.save_team(&team).unwrap();
        for (member, key) in [(&owner, &owner_key), (&peer, &peer_key)] {
            store.save_member(&team.team_id, member, 1).unwrap();
            store
                .save_device(
                    &Device {
                        device_id: key.device_id().into(),
                        member_id: member.member_id.clone(),
                        public_key: key.public_key(),
                        active: true,
                    },
                    1,
                )
                .unwrap();
        }
        let svc = TeamService::open(&db, &team.team_id).unwrap();
        let other_team = Team::create("Other", &owner.member_id, 1).unwrap();
        store.save_team(&other_team).unwrap();
        store
            .save_member(
                &other_team.team_id,
                &Member {
                    member_id: "other".into(),
                    display_name: "Other".into(),
                    role: Role::Implementer,
                    active: true,
                },
                1,
            )
            .unwrap();
        assert_eq!(svc.teammates().unwrap().len(), 2);
        assert!(!svc
            .teammates()
            .unwrap()
            .iter()
            .any(|m| m.member_id == "other"));
        let task = svc
            .assign_task(&peer.member_id, "Implement the page")
            .unwrap();
        let message = svc.broadcast("Hello team").unwrap();
        assert!(task.queued && message.queued);
        assert_eq!(
            store.task(&task.item_id).unwrap().unwrap().description,
            "Implement the page"
        );
        assert_eq!(
            store.broadcast(&message.item_id).unwrap().unwrap().message,
            "Hello team"
        );
        assert!(svc
            .assign_task("unknown", "Unauthorized recipient")
            .is_err());
        assert!(svc.broadcast("").is_err());
        drop(store);
        let denied_db = root.join("peer.sqlite");
        std::fs::copy(&db, &denied_db).unwrap();
        // The peer identity was created at the path associated with peer.sqlite.
        let denied_store = TeamStore::open(&denied_db).unwrap();
        denied_store.save_profile(&peer, 1, 1).unwrap();
        let denied = TeamService::open(&denied_db, &team.team_id).unwrap();
        assert!(denied.assign_task(&owner.member_id, "No grant").is_err());
        assert!(denied
            .update_task(&task.item_id, TaskStatus::Delivered)
            .is_ok());
        assert!(denied
            .update_task(&task.item_id, TaskStatus::Queued)
            .is_err());
    }
}
