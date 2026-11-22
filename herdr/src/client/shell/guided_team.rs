//! Guided team actions. All network/assistant work runs outside the render loop.
use super::*;
use crate::team::Role;

impl ClientShellState {
    pub(super) fn guided_dashboard_key(
        &mut self,
        key: &crate::input::TerminalKey,
        outcome: &mut ClientShellInput,
    ) -> bool {
        use ClientTeamDashboardAction as A;
        let Some(ClientShellOverlay::TeamDashboard(d)) = self.overlay.as_mut() else {
            return false;
        };
        if d.guided.consent.is_some() {
            match key.code {
                KeyCode::Up => d.guided.consent_scroll = d.guided.consent_scroll.saturating_sub(1),
                KeyCode::Down => {
                    d.guided.consent_scroll = d.guided.consent_scroll.saturating_add(1)
                }
                KeyCode::PageUp => {
                    d.guided.consent_scroll = d.guided.consent_scroll.saturating_sub(10)
                }
                KeyCode::PageDown => {
                    d.guided.consent_scroll = d.guided.consent_scroll.saturating_add(10)
                }
                _ => {}
            }
            let decision = match key.code {
                KeyCode::Char('1') => Some(crate::cli::questions::QuestionDecision::AllowOnce),
                KeyCode::Char('2') => Some(crate::cli::questions::QuestionDecision::AllowProject),
                KeyCode::Char('x') => Some(crate::cli::questions::QuestionDecision::Deny),
                KeyCode::Esc => {
                    d.guided.consent = None;
                    None
                }
                _ => None,
            };
            if let Some(decision) = decision {
                self.run_question_decision(decision);
            }
            outcome.repaint = true;
            return true;
        }
        if let Some((_, _, scroll)) = d.guided.details.as_mut() {
            match key.code {
                KeyCode::Esc => d.guided.details = None,
                KeyCode::Up | KeyCode::Char('k') => *scroll = scroll.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => *scroll = scroll.saturating_add(1),
                KeyCode::PageUp => *scroll = scroll.saturating_sub(12),
                KeyCode::PageDown => *scroll = scroll.saturating_add(12),
                KeyCode::Home => *scroll = 0,
                _ => {}
            }
            outcome.repaint = true;
            return true;
        }
        if d.guided.invite.is_some() {
            match key.code {
                KeyCode::Esc => {
                    d.guided.invite = None;
                    d.status = "Join cancelled. Your existing team is unchanged.".into();
                }
                KeyCode::Enter if d.pending.is_none() => {
                    let _ = self.start_team_dashboard_action(A::AcceptInvite, Vec::new());
                }
                _ => {}
            }
            outcome.repaint = true;
            return true;
        }
        if d.form.is_some() {
            return false;
        }
        let action = match key.code {
            KeyCode::Char('[') | KeyCode::Char(']')
                if d.section == ClientTeamDashboardSection::Requests =>
            {
                d.guided.question_cursor = if key.code == KeyCode::Char(']') {
                    (d.guided.question_cursor + 1).min(d.guided.questions.len().saturating_sub(1))
                } else {
                    d.guided.question_cursor.saturating_sub(1)
                };
                outcome.repaint = true;
                return true;
            }
            KeyCode::Enter if d.section == ClientTeamDashboardSection::Requests => {
                if let Some(q) = d.guided.questions.get(d.guided.question_cursor) {
                    let me = d
                        .snapshot
                        .as_ref()
                        .ok()
                        .and_then(|s| s.profile.as_ref())
                        .map(|p| p.member_id.as_str());
                    if let Some(answer) = q.result.as_ref() {
                        d.guided.details = Some(("Question answer".into(), answer.clone(), 0));
                    } else if Some(q.target_member_id.as_str()) == me {
                        let warning = crate::cli::questions::policy(
                            &crate::team_view::default_database_path(),
                            &q.repository_id,
                        )
                        .map(|p| p.warning)
                        .unwrap_or_else(|e| e);
                        d.guided.consent = Some((
                            q.request_id.clone(),
                            format!("{}\n\n{}", q.question, warning),
                        ));
                        d.guided.consent_scroll = 0;
                    } else {
                        d.status = format!("Question: {}. Status: {}", q.question, q.delivery);
                    }
                }
                outcome.repaint = true;
                return true;
            }
            KeyCode::Char('/') => Some(A::Ask),
            KeyCode::Char('o') => Some(A::Project),
            KeyCode::Char('g') => Some(A::SwitchTeam),
            KeyCode::Char('f') if d.section == ClientTeamDashboardSection::Tasks => {
                Some(A::TaskFollowUp)
            }
            KeyCode::Enter if d.section == ClientTeamDashboardSection::Tasks => {
                Some(A::TaskDetails)
            }
            KeyCode::Enter if d.section == ClientTeamDashboardSection::Broadcast => {
                Some(A::MessageDetails)
            }
            KeyCode::Up | KeyCode::Down if d.section == ClientTeamDashboardSection::Tasks => {
                self.move_team_dashboard_task(if key.code == KeyCode::Down { 1 } else { -1 });
                outcome.repaint = true;
                return true;
            }
            KeyCode::Up | KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('k')
                if d.section == ClientTeamDashboardSection::Broadcast =>
            {
                let count = d
                    .snapshot
                    .as_ref()
                    .ok()
                    .map(|s| s.broadcasts.len())
                    .unwrap_or_default();
                d.guided.message_cursor = if matches!(key.code, KeyCode::Down | KeyCode::Char('j'))
                {
                    (d.guided.message_cursor + 1).min(count.saturating_sub(1))
                } else {
                    d.guided.message_cursor.saturating_sub(1)
                };
                outcome.repaint = true;
                return true;
            }
            _ => None,
        };
        if let Some(action) = action {
            self.begin_team_dashboard_action(action);
            outcome.repaint = true;
            return true;
        }
        false
    }

    fn run_question_decision(&mut self, decision: crate::cli::questions::QuestionDecision) {
        let db = crate::team_view::default_database_path();
        let identity = crate::team_session::identity_path(&db);
        let Some(ClientShellOverlay::TeamDashboard(d)) = self.overlay.as_mut() else {
            return;
        };
        if d.pending.is_some() {
            return;
        }
        let Some((request, _)) = d.guided.consent.take() else {
            return;
        };
        let Some(team) = d
            .snapshot
            .as_ref()
            .ok()
            .and_then(|s| s.teams.first())
            .map(|t| t.team_id.clone())
        else {
            return;
        };
        let (tx, rx) = std::sync::mpsc::channel();
        d.pending = Some(rx);
        d.status = "Processing your decision. Assistant answers can take a moment.".into();
        std::thread::spawn(move || {
            let work = crate::cli::questions::decide_and_run(
                &db, &identity, &team, &request, decision, true,
            );
            let success = work.is_ok();
            let result = ClientTeamDashboardResult {
                status: if success {
                    "Question processed. The result is saved in Requests and queued for your teammate.".into()
                } else {
                    work.as_ref().err().cloned().unwrap_or_default()
                },
                success,
                details: work.ok(),
                action: None,
                relay_credentials: None,
                share_code: None,
                invite: None,
            };
            let _ = tx.send(result);
        });
    }
    pub(super) fn begin_guided_form(&mut self, action: ClientTeamDashboardAction) -> bool {
        use ClientTeamDashboardAction as A;
        let Some(ClientShellOverlay::TeamDashboard(d)) = self.overlay.as_mut() else {
            return false;
        };
        if d.pending.is_some() {
            d.status = "Finishing the previous action…".into();
            return true;
        }
        d.guided.details = None;
        let snapshot = d.snapshot.as_ref().ok();
        let profile = snapshot.and_then(|s| s.profile.as_ref());
        if matches!(action, A::CreateTeam | A::Join) && profile.is_none() {
            d.guided.next = Some(action);
            self.begin_guided_form(A::Profile);
            return true;
        }
        let fields = match action {
            A::Profile => vec![(
                "Your name".into(),
                TextEditor::new(
                    profile.map(|p| p.display_name.as_str()).unwrap_or(""),
                    false,
                ),
            )],
            A::Project => vec![(
                "Project folder".into(),
                TextEditor::new(
                    &std::env::current_dir()
                        .unwrap_or_default()
                        .to_string_lossy(),
                    false,
                ),
            )],
            A::Task | A::Ask | A::Check => vec![(
                if action == A::Task {
                    "Task"
                } else {
                    "Question"
                }
                .into(),
                TextEditor::default(),
            )],
            A::Read => vec![(
                "Question about this computer's project".into(),
                TextEditor::default(),
            )],
            A::TaskFollowUp => vec![(
                "Question or update about this task".into(),
                TextEditor::default(),
            )],
            A::Broadcast => vec![(
                "Message to everyone in this team".into(),
                TextEditor::default(),
            )],
            A::SwitchTeam => {
                let base = crate::team_view::base_database_path();
                d.guided.contexts = crate::team_session::list_contexts(&base).unwrap_or_default();
                d.guided.person_cursor = 0;
                d.status =
                    "Choose a team; Enter switches without deleting your other team's data.".into();
                d.form = Some(ClientTeamDashboardForm {
                    action,
                    fields: Vec::new(),
                    current: 0,
                    member_picker_active: true,
                    member_cursor: 0,
                    selected_member_ids: Vec::new(),
                });
                return true;
            }
            A::TaskDetails | A::MessageDetails => {
                self.open_team_details(action);
                return true;
            }
            _ => return false,
        };
        let mut fields = fields;
        let db = crate::team_view::default_database_path();
        if matches!(
            action,
            A::Task | A::Ask | A::Check | A::Read | A::Broadcast | A::TaskFollowUp
        ) {
            if let Ok(Some(draft)) = crate::team_session::load_draft(&db, draft_kind(action)) {
                if let Some((_, editor)) = fields.first_mut() {
                    *editor = TextEditor::new(&draft, false);
                }
            }
        }
        d.form = Some(ClientTeamDashboardForm {
            action,
            fields,
            current: 0,
            member_picker_active: matches!(action, A::Task | A::Ask | A::Check),
            member_cursor: 0,
            selected_member_ids: Vec::new(),
        });
        d.status = match action {
            A::Task => "Choose who should receive this task. Enter selects the highlighted person.".into(),
            A::Ask | A::Check => "Choose this computer or a teammate, then type your question.".into(),
            A::Project => "Optional. The folder stays on this device; only its project name is announced to the team.".into(),
            A::Broadcast => "Everyone approved in this team will receive this message. Offline devices receive it after reconnecting.".into(),
            _ => String::new(),
        };
        true
    }

    pub(super) fn guided_form_key(
        &mut self,
        key: &crate::input::TerminalKey,
        outcome: &mut ClientShellInput,
    ) -> bool {
        use ClientTeamDashboardAction as A;
        let Some(ClientShellOverlay::TeamDashboard(d)) = self.overlay.as_mut() else {
            return false;
        };
        let Some(form) = d.form.as_mut() else {
            return false;
        };
        if !matches!(
            form.action,
            A::Profile
                | A::Project
                | A::Task
                | A::Ask
                | A::Read
                | A::Check
                | A::Broadcast
                | A::TaskFollowUp
                | A::SwitchTeam
        ) {
            return false;
        }
        outcome.repaint = true;
        if key.code == KeyCode::Esc {
            if let Some((_, editor)) = form.fields.first() {
                let _ = crate::team_session::save_draft(
                    &crate::team_view::default_database_path(),
                    draft_kind(form.action),
                    editor.as_str(),
                );
            }
            d.form = None;
            d.status = "Draft kept. Choose the action again to continue.".into();
            return true;
        }
        if form.action == A::SwitchTeam {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    form.member_cursor = form.member_cursor.saturating_sub(1)
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    form.member_cursor =
                        (form.member_cursor + 1).min(d.guided.contexts.len().saturating_sub(1))
                }
                KeyCode::Enter => {
                    if let Some((path, name)) = d.guided.contexts.get(form.member_cursor).cloned() {
                        match crate::team_session::activate_database(
                            &crate::team_view::base_database_path(),
                            &path,
                        ) {
                            Ok(()) => {
                                d.snapshot =
                                    crate::team_view::load_default().map_err(|e| e.to_string());
                                d.form = None;
                                d.status =
                                    format!("Switched to {name}. Your other team is unchanged.");
                                self.restore_guided_connection();
                            }
                            Err(e) => d.status = e,
                        }
                    }
                }
                _ => {}
            }
            return true;
        }
        if form.member_picker_active {
            let members = guided_recipients(d.snapshot.as_ref().ok(), form.action);
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    form.member_cursor = form.member_cursor.saturating_sub(1)
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    form.member_cursor =
                        (form.member_cursor + 1).min(members.len().saturating_sub(1))
                }
                KeyCode::Char(' ') | KeyCode::Enter => {
                    if let Some((id, name)) = members.get(form.member_cursor) {
                        form.selected_member_ids = vec![id.clone()];
                        form.member_picker_active = false;
                        d.status = format!(
                            "To: {name}. Type your {} and press Enter to send.",
                            if form.action == A::Task {
                                "task"
                            } else {
                                "question"
                            }
                        );
                    } else {
                        d.status =
                            "Invite a teammate first. They can receive queued tasks while offline."
                                .into();
                    }
                }
                _ => {}
            }
            return true;
        }
        if key.code != KeyCode::Enter {
            if let Some((_, editor)) = form.fields.get_mut(form.current) {
                outcome.repaint |= editor.handle_key(key).unwrap_or(false);
            }
            return true;
        }
        let text = form
            .fields
            .first()
            .map(|(_, e)| e.as_str().trim().to_owned())
            .unwrap_or_default();
        if text.is_empty() {
            d.status = "Enter a value to continue.".into();
            return true;
        }
        let action = form.action;
        let target = form
            .selected_member_ids
            .first()
            .cloned()
            .unwrap_or_default();
        let db = crate::team_view::default_database_path();
        let _ = crate::team_session::save_draft(&db, draft_kind(action), &text);
        let snapshot = d.snapshot.as_ref().ok();
        let local_repo = snapshot
            .and_then(|s| s.repositories.first())
            .cloned()
            .unwrap_or_default();
        let values = match action {
            A::Task => vec![target, text],
            A::Ask | A::Check if target != "@local" => {
                let team = snapshot
                    .and_then(|s| s.teams.first())
                    .map(|t| t.team_id.clone())
                    .unwrap_or_default();
                let projects =
                    crate::cli::questions::list_projects(&db, &team, &target).unwrap_or_default();
                let Some(project) = projects.first() else {
                    d.status="This teammate has not connected a project yet. Ask them to choose Connect project. Your question draft is kept.".into();
                    return true;
                };
                vec![target, project.repository_id.clone(), text]
            }
            A::Ask | A::Read => {
                if local_repo.is_empty() {
                    d.form = None;
                    d.guided.next = Some(A::Read);
                    self.begin_guided_form(A::Project);
                    return true;
                }
                vec![text, local_repo]
            }
            A::TaskFollowUp => {
                let task = snapshot
                    .and_then(|s| s.tasks.get(d.task_cursor))
                    .map(|t| t.task_id.clone())
                    .unwrap_or_default();
                vec![task, text]
            }
            _ => vec![text],
        };
        let action = if matches!(action, A::Ask | A::Check) {
            if values.len() == 3 {
                A::Check
            } else {
                A::Read
            }
        } else {
            action
        };
        d.form = None;
        if let Err(error) = self.start_team_dashboard_action(action, values) {
            if let Some(ClientShellOverlay::TeamDashboard(d)) = self.overlay.as_mut() {
                d.status = error;
            }
        }
        true
    }

    pub(super) fn start_guided_action(
        &mut self,
        action: ClientTeamDashboardAction,
        values: &[String],
    ) -> Option<Result<(), String>> {
        use ClientTeamDashboardAction as A;
        if !matches!(
            action,
            A::Profile
                | A::Project
                | A::Read
                | A::Check
                | A::Join
                | A::AcceptInvite
                | A::TaskFollowUp
                | A::TaskStatus
        ) {
            return None;
        }
        let db = crate::team_view::default_database_path();
        let base = crate::team_view::base_database_path();
        let identity = crate::team_session::identity_path(&db);
        let Some(ClientShellOverlay::TeamDashboard(d)) = self.overlay.as_mut() else {
            return Some(Err("Open the team home first".into()));
        };
        let profile = d
            .snapshot
            .as_ref()
            .ok()
            .and_then(|s| s.profile.as_ref())
            .cloned();
        let team = d
            .snapshot
            .as_ref()
            .ok()
            .and_then(|s| s.teams.first())
            .map(|t| t.team_id.clone());
        let role = if d.guided.next == Some(A::CreateTeam) {
            Role::Manager
        } else {
            profile
                .as_ref()
                .map(|p| p.role.clone())
                .unwrap_or(Role::Implementer)
        };
        let name = profile
            .as_ref()
            .map(|p| p.display_name.clone())
            .unwrap_or_default();
        let queued_task = d
            .snapshot
            .as_ref()
            .ok()
            .and_then(|s| s.tasks.iter().find(|t| Some(&t.task_id) == values.first()))
            .is_some_and(|t| t.status == crate::team::TaskStatus::Queued);
        let pending_invite = if action == A::AcceptInvite {
            d.guided.invite.take()
        } else {
            None
        };
        let values = values.to_vec();
        let sink = self.relay_credential_sink.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        d.pending = Some(receiver);
        d.status = "Working…".into();
        std::thread::spawn(move || {
            let mut result = ClientTeamDashboardResult {
                status: String::new(),
                relay_credentials: None,
                share_code: None,
                action: Some(action),
                success: false,
                details: None,
                invite: None,
            };
            let work = (|| -> Result<String, String> {
                match action {
                    A::Profile => {
                        crate::team_session::setup_profile(&db, &values[0], role)?;
                        Ok("Profile ready. Your identity is saved on this device.".into())
                    }
                    A::Project => {
                        let approval = crate::team_session::link_project(
                            &db,
                            std::path::Path::new(&values[0]),
                        )?;
                        if team.is_some() {
                            let label = std::path::Path::new(&values[0])
                                .file_name()
                                .and_then(|s| s.to_str())
                                .unwrap_or("Project");
                            crate::cli::questions::publish_project(
                                &db,
                                &identity,
                                &approval.repository_id,
                                label,
                            )?;
                        }
                        Ok(
                            "Project connected on this device. You can now ask local questions."
                                .into(),
                        )
                    }
                    A::Read => {
                        let answer = crate::cli::read::answer_local(&db, &values[1], &values[0])?;
                        result.details = Some(answer);
                        Ok(
                            "Answer ready. Use Up/Down or the mouse wheel to read; Esc returns."
                                .into(),
                        )
                    }
                    A::Check => {
                        let id = crate::cli::questions::ask(
                            &db,
                            &identity,
                            team.as_deref().ok_or("Join a team first")?,
                            &values[0],
                            &values[1],
                            &values[2],
                        )?;
                        Ok(format!(
                            "Question sent ({id}). Waiting for the repository owner; see Requests."
                        ))
                    }
                    A::Join => {
                        let input = values[0].trim();
                        let endpoint = std::env::var("HERDR_RELAY_URL")
                            .unwrap_or_else(|_| crate::cli::relay::DEFAULT_RELAY_URL.into());
                        let signed = if crate::cli::relay::is_short_invite_code(input) {
                            crate::cli::relay::claim_invite_code(&endpoint, input)?
                        } else {
                            input.to_owned()
                        };
                        let bundle = crate::cli::team::decode_invite_code(&signed)
                            .map_err(|e| e.to_string())?;
                        crate::cli::team::validate_invite_for_existing_team(
                            &bundle,
                            &bundle.invitation.team_id,
                            &bundle.invitation.authority_member_id,
                        )
                        .map_err(|e| e.to_string())?;
                        let label = bundle.invitation.team_name.clone();
                        result.invite = Some(ClientPendingInvite(bundle));
                        Ok(format!("Join {label}? Enter confirms, Esc cancels. Your previous team will be preserved."))
                    }
                    A::AcceptInvite => {
                        let bundle = pending_invite
                            .ok_or("Generate a fresh invitation and try Join again")?
                            .0;
                        let dest =
                            crate::team_session::prepare_join_database(&base, &db, &bundle, &name)?;
                        let dest_identity = crate::team_session::identity_path(&dest);
                        let existing = crate::team_storage::TeamStore::open(&dest)
                            .map_err(|e| e.to_string())?
                            .team(&bundle.invitation.team_id)
                            .map_err(|e| e.to_string())?
                            .is_some();
                        let request = if existing {
                            crate::cli::join_inbox::reconnect_request(
                                &dest,
                                &dest_identity,
                                &bundle,
                            )?
                        } else {
                            Some(
                                crate::cli::team::accept_invite_bundle(
                                    &dest,
                                    &dest_identity,
                                    &bundle,
                                )
                                .map_err(|e| e.to_string())?,
                            )
                        };
                        if let Some(request) = request {
                            crate::cli::join_inbox::submit(
                                &bundle.relay_url,
                                &bundle.relay_token,
                                &request,
                            )?;
                        }
                        crate::team_session::save_connection(
                            &dest,
                            &bundle.relay_url,
                            &bundle.relay_token,
                        )?;
                        crate::team_session::activate_database(&base, &dest)?;
                        result.relay_credentials = Some((bundle.relay_url, bundle.relay_token));
                        Ok("Connected. Waiting for manager approval if this is your first join. See Requests; this device will reconnect after restart.".into())
                    }
                    A::TaskFollowUp => {
                        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
                        let output = std::process::Command::new(exe)
                            .args([
                                "task",
                                "follow-up",
                                "--db",
                                &db.to_string_lossy(),
                                "--team",
                                team.as_deref().ok_or("Join a team first")?,
                                "--actor",
                                &profile.as_ref().ok_or("Set up your profile")?.member_id,
                                "--identity",
                                &identity.to_string_lossy(),
                                "--task",
                                &values[0],
                                "--message",
                                &values[1],
                            ])
                            .output()
                            .map_err(|e| e.to_string())?;
                        if !output.status.success() {
                            return Err(String::from_utf8_lossy(&output.stderr)
                                .chars()
                                .take(2000)
                                .collect());
                        }
                        Ok("Task follow-up saved and queued for delivery.".into())
                    }
                    A::TaskStatus => {
                        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
                        let statuses = if queued_task && values[1] == "in_progress" {
                            vec!["delivered", "in_progress"]
                        } else {
                            vec![values[1].as_str()]
                        };
                        for status in statuses {
                            let output = std::process::Command::new(&exe)
                                .args([
                                    "task",
                                    "status",
                                    "--db",
                                    &db.to_string_lossy(),
                                    "--team",
                                    team.as_deref().ok_or("Join a team first")?,
                                    "--actor",
                                    &profile.as_ref().ok_or("Set up your profile")?.member_id,
                                    "--identity",
                                    &identity.to_string_lossy(),
                                    "--task",
                                    &values[0],
                                    "--status",
                                    status,
                                ])
                                .output()
                                .map_err(|e| e.to_string())?;
                            if !output.status.success() {
                                return Err(String::from_utf8_lossy(&output.stderr)
                                    .chars()
                                    .take(2000)
                                    .collect());
                            }
                        }
                        Ok("Task updated and queued for your team.".into())
                    }
                    _ => Err("Unsupported action".into()),
                }
            })();
            match work {
                Ok(status) => {
                    result.status = status;
                    result.success = true;
                    let _ = crate::team_session::save_draft(&db, draft_kind(action), "");
                }
                Err(error) => result.status = error,
            }
            if let (Some(sink), Some((url, token))) = (sink, result.relay_credentials.as_ref()) {
                let _ = sink.set(url.clone(), token.clone());
            }
            let _ = sender.send(result);
        });
        Some(Ok(()))
    }

    pub(super) fn restore_guided_connection(&mut self) {
        let db = crate::team_view::default_database_path();
        let connection = crate::team_session::connection_from_env_or_store(&db)
            .ok()
            .flatten();
        if let Some(ClientShellOverlay::TeamDashboard(d)) = self.overlay.as_mut() {
            d.relay_url = connection.as_ref().map(|c| c.endpoint.clone());
            d.relay_token = connection.as_ref().map(|c| c.token.clone());
            d.join_inbox.next_poll = None;
            d.join_inbox.pending = None;
            d.guided.refresh_at = None;
            d.join_inbox.entries.clear();
            d.guided.questions.clear();
        }
        if let (Some(sink), Some(c)) = (self.relay_credential_sink.as_ref(), connection) {
            let _ = sink.set(c.endpoint, c.token);
        }
    }

    pub(super) fn tick_guided_team(&mut self) -> bool {
        let Some(ClientShellOverlay::TeamDashboard(d)) = self.overlay.as_mut() else {
            return false;
        };
        if d.pending.is_none()
            && d.form.is_none()
            && d.guided.next.is_some()
            && d.snapshot
                .as_ref()
                .ok()
                .and_then(|s| s.profile.as_ref())
                .is_some()
        {
            let action = d.guided.next.take().unwrap();
            self.begin_team_dashboard_action(action);
            return true;
        }
        if d.guided
            .refresh_at
            .is_none_or(|next| next <= std::time::Instant::now())
        {
            d.guided.refresh_at =
                Some(std::time::Instant::now() + std::time::Duration::from_secs(3));
            let db = crate::team_view::default_database_path();
            d.guided.questions = crate::cli::questions::pending_questions(&db).unwrap_or_default();
            d.snapshot = crate::team_view::load_default().map_err(|e| e.to_string());
            return true;
        }
        false
    }

    pub(super) fn open_team_details(&mut self, action: ClientTeamDashboardAction) {
        let Some(ClientShellOverlay::TeamDashboard(d)) = self.overlay.as_mut() else {
            return;
        };
        if action == ClientTeamDashboardAction::MessageDetails {
            if let Some(message) = d
                .snapshot
                .as_ref()
                .ok()
                .and_then(|s| s.broadcasts.get(d.guided.message_cursor))
            {
                d.guided.details = Some((
                    format!("Message from {}", message.sender_member_id),
                    message.message.clone(),
                    0,
                ));
            }
        } else if let Some(task) = d
            .snapshot
            .as_ref()
            .ok()
            .and_then(|s| s.tasks.get(d.task_cursor))
        {
            let db = crate::team_view::default_database_path();
            let mut body = format!("{}\n\nState: {:?}\n\n", task.description, task.status);
            if let Ok(store) = crate::team_storage::TeamStore::open_read_only(&db) {
                if let Ok(activity) = store.task_activity(&task.task_id) {
                    for row in activity {
                        body.push_str(&format!("{}: {}\n", row.actor_member_id, row.message));
                    }
                }
            }
            d.guided.details = Some(("Task details and history".into(), body, 0));
        }
    }
}

pub(super) fn draft_kind(action: ClientTeamDashboardAction) -> &'static str {
    use ClientTeamDashboardAction as A;
    match action {
        A::Broadcast => "message",
        A::Task => "task",
        A::TaskFollowUp => "followup",
        A::Ask | A::Read | A::Check => "question",
        _ => "setup",
    }
}

pub(super) fn guided_recipients(
    snapshot: Option<&crate::team_view::TeamDashboardSnapshot>,
    action: ClientTeamDashboardAction,
) -> Vec<(String, String)> {
    use ClientTeamDashboardAction as A;
    let mut people = Vec::new();
    if matches!(action, A::Ask | A::Read) {
        people.push(("@local".into(), "This computer".into()));
    }
    if let Some(s) = snapshot {
        let me = s.profile.as_ref().map(|p| p.member_id.as_str());
        people.extend(
            s.teammates
                .iter()
                .filter(|p| p.active && Some(p.member_id.as_str()) != me)
                .map(|p| (p.member_id.clone(), p.display_name.clone())),
        );
    }
    people
}
