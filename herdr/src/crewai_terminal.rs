//! Conversational session hosted by an ordinary Herdr terminal pane.
//! Shells and other panes continue running independently of this process.
use base64::Engine as _;
use std::io::{self, IsTerminal, Write};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind},
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Terminal,
};
use unicode_width::UnicodeWidthChar;

use crate::crewai_command::{self, Command};
use crate::crewai_composer::{Composer, Picker};
use crate::crewai_conversation::{ConversationEntry, ConversationKind};
use crate::crewai_service::{BackgroundOperation, OperationUpdate, TeamService};
use crate::team_storage::TeamStore;

struct ScreenGuard;
impl ScreenGuard {
    fn enter() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        if let Err(error) = execute!(
            io::stdout(),
            EnterAlternateScreen,
            event::EnableMouseCapture,
            event::EnableBracketedPaste
        ) {
            let _ = execute!(
                io::stdout(),
                event::DisableBracketedPaste,
                event::DisableMouseCapture,
                LeaveAlternateScreen
            );
            let _ = terminal::disable_raw_mode();
            return Err(error);
        }
        Ok(Self)
    }
}
impl Drop for ScreenGuard {
    fn drop(&mut self) {
        let _ = execute!(
            io::stdout(),
            event::DisableBracketedPaste,
            event::DisableMouseCapture,
            LeaveAlternateScreen
        );
        let _ = terminal::disable_raw_mode();
        let _ = io::stdout().flush();
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn display_safe(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_control() && !matches!(ch, '\n' | '\t') {
                '�'
            } else {
                ch
            }
        })
        .collect()
}

fn local_confirmation(repository_id: String, question: String) -> TeamPanel {
    TeamPanel {
        title: "Confirm local Codex access".into(),
        selected: 3,
        rows: vec![
            PanelRow {
                label: "OS isolation is not verified.".into(),
                action: None,
            },
            PanelRow {
                label: "Codex may access files and network.".into(),
                action: None,
            },
            PanelRow {
                label: "This question and answer stay on this device.".into(),
                action: None,
            },
            PanelRow {
                label: "Continue with limited isolation".into(),
                action: Some(TeamAction::RunLocal(repository_id, question)),
            },
        ],
    }
}

fn question_review(question: crate::cli::questions::Question) -> TeamPanel {
    let safe = display_safe(&question.question).replace('\n', " ");
    let characters = safe.chars().collect::<Vec<_>>();
    let mut rows = characters
        .chunks(35)
        .enumerate()
        .map(|(index, chunk)| PanelRow {
            label: format!(
                "{}{}",
                if index == 0 {
                    "Question: "
                } else {
                    "          "
                },
                chunk.iter().collect::<String>()
            ),
            action: None,
        })
        .collect::<Vec<_>>();
    rows.push(PanelRow {
        label: "OS isolation is not verified.".into(),
        action: None,
    });
    rows.push(PanelRow {
        label: "Codex may access files and network.".into(),
        action: None,
    });
    rows.push(PanelRow {
        label: "Only the bounded final answer is shared.".into(),
        action: None,
    });
    for (label, decision) in [
        (
            "Allow once",
            crate::cli::questions::QuestionDecision::AllowOnce,
        ),
        (
            "Allow this project",
            crate::cli::questions::QuestionDecision::AllowProject,
        ),
        ("Decline", crate::cli::questions::QuestionDecision::Deny),
    ] {
        rows.push(PanelRow {
            label: label.into(),
            action: Some(TeamAction::ConfirmQuestion(question.clone(), decision)),
        });
    }
    TeamPanel {
        title: "Review project question".into(),
        rows,
        selected: 0,
    }
}

#[derive(Clone)]
enum TeamAction {
    Invite,
    Requests,
    PollRequests,
    Review(crate::cli::team::JoinRequest),
    Decide(crate::cli::team::JoinRequest, bool),
    ConnectProject,
    ConnectForRead(String),
    EditName,
    Join,
    Create,
    Switch,
    Activate(std::path::PathBuf),
    Questions,
    ReviewQuestion(crate::cli::questions::Question),
    ConfirmQuestion(
        crate::cli::questions::Question,
        crate::cli::questions::QuestionDecision,
    ),
    RunQuestion(
        crate::cli::questions::Question,
        crate::cli::questions::QuestionDecision,
    ),
    PrepareLocal(String, String),
    RunLocal(String, String),
    SendRemote(String, String, String, String),
}
#[derive(Clone)]
struct PanelRow {
    label: String,
    action: Option<TeamAction>,
}
struct TeamPanel {
    title: String,
    rows: Vec<PanelRow>,
    selected: usize,
}
enum TeamActionResult {
    Invite(String),
    Requests(crate::cli::join_inbox::Inbox, bool),
    Decision(bool),
    LocalAnswer(String, String, Result<String, String>),
    QuestionFinished(Result<String, String>),
}

fn perform_team_action(
    session: &mut Session,
    terminal: &mut crate::crewai_onboarding::PaneTerminal,
    action: TeamAction,
) -> io::Result<()> {
    match action {
        TeamAction::Invite
        | TeamAction::Requests
        | TeamAction::PollRequests
        | TeamAction::Decide(_, _)
        | TeamAction::RunQuestion(_, _)
        | TeamAction::RunLocal(_, _) => session.start_action(action),
        TeamAction::Questions => match session.service.questions() {
            Ok(questions) => {
                let rows = questions
                    .into_iter()
                    .filter(|question| {
                        question.target_member_id == session.member_id
                            && matches!(
                                question.delivery.as_str(),
                                "Created" | "Queued" | "Delivered"
                            )
                    })
                    .map(|question| PanelRow {
                        label: format!("{}", display_safe(&question.question).replace('\n', " ")),
                        action: Some(TeamAction::ReviewQuestion(question)),
                    })
                    .collect::<Vec<_>>();
                if rows.is_empty() {
                    session.notice = "No project questions awaiting your approval".into();
                } else {
                    session.panel = Some(TeamPanel {
                        title: "Project questions".into(),
                        rows,
                        selected: 0,
                    });
                }
            }
            Err(error) => session.notice = error,
        },
        TeamAction::ReviewQuestion(question) => session.panel = Some(question_review(question)),
        TeamAction::ConfirmQuestion(question, decision) => {
            if decision == crate::cli::questions::QuestionDecision::Deny {
                session.start_action(TeamAction::RunQuestion(question, decision));
            } else {
                session.panel = Some(TeamPanel {
                    title: "Confirm limited isolation".into(),
                    selected: 3,
                    rows: vec![
                        PanelRow {
                            label: "OS isolation is not verified.".into(),
                            action: None,
                        },
                        PanelRow {
                            label: "Codex may access files and network.".into(),
                            action: None,
                        },
                        PanelRow {
                            label: "The bounded answer will be sent to your teammate.".into(),
                            action: None,
                        },
                        PanelRow {
                            label: "I understand; answer this question".into(),
                            action: Some(TeamAction::RunQuestion(question, decision)),
                        },
                    ],
                });
            }
        }
        TeamAction::PrepareLocal(repository_id, question) => {
            session.panel = Some(local_confirmation(repository_id, question))
        }
        TeamAction::SendRemote(target, repository_id, question, name) => {
            match session
                .service
                .ask_remote(&target, &repository_id, &question)
            {
                Ok(_) => {
                    let input = session.composer.text.clone();
                    let _ = session.record_command(&input);
                    session.command_history.push(input);
                    session.composer.clear();
                    let _ = session.save_draft();
                    session.notice = format!("Question saved; syncing for {name}'s approval");
                    session.schedule_sync();
                }
                Err(error) => session.notice = error,
            }
        }
        TeamAction::Review(request) => {
            session.panel = Some(TeamPanel {
                title: format!("{} wants to join", request.member.display_name),
                rows: vec![
                    PanelRow {
                        label: "Approve teammate".into(),
                        action: Some(TeamAction::Decide(request.clone(), true)),
                    },
                    PanelRow {
                        label: "Decline request".into(),
                        action: Some(TeamAction::Decide(request, false)),
                    },
                ],
                selected: 0,
            });
        }
        TeamAction::ConnectProject => {
            let initial = std::env::current_dir()
                .ok()
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default();
            if let Some(folder)=crate::crewai_onboarding::prompt(terminal,"Connect a project","Choose a folder on this device. Project files stay here; questions still require your approval.",&initial)? {
                match session.service.connect_project(std::path::Path::new(&folder)) {Ok(_)=>{session.notice="Project connected on this device".into();session.schedule_sync();},Err(error)=>session.notice=error}
            }
        }
        TeamAction::ConnectForRead(question) => {
            let initial = std::env::current_dir()
                .ok()
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default();
            if let Some(folder) = crate::crewai_onboarding::prompt(
                terminal,
                "Connect your project",
                "The project stays on this device. Choose the folder to ask about.",
                &initial,
            )? {
                match session
                    .service
                    .connect_project(std::path::Path::new(&folder))
                {
                    Ok(project) => {
                        session.schedule_sync();
                        session.panel = Some(local_confirmation(project.repository_id, question));
                    }
                    Err(error) => session.notice = error,
                }
            }
        }
        TeamAction::EditName => {
            if let Some(name) = crate::crewai_onboarding::prompt(
                terminal,
                "Edit your name",
                "Choose the name shown on this device.",
                &session.member_name,
            )? {
                match TeamService::setup_profile(
                    &session.database,
                    &name,
                    crate::team::Role::Implementer,
                ) {
                    Ok(member) => {
                        session.member_name = member.display_name;
                        session.notice = "Name updated on this device".into();
                    }
                    Err(error) => session.notice = error,
                }
            }
        }
        TeamAction::Join => {
            let base = crate::team_view::base_database_path();
            match crate::crewai_onboarding::join(
                terminal,
                &base,
                &session.database,
                &session.member_name,
            )? {
                Ok(Some(path)) => {
                    session.save_draft().map_err(io::Error::other)?;
                    match Session::open(path) {
                        Ok(new_session) => *session = new_session,
                        Err(error) => session.notice = error,
                    };
                }
                Ok(None) => {}
                Err(error) => session.notice = error,
            }
        }
        TeamAction::Create => {
            if let Some(team_name) = crate::crewai_onboarding::prompt(
                terminal,
                "Create another team",
                "Choose a team name.",
                "",
            )? {
                let base = crate::team_view::base_database_path();
                let path = crate::team_session::new_team_database(&base);
                let work_path = path.clone();
                let name = session.member_name.clone();
                let result = crate::crewai_onboarding::wait_for(
                    terminal,
                    "Creating your team",
                    move || {
                        TeamService::setup_profile(&work_path, &name, crate::team::Role::Manager)?;
                        TeamService::create_team(&work_path, &team_name).map(|_| ())
                    },
                )?;
                match result {
                    Ok(()) => {
                        session.save_draft().map_err(io::Error::other)?;
                        match crate::team_session::activate_database(&base, &path)
                            .and_then(|_| Session::open(path))
                        {
                            Ok(new_session) => *session = new_session,
                            Err(error) => session.notice = error,
                        }
                    }
                    Err(error) => session.notice = error,
                }
            }
        }
        TeamAction::Switch => {
            let base = crate::team_view::base_database_path();
            match crate::team_session::list_contexts(&base) {
                Ok(contexts) if !contexts.is_empty() => {
                    let rows = contexts
                        .into_iter()
                        .map(|(path, name)| PanelRow {
                            label: name,
                            action: Some(TeamAction::Activate(path)),
                        })
                        .collect();
                    session.panel = Some(TeamPanel {
                        title: "Switch team".into(),
                        rows,
                        selected: 0,
                    });
                }
                Ok(_) => session.notice = "No saved teams found".into(),
                Err(error) => session.notice = error,
            }
        }
        TeamAction::Activate(path) => {
            session.save_draft().map_err(io::Error::other)?;
            let base = crate::team_view::base_database_path();
            match crate::team_session::activate_database(&base, &path)
                .and_then(|_| Session::open(path))
            {
                Ok(new_session) => *session = new_session,
                Err(error) => session.notice = error,
            }
        }
    }
    Ok(())
}

struct Session {
    database: std::path::PathBuf,
    team_id: String,
    team_name: String,
    member_id: String,
    member_name: String,
    service: TeamService,
    composer: Composer,
    forced_picker: Option<Picker>,
    command_history: Vec<String>,
    history_index: Option<usize>,
    history_draft: String,
    selected_entry: Option<String>,
    panel: Option<TeamPanel>,
    action: Option<BackgroundOperation<TeamActionResult>>,
    action_is_poll: bool,
    invite_code: Option<String>,
    invite_created_at: Option<std::time::Instant>,
    noticed_questions: std::collections::HashSet<String>,
    notice: String,
    scroll_up: u16,
    sync: Option<BackgroundOperation<bool>>,
}

impl Session {
    fn open(database: std::path::PathBuf) -> Result<Self, String> {
        let store = TeamStore::open(&database).map_err(|e| e.to_string())?;
        let profile = store
            .load_profile()
            .map_err(|e| e.to_string())?
            .ok_or("Create or join a team using the Team dashboard first")?;
        let team = store
            .teams()
            .map_err(|e| e.to_string())?
            .into_iter()
            .next()
            .ok_or("Create or join a team using the Team dashboard first")?;
        let service = TeamService::open(&database, &team.team_id)?;
        let input = store
            .conversation_draft(&team.team_id, "main")
            .map_err(|e| e.to_string())?
            .unwrap_or_default();
        let command_history = service
            .conversation()
            .unwrap_or_default()
            .into_iter()
            .filter(|entry| entry.kind == ConversationKind::Command)
            .map(|entry| entry.text)
            .collect();
        let mut session = Self {
            database,
            team_id: team.team_id,
            team_name: team.name,
            member_id: profile.member.member_id,
            member_name: profile.member.display_name,
            service,
            composer: Composer::from_text(input),
            forced_picker: None,
            command_history,
            history_index: None,
            history_draft: String::new(),
            selected_entry: None,
            panel: None,
            action: None,
            action_is_poll: false,
            invite_code: None,
            invite_created_at: None,
            noticed_questions: std::collections::HashSet::new(),
            notice: "Type / to see commands".into(),
            scroll_up: 0,
            sync: None,
        };
        if !session.approved_members().contains(&session.member_id) {
            session.notice = "Join request sent. Waiting for the manager's approval.".into();
        }
        Ok(session)
    }

    fn save_draft(&self) -> Result<(), String> {
        TeamStore::open(&self.database)
            .map_err(|e| e.to_string())?
            .save_conversation_draft(&self.team_id, "main", &self.composer.text)
            .map_err(|e| e.to_string())
    }

    fn record_command(&self, text: &str) -> Result<String, String> {
        let at = now();
        let entry = ConversationEntry {
            team_id: self.team_id.clone(),
            thread_id: "main".into(),
            entry_id: format!("command:{}", crate::cli::team::generate_token()),
            related_id: None,
            kind: ConversationKind::Command,
            speaker_id: Some(self.member_id.clone()),
            recipient_id: None,
            text: text.into(),
            status: "submitted".into(),
            created_at: at,
            updated_at: at,
        };
        let id = entry.entry_id.clone();
        TeamStore::open(&self.database)
            .map_err(|e| e.to_string())?
            .save_conversation_entry(&entry)
            .map_err(|e| e.to_string())?;
        Ok(id)
    }

    fn schedule_sync(&mut self) {
        if self.sync.is_none() {
            self.sync = Some(self.service.start_sync());
        }
    }
    fn poll_sync(&mut self) {
        let Some(operation) = &self.sync else { return };
        while let Ok(update) = operation.updates.try_recv() {
            match update {
                OperationUpdate::Started => {}
                OperationUpdate::Finished(result) => {
                    if self.action.is_some() && !self.action_is_poll && result.is_ok() {
                        self.sync = None;
                        return;
                    }
                    self.notice = match result {
                        Ok(true) => "Team sync complete".into(),
                        Ok(false) => "Saved locally; connect the team to sync".into(),
                        Err(error) if error.contains("Waiting for the manager") => {
                            "Waiting for the manager's approval".into()
                        }
                        Err(error) => format!("Saved locally; sync failed: {error}"),
                    };
                    self.sync = None;
                    return;
                }
            }
        }
    }

    fn team_panel(&self) -> TeamPanel {
        let mut rows = Vec::new();
        let approved = self.approved_members();
        if let Ok(members) = self.service.teammates() {
            for member in members.iter().filter(|member| member.active) {
                let status = if approved.contains(&member.member_id) {
                    "approved"
                } else {
                    "awaiting approval"
                };
                rows.push(PanelRow {
                    label: format!("{} · {:?} · {status}", member.display_name, member.role),
                    action: None,
                });
            }
        }
        if let Ok(projects) = self.local_projects() {
            for project in &projects {
                let label = std::path::Path::new(&project.local_path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("Project");
                rows.push(PanelRow {
                    label: format!("Project: {label} · connected on this device"),
                    action: None,
                });
            }
        }
        if self.is_authority() {
            rows.push(PanelRow {
                label: "Invite a teammate".into(),
                action: Some(TeamAction::Invite),
            });
            rows.push(PanelRow {
                label: "Review join requests".into(),
                action: Some(TeamAction::Requests),
            });
        }
        for (label, action) in [
            ("Review project questions", TeamAction::Questions),
            ("Connect a project", TeamAction::ConnectProject),
            ("Edit my name", TeamAction::EditName),
            ("Join another team", TeamAction::Join),
            ("Create another team", TeamAction::Create),
            ("Switch team", TeamAction::Switch),
        ] {
            rows.push(PanelRow {
                label: label.into(),
                action: Some(action),
            });
        }
        let selected = rows
            .iter()
            .position(|row| row.action.is_some())
            .unwrap_or(0);
        TeamPanel {
            title: "People and team actions".into(),
            rows,
            selected,
        }
    }

    fn local_projects(&self) -> Result<Vec<crate::team::RepositoryApproval>, String> {
        let store = TeamStore::open_read_only(&self.database).map_err(|error| error.to_string())?;
        let mut seen = std::collections::HashSet::new();
        Ok(store
            .repository_approvals()
            .map_err(|error| error.to_string())?
            .into_iter()
            .filter(|project| {
                project.owner_member_id == self.member_id && seen.insert(project.local_path.clone())
            })
            .collect())
    }

    fn approved_members(&self) -> std::collections::HashSet<String> {
        let mut approved = std::collections::HashSet::new();
        if let Ok(store) = TeamStore::open_read_only(&self.database) {
            if let Ok(Some(team)) = store.team(&self.team_id) {
                approved.insert(team.authority_member_id);
            }
            if let Ok(events) = store.team_events(&self.team_id, 20_000) {
                for event in events {
                    if event.event_type == crate::team::EventType::MemberAdded {
                        if let Some(id) = event
                            .payload
                            .get("member_id")
                            .and_then(|value| value.as_str())
                        {
                            if event
                                .payload
                                .get("active")
                                .and_then(|value| value.as_bool())
                                == Some(true)
                            {
                                approved.insert(id.to_owned());
                            } else {
                                approved.remove(id);
                            }
                        }
                    }
                }
            }
        }
        approved
    }

    fn is_authority(&self) -> bool {
        TeamStore::open_read_only(&self.database)
            .ok()
            .and_then(|store| store.team(&self.team_id).ok().flatten())
            .is_some_and(|team| team.authority_member_id == self.member_id)
    }

    fn start_action(&mut self, action: TeamAction) {
        if self.action.is_some() && !self.action_is_poll {
            self.notice = "Another team action is still running".into();
            return;
        }
        if self.action_is_poll {
            self.action = None;
        }
        let service = self.service.clone();
        self.action_is_poll = matches!(&action, TeamAction::PollRequests);
        let long = matches!(
            &action,
            TeamAction::RunLocal(_, _)
                | TeamAction::RunQuestion(
                    _,
                    crate::cli::questions::QuestionDecision::AllowOnce
                        | crate::cli::questions::QuestionDecision::AllowProject
                )
        );
        self.action = Some(match action {
            TeamAction::Invite => crate::crewai_service::dispatch(move || {
                service.sync()?;
                service.invite().map(TeamActionResult::Invite)
            }),
            TeamAction::Requests => crate::crewai_service::dispatch(move || {
                service
                    .join_requests()
                    .map(|inbox| TeamActionResult::Requests(inbox, true))
            }),
            TeamAction::PollRequests => crate::crewai_service::dispatch(move || {
                service
                    .join_requests()
                    .map(|inbox| TeamActionResult::Requests(inbox, false))
            }),
            TeamAction::Decide(request, approve) => crate::crewai_service::dispatch(move || {
                service
                    .decide_join(&request, approve)
                    .map(|_| TeamActionResult::Decision(approve))
            }),
            TeamAction::RunQuestion(question, decision) => {
                crate::crewai_service::dispatch(move || {
                    Ok(TeamActionResult::QuestionFinished(service.decide_question(
                        &question.request_id,
                        decision,
                        decision != crate::cli::questions::QuestionDecision::Deny,
                    )))
                })
            }
            TeamAction::RunLocal(repository_id, question) => {
                let command = self.composer.text.clone();
                let id = match self.record_command(&command) {
                    Ok(id) => id,
                    Err(error) => {
                        self.notice = error;
                        return;
                    }
                };
                self.command_history.push(command.clone());
                self.composer.clear();
                let _ = self.save_draft();
                crate::crewai_service::dispatch(move || {
                    Ok(TeamActionResult::LocalAnswer(
                        id,
                        command,
                        service.answer_local(&repository_id, &question, true),
                    ))
                })
            }
            _ => return,
        });
        if !self.action_is_poll {
            self.notice = if long {
                "Codex is answering…"
            } else {
                "Working…"
            }
            .into();
        }
    }

    fn poll_action(&mut self) {
        let Some(action) = &self.action else { return };
        while let Ok(update) = action.updates.try_recv() {
            if let OperationUpdate::Finished(result) = update {
                self.action = None;
                match result {
                    Ok(TeamActionResult::Invite(code)) => {
                        self.invite_code = Some(code);
                        self.invite_created_at = Some(std::time::Instant::now());
                        self.notice =
                            "Share the invite code privately; Ctrl+Y requests copy".into();
                    }
                    Ok(TeamActionResult::Requests(inbox, show)) => {
                        let rows = inbox
                            .requests
                            .into_iter()
                            .filter(|entry| entry.status == "pending")
                            .map(|entry| PanelRow {
                                label: format!(
                                    "{} · request to join",
                                    entry.request.member.display_name
                                ),
                                action: Some(TeamAction::Review(entry.request)),
                            })
                            .collect::<Vec<_>>();
                        if show {
                            if rows.is_empty() {
                                self.notice = "No teammates waiting to join".into();
                            } else {
                                self.panel = Some(TeamPanel {
                                    title: "Join requests".into(),
                                    rows,
                                    selected: 0,
                                });
                            }
                        } else if !rows.is_empty() {
                            self.notice = format!(
                                "{} teammate(s) waiting to join · /team-mates to review",
                                rows.len()
                            );
                        }
                    }
                    Ok(TeamActionResult::Decision(approved)) => {
                        self.notice = if approved {
                            "Join request approved; syncing"
                        } else {
                            "Join request declined"
                        }
                        .into();
                        self.schedule_sync();
                    }
                    Ok(TeamActionResult::QuestionFinished(result)) => {
                        self.notice = match result {
                            Ok(_) => "Question handled; syncing the result".into(),
                            Err(error) => format!("Could not answer: {error}"),
                        };
                        self.schedule_sync();
                    }
                    Ok(TeamActionResult::LocalAnswer(command_id, command, result)) => {
                        let at = now();
                        let (kind, text, status) = match result {
                            Ok(answer) => (ConversationKind::Answer, answer, "completed"),
                            Err(error) => {
                                self.composer = Composer::from_text(command);
                                let _ = self.save_draft();
                                (ConversationKind::Failure, error, "failed")
                            }
                        };
                        let entry = ConversationEntry {
                            team_id: self.team_id.clone(),
                            thread_id: "main".into(),
                            entry_id: format!("local-answer:{command_id}"),
                            related_id: Some(command_id),
                            kind,
                            speaker_id: Some(self.member_id.clone()),
                            recipient_id: Some(self.member_id.clone()),
                            text,
                            status: status.into(),
                            created_at: at,
                            updated_at: at,
                        };
                        match TeamStore::open(&self.database)
                            .map_err(|e| e.to_string())
                            .and_then(|store| {
                                store
                                    .save_conversation_entry(&entry)
                                    .map_err(|e| e.to_string())
                            }) {
                            Ok(()) => {
                                self.notice = if status == "completed" {
                                    "Local answer ready"
                                } else {
                                    "Could not answer; your question is kept in the composer"
                                }
                                .into()
                            }
                            Err(error) => {
                                self.notice = format!("Could not save the local answer: {error}")
                            }
                        }
                    }
                    Err(error) => {
                        if !self.action_is_poll {
                            self.notice = error;
                        }
                    }
                }
                return;
            }
        }
    }

    fn previous_command(&mut self) {
        if self.command_history.is_empty() {
            return;
        }
        let next = match self.history_index {
            None => {
                self.history_draft = self.composer.text.clone();
                self.command_history.len() - 1
            }
            Some(index) => index.saturating_sub(1),
        };
        self.history_index = Some(next);
        self.composer = Composer::from_text(self.command_history[next].clone());
        self.composer.picker_suppressed = true;
    }

    fn next_command(&mut self) {
        let Some(index) = self.history_index else {
            return;
        };
        if index + 1 < self.command_history.len() {
            self.history_index = Some(index + 1);
            self.composer = Composer::from_text(self.command_history[index + 1].clone());
        } else {
            self.history_index = None;
            self.composer = Composer::from_text(self.history_draft.clone());
        }
        self.composer.picker_suppressed = true;
    }

    fn submit(&mut self) -> bool {
        let input = self.composer.text.trim().to_owned();
        if input.is_empty() {
            return false;
        }
        if !input.starts_with('/') {
            let Some(selected) = self.selected_entry.as_deref() else {
                self.notice = "Choose a message to reply to, or start with /".into();
                return false;
            };
            let Some(message_id) = selected.strip_prefix("message:") else {
                self.notice = "Select a teammate's message to reply".into();
                return false;
            };
            let entries = match self.service.conversation() {
                Ok(entries) => entries,
                Err(error) => {
                    self.notice = error;
                    return false;
                }
            };
            let Some(entry) = entries.iter().find(|entry| {
                entry.entry_id == selected && entry.kind == ConversationKind::Message
            }) else {
                self.notice = "That message is no longer available".into();
                return false;
            };
            let target = if entry.speaker_id.as_deref() == Some(self.member_id.as_str()) {
                entry.recipient_id.as_deref()
            } else {
                entry.speaker_id.as_deref()
            };
            let Some(target) = target else {
                self.notice = "The reply recipient is unavailable".into();
                return false;
            };
            match self
                .service
                .send_direct(target, Some(message_id.to_owned()), &input)
            {
                Ok(_) => {
                    self.notice = "Reply saved; syncing with your teammate".into();
                    self.composer.clear();
                    self.history_index = None;
                    let _ = self.save_draft();
                    return true;
                }
                Err(error) => {
                    self.notice = error;
                    return false;
                }
            }
        }
        let command = match crewai_command::parse(&input) {
            Ok(value) => value,
            Err(error) => {
                self.notice = error.into();
                return false;
            }
        };
        if let Command::Read {
            question,
            target: None,
        } = &command
        {
            let projects = match self.local_projects() {
                Ok(projects) => projects,
                Err(error) => {
                    self.notice = error;
                    return false;
                }
            };
            if projects.is_empty() {
                self.panel = Some(TeamPanel {
                    title: "Connect a project first".into(),
                    rows: vec![PanelRow {
                        label: "Connect a folder on this device".into(),
                        action: Some(TeamAction::ConnectForRead(question.clone())),
                    }],
                    selected: 0,
                });
                return false;
            }
            if projects.len() == 1 {
                self.panel = Some(local_confirmation(
                    projects[0].repository_id.clone(),
                    question.clone(),
                ));
            } else {
                let rows = projects
                    .into_iter()
                    .map(|project| {
                        let name = std::path::Path::new(&project.local_path)
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("Project")
                            .to_owned();
                        PanelRow {
                            label: name,
                            action: Some(TeamAction::PrepareLocal(
                                project.repository_id,
                                question.clone(),
                            )),
                        }
                    })
                    .collect();
                self.panel = Some(TeamPanel {
                    title: "Choose your project".into(),
                    rows,
                    selected: 0,
                });
            }
            return false;
        }
        let target = match &command {
            Command::Task { target, .. } | Command::Message { target, .. } => Some(target.as_str()),
            Command::Read {
                target: Some(target),
                ..
            } => Some(target.as_str()),
            _ => None,
        };
        let selected_id = if let Some(target) = target {
            let members = match self.service.teammates() {
                Ok(members) => members,
                Err(error) => {
                    self.notice = error;
                    return false;
                }
            };
            let chosen = self
                .composer
                .selected_member
                .as_ref()
                .filter(|(_, name, _)| name.eq_ignore_ascii_case(target))
                .and_then(|(id, _, _)| crewai_command::resolve_selected_member(id, &members).ok());
            match chosen.or_else(|| crewai_command::resolve_member(target, &members).ok()) {
                Some(member) => Some(member.member_id.clone()),
                None => {
                    self.forced_picker = self.composer.target_picker(target, &members);
                    self.notice = if self.forced_picker.is_some() {
                        "Choose a teammate".into()
                    } else {
                        "No approved teammate matches; type @ to choose".into()
                    };
                    return false;
                }
            }
        } else {
            None
        };
        if let (
            Command::Read {
                question,
                target: Some(name),
            },
            Some(id),
        ) = (&command, selected_id.as_ref())
        {
            match self.service.projects(id) {
                Ok(projects) if projects.len() > 1 => {
                    let rows = projects
                        .into_iter()
                        .map(|project| PanelRow {
                            label: project.label,
                            action: Some(TeamAction::SendRemote(
                                id.clone(),
                                project.repository_id,
                                question.clone(),
                                name.clone(),
                            )),
                        })
                        .collect();
                    self.panel = Some(TeamPanel {
                        title: format!("Choose {name}'s project"),
                        rows,
                        selected: 0,
                    });
                    return false;
                }
                Err(error) => {
                    self.notice = error;
                    return false;
                }
                _ => {}
            }
        }
        let result: Result<String, String> = (|| {
            match command {
            Command::TeamMates => {self.panel=Some(self.team_panel());Ok("Choose a team action or review your teammates".into())},
            Command::Broadcast{message} => {let saved=self.service.broadcast(&message)?; if saved.queued {Ok("Message saved; syncing with the team".into())} else {Ok("Message saved locally; queue unavailable".into())}},
            Command::Task{target,description} => {let id=selected_id.as_deref().ok_or("Choose a teammate")?; let saved=self.service.assign_task(id,&description)?; Ok(if saved.queued {format!("Task for {target} saved; syncing")} else {"Task saved locally; queue unavailable".into()})},
            Command::Message{target,message} => {let id=selected_id.as_deref().ok_or("Choose a teammate")?; self.service.send_direct(id,None,&message)?; Ok(format!("Message to {target} saved; syncing"))},
            Command::Read{question,target:Some(target)} => {let id=selected_id.as_deref().ok_or("Choose a teammate")?; let projects=self.service.projects(id)?; if projects.len()!=1 {return Err(if projects.is_empty(){"This teammate has not connected a project yet"}else{"This teammate has multiple projects; choose one in the Team dashboard for now"}.into())}; self.service.ask_remote(id,&projects[0].repository_id,&question)?; Ok(format!("Question saved; syncing for {target}'s approval"))},
            Command::Read{target:None,..} => Err("Local questions require project and assistant setup; use the Team dashboard for now".into()),
        }
        })();
        match result {
            Ok(notice) => {
                if let Err(error) = self.record_command(&input) {
                    self.notice = format!("{notice}; history could not be saved: {error}");
                } else {
                    self.notice = notice;
                }
                self.command_history.push(input);
                self.history_index = None;
                self.composer.clear();
                self.forced_picker = None;
                self.scroll_up = 0;
                if let Err(error) = self.save_draft() {
                    self.notice = format!("{}; draft could not be cleared: {error}", self.notice);
                }
                true
            }
            Err(error) => {
                self.notice = error;
                false
            }
        }
    }
}

fn cursor_position(text: &str, width: u16) -> (u16, u16) {
    let width = width.max(1);
    let (mut row, mut col) = (0u16, 0u16);
    for character in text.chars() {
        if character == '\n' {
            row = row.saturating_add(1);
            col = 0;
            continue;
        }
        let size = character.width().unwrap_or(0).min(width as usize) as u16;
        if col.saturating_add(size) > width {
            row = row.saturating_add(1);
            col = 0;
        }
        col = col.saturating_add(size);
        if col >= width {
            row = row.saturating_add(1);
            col = 0;
        }
    }
    (row, col)
}

pub(crate) fn run() -> io::Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(io::Error::other("crewai needs an interactive terminal"));
    }
    let _guard = ScreenGuard::enter()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let mut session = loop {
        let current = crate::team_view::default_database_path();
        match Session::open(current.clone()) {
            Ok(session) => break session,
            Err(error) => {
                let base = crate::team_view::base_database_path();
                if !crate::crewai_onboarding::welcome(&mut terminal, &base, &current)? {
                    return Ok(());
                }
                // An actual storage failure should not be hidden by an endless welcome loop.
                if let Err(next) = Session::open(crate::team_view::default_database_path()) {
                    return Err(io::Error::other(format!("{error}; {next}")));
                }
            }
        }
    };
    let mut last_refresh = std::time::Instant::now();
    let mut last_join_poll = std::time::Instant::now()
        .checked_sub(Duration::from_secs(15))
        .unwrap_or_else(std::time::Instant::now);
    loop {
        session.poll_sync();
        session.poll_action();
        if session
            .invite_created_at
            .is_some_and(|created| created.elapsed() >= Duration::from_secs(600))
        {
            session.invite_code = None;
            session.invite_created_at = None;
            session.notice = "Invite expired; create another if needed".into();
        }
        if last_join_poll.elapsed() >= Duration::from_secs(15)
            && session.action.is_none()
            && session.is_authority()
        {
            session.start_action(TeamAction::PollRequests);
            last_join_poll = std::time::Instant::now();
        }
        let entries = session.service.conversation().unwrap_or_else(|error| {
            session.notice = format!("Could not load history: {error}");
            Vec::new()
        });
        let members = session.service.teammates().unwrap_or_default();
        let picker = if session.panel.is_none() {
            session
                .forced_picker
                .clone()
                .or_else(|| session.composer.picker(&members))
        } else {
            None
        };
        let mut picker_area: Option<(Rect, usize)> = None;
        let mut panel_area: Option<(Rect, usize)> = None;
        let mut transcript_area: Option<(Rect, u16)> = None;
        let mut transcript_rows: Vec<(String, usize, usize)> = Vec::new();
        terminal.draw(|frame| {
            let width = frame.area().width.saturating_sub(2).max(1);
            let input_rows = Paragraph::new(session.composer.text.as_str())
                .wrap(Wrap { trim: false })
                .line_count(width)
                .max(1);
            let input_height = (input_rows.min(4) as u16).saturating_add(2);
            let chunks = Layout::vertical([
                Constraint::Length(2),
                Constraint::Min(2),
                Constraint::Length(input_height),
                Constraint::Length(1),
            ])
            .split(frame.area());
            let header = format!(
                " crewAI  |  Team: {}  |  You: {}",
                session.team_name, session.member_name
            );
            frame.render_widget(
                Paragraph::new(header)
                    .style(Style::default().fg(Color::Cyan))
                    .block(Block::default().borders(Borders::BOTTOM)),
                chunks[0],
            );
            let mut lines = Vec::new();
            let mut visual_row = 0usize;
            for entry in &entries {
                let first_line = lines.len();
                let speaker = entry
                    .speaker_id
                    .as_ref()
                    .and_then(|id| members.iter().find(|member| &member.member_id == id))
                    .map(|member| member.display_name.as_str())
                    .unwrap_or("Teammate");
                let label = if entry.kind == ConversationKind::Answer {
                    format!("crewAI · {speaker}'s project")
                } else if entry.speaker_id.as_deref() == Some(session.member_id.as_str()) {
                    "You".to_owned()
                } else {
                    match entry.kind {
                        ConversationKind::Message => speaker.to_owned(),
                        ConversationKind::Task => format!("Task · {speaker}"),
                        ConversationKind::Broadcast => format!("Team · {speaker}"),
                        ConversationKind::Approval => format!("Question · {speaker}"),
                        _ => "crewAI".to_owned(),
                    }
                };
                let selected = session.selected_entry.as_deref() == Some(entry.entry_id.as_str());
                lines.push(Line::from(Span::styled(
                    label,
                    Style::default()
                        .fg(if selected { Color::Black } else { Color::Cyan })
                        .bg(if selected { Color::Cyan } else { Color::Reset })
                        .add_modifier(Modifier::BOLD),
                )));
                lines.extend(entry.text.lines().map(|line| Line::raw(display_safe(line))));
                if !entry.status.is_empty()
                    && entry.status != "recorded"
                    && entry.status != "submitted"
                {
                    lines.push(Line::styled(
                        display_safe(&entry.status),
                        Style::default().fg(Color::DarkGray),
                    ));
                }
                lines.push(Line::raw(""));
                let height = Paragraph::new(lines[first_line..].to_vec())
                    .wrap(Wrap { trim: false })
                    .line_count(chunks[1].width.max(1));
                transcript_rows.push((
                    entry.entry_id.clone(),
                    visual_row,
                    visual_row.saturating_add(height),
                ));
                visual_row = visual_row.saturating_add(height);
            }
            if lines.is_empty() {
                lines.push(Line::raw("Type /team-mates to see your team."));
            }
            let transcript = Paragraph::new(lines).wrap(Wrap { trim: false });
            let bottom = (transcript
                .line_count(chunks[1].width.max(1))
                .min(u16::MAX as usize) as u16)
                .saturating_sub(chunks[1].height)
                .saturating_sub(session.scroll_up);
            frame.render_widget(transcript.scroll((bottom, 0)), chunks[1]);
            transcript_area = Some((chunks[1], bottom));
            let (cursor_row, cursor_col) = cursor_position(
                &session.composer.text[..session.composer.cursor],
                chunks[2].width.saturating_sub(2),
            );
            let inner_height = chunks[2].height.saturating_sub(2);
            let input_scroll = cursor_row.saturating_sub(inner_height.saturating_sub(1));
            let title = session
                .selected_entry
                .as_ref()
                .and_then(|id| {
                    entries.iter().find(|entry| {
                        &entry.entry_id == id && entry.kind == ConversationKind::Message
                    })
                })
                .and_then(|entry| {
                    let target = if entry.speaker_id.as_deref() == Some(session.member_id.as_str())
                    {
                        entry.recipient_id.as_ref()
                    } else {
                        entry.speaker_id.as_ref()
                    }?;
                    members
                        .iter()
                        .find(|member| &member.member_id == target)
                        .map(|member| format!(" replying to {} ", member.display_name))
                })
                .unwrap_or_else(|| " message ".into());
            frame.render_widget(
                Paragraph::new(session.composer.text.as_str())
                    .block(Block::default().borders(Borders::ALL).title(title))
                    .wrap(Wrap { trim: false })
                    .scroll((input_scroll, 0)),
                chunks[2],
            );
            if chunks[2].width > 2 && chunks[2].height > 2 {
                frame.set_cursor_position((
                    chunks[2].x + 1 + cursor_col.min(chunks[2].width - 3),
                    chunks[2].y
                        + 1
                        + cursor_row
                            .saturating_sub(input_scroll)
                            .min(inner_height - 1),
                ));
            }
            let footer = if let Some(code) = &session.invite_code {
                format!(
                    "Invite: {code} (10 minutes, one use) · Ctrl+Y requests copy | {}",
                    session.notice
                )
            } else {
                format!(
                    "{}  |  Click a message or question",
                    display_safe(&session.notice)
                )
            };
            frame.render_widget(
                Paragraph::new(footer).style(Style::default().fg(Color::DarkGray)),
                chunks[3],
            );
            if let Some(picker) = &picker {
                let visible = picker
                    .options
                    .len()
                    .min(5)
                    .min(chunks[1].height.saturating_sub(2) as usize);
                if visible > 0 {
                    let first = picker.selected.saturating_sub(visible - 1);
                    let area = Rect {
                        x: chunks[1].x,
                        y: chunks[2].y.saturating_sub(visible as u16 + 2),
                        width: chunks[1].width.min(58),
                        height: visible as u16 + 2,
                    };
                    let options = picker
                        .options
                        .iter()
                        .enumerate()
                        .skip(first)
                        .take(visible)
                        .map(|(i, option)| {
                            let prefix = if i == picker.selected { "> " } else { "  " };
                            let style = if i == picker.selected {
                                Style::default().fg(Color::Black).bg(Color::Cyan)
                            } else {
                                Style::default()
                            };
                            Line::styled(
                                format!("{prefix}{}  {}", option.label, option.description),
                                style,
                            )
                        })
                        .collect::<Vec<_>>();
                    frame.render_widget(Clear, area);
                    frame.render_widget(
                        Paragraph::new(options)
                            .block(Block::default().borders(Borders::ALL).title(" Select ")),
                        area,
                    );
                    picker_area = Some((area, first));
                }
            }
            if let Some(panel) = &session.panel {
                let visible = panel
                    .rows
                    .len()
                    .min(10)
                    .min(frame.area().height.saturating_sub(4) as usize);
                if visible > 0 {
                    let first = panel.selected.saturating_sub(visible - 1);
                    let width = frame.area().width.saturating_sub(4).min(72);
                    let height = visible as u16 + 2;
                    let area = Rect {
                        x: frame.area().x + (frame.area().width - width) / 2,
                        y: frame.area().y + (frame.area().height - height) / 2,
                        width,
                        height,
                    };
                    let rows = panel
                        .rows
                        .iter()
                        .enumerate()
                        .skip(first)
                        .take(visible)
                        .map(|(index, row)| {
                            let marker = if index == panel.selected { "> " } else { "  " };
                            let style = if index == panel.selected {
                                Style::default().fg(Color::Black).bg(Color::Cyan)
                            } else if row.action.is_none() {
                                Style::default().fg(Color::DarkGray)
                            } else {
                                Style::default()
                            };
                            Line::styled(format!("{marker}{}", display_safe(&row.label)), style)
                        })
                        .collect::<Vec<_>>();
                    frame.render_widget(Clear, area);
                    frame.render_widget(
                        Paragraph::new(rows).block(
                            Block::default()
                                .borders(Borders::ALL)
                                .title(panel.title.as_str()),
                        ),
                        area,
                    );
                    panel_area = Some((area, first));
                }
            }
        })?;
        if event::poll(Duration::from_millis(200))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if session.panel.is_some() {
                        match key.code {
                            KeyCode::Esc => session.panel = None,
                            KeyCode::Up => {
                                if let Some(panel) = &mut session.panel {
                                    panel.selected = panel.selected.saturating_sub(1);
                                }
                            }
                            KeyCode::Down => {
                                if let Some(panel) = &mut session.panel {
                                    panel.selected = (panel.selected + 1)
                                        .min(panel.rows.len().saturating_sub(1));
                                }
                            }
                            KeyCode::Enter => {
                                let action = session
                                    .panel
                                    .as_ref()
                                    .and_then(|panel| panel.rows.get(panel.selected))
                                    .and_then(|row| row.action.clone());
                                if let Some(action) = action {
                                    session.panel = None;
                                    perform_team_action(&mut session, &mut terminal, action)?;
                                }
                            }
                            _ => {}
                        }
                        continue;
                    }
                    match (key.code, key.modifiers) {
                        (KeyCode::Esc, _) if picker.is_some() => {
                            session.forced_picker = None;
                            session.composer.picker_suppressed = true;
                        }
                        (KeyCode::Esc, _) if session.selected_entry.is_some() => {
                            session.selected_entry = None;
                            session.notice = "Reply selection cleared".into();
                        }
                        (KeyCode::Esc, _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                            session.save_draft().map_err(io::Error::other)?;
                            break;
                        }
                        (KeyCode::Enter, KeyModifiers::SHIFT) => {
                            session.forced_picker = None;
                            session.composer.insert("\n");
                        }
                        (KeyCode::Enter, _) | (KeyCode::Tab, _) if picker.is_some() => {
                            if let Some(picker) = &picker {
                                session.composer.choose(picker, picker.selected);
                                session.forced_picker = None;
                            }
                        }
                        (KeyCode::Enter, _) => {
                            if session.submit() {
                                session.schedule_sync();
                            }
                        }
                        (KeyCode::Char('y'), KeyModifiers::CONTROL)
                            if session.invite_code.is_some() =>
                        {
                            if let Some(code) = &session.invite_code {
                                let encoded = base64::engine::general_purpose::STANDARD
                                    .encode(code.as_bytes());
                                execute!(
                                    io::stdout(),
                                    crossterm::style::Print(format!("\x1b]52;c;{encoded}\x07"))
                                )?;
                                session.notice =
                                    "Copy requested; if unavailable, enter the code manually"
                                        .into();
                            }
                        }
                        (KeyCode::Up, _) if picker.is_some() => {
                            if let Some(forced) = &mut session.forced_picker {
                                forced.selected = forced.selected.saturating_sub(1)
                            } else {
                                session.composer.picker_index =
                                    session.composer.picker_index.saturating_sub(1)
                            }
                        }
                        (KeyCode::Down, _) if picker.is_some() => {
                            if let Some(picker) = &picker {
                                if let Some(forced) = &mut session.forced_picker {
                                    forced.selected =
                                        (forced.selected + 1).min(picker.options.len() - 1)
                                } else {
                                    session.composer.picker_index = (session.composer.picker_index
                                        + 1)
                                    .min(picker.options.len() - 1)
                                }
                            }
                        }
                        (KeyCode::Up, _) => session.previous_command(),
                        (KeyCode::Down, _) => session.next_command(),
                        (KeyCode::Left, _) => {
                            session.forced_picker = None;
                            session.composer.left();
                        }
                        (KeyCode::Right, _) => {
                            session.forced_picker = None;
                            session.composer.right();
                        }
                        (KeyCode::Home, _) => {
                            session.forced_picker = None;
                            session.composer.home();
                        }
                        (KeyCode::End, _) => {
                            session.forced_picker = None;
                            session.composer.end();
                        }
                        (KeyCode::Backspace, _) => {
                            session.forced_picker = None;
                            session.composer.backspace();
                        }
                        (KeyCode::Delete, _) => {
                            session.forced_picker = None;
                            session.composer.delete();
                        }
                        (KeyCode::Char(c), m)
                            if !m.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                        {
                            session.forced_picker = None;
                            if !session.composer.insert(&c.to_string()) {
                                session.notice = "Input is invalid or too long".into();
                            }
                        }
                        (KeyCode::PageUp, _) => {
                            session.scroll_up = session.scroll_up.saturating_add(5)
                        }
                        (KeyCode::PageDown, _) => {
                            session.scroll_up = session.scroll_up.saturating_sub(5)
                        }
                        _ => {}
                    }
                }
                Event::Paste(text) => {
                    session.forced_picker = None;
                    if !session.composer.insert(&text.replace("\r\n", "\n")) {
                        session.notice = "Paste is invalid or too long".into();
                    }
                }
                Event::Mouse(mouse)
                    if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) =>
                {
                    if let (Some(panel), Some((area, first))) = (&session.panel, panel_area) {
                        if mouse.column > area.x
                            && mouse.column < area.x + area.width.saturating_sub(1)
                            && mouse.row > area.y
                            && mouse.row < area.y + area.height.saturating_sub(1)
                        {
                            let index = first + (mouse.row - area.y - 1) as usize;
                            let action = panel.rows.get(index).and_then(|row| row.action.clone());
                            if let Some(action) = action {
                                session.panel = None;
                                perform_team_action(&mut session, &mut terminal, action)?;
                            } else if let Some(panel) = &mut session.panel {
                                panel.selected = index.min(panel.rows.len().saturating_sub(1));
                            }
                        }
                        continue;
                    }
                    let mut selected_picker = false;
                    if let (Some(picker), Some((area, first))) = (&picker, picker_area) {
                        if mouse.column > area.x
                            && mouse.column < area.x + area.width.saturating_sub(1)
                            && mouse.row > area.y
                            && mouse.row < area.y + area.height.saturating_sub(1)
                        {
                            let index = first + (mouse.row - area.y - 1) as usize;
                            session.composer.choose(picker, index);
                            session.forced_picker = None;
                            selected_picker = true;
                        }
                    }
                    if !selected_picker {
                        if let Some((area, scroll)) = transcript_area {
                            if mouse.column >= area.x
                                && mouse.column < area.x + area.width
                                && mouse.row >= area.y
                                && mouse.row < area.y + area.height
                            {
                                let row = usize::from(mouse.row - area.y) + usize::from(scroll);
                                if let Some((id, _, _)) = transcript_rows
                                    .iter()
                                    .find(|(_, start, end)| row >= *start && row < *end)
                                {
                                    if entries.iter().any(|entry| {
                                        &entry.entry_id == id
                                            && entry.kind == ConversationKind::Message
                                    }) {
                                        session.selected_entry = Some(id.clone());
                                        session.notice =
                                            "Type a reply or use a slash command".into();
                                    } else if let Some(request_id) = id.strip_prefix("question:") {
                                        if let Ok(questions) = session.service.questions() {
                                            if let Some(question) =
                                                questions.into_iter().find(|question| {
                                                    question.request_id == request_id
                                                        && question.target_member_id
                                                            == session.member_id
                                                        && matches!(
                                                            question.delivery.as_str(),
                                                            "Created" | "Queued" | "Delivered"
                                                        )
                                                })
                                            {
                                                session.panel = Some(question_review(question));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        if last_refresh.elapsed() >= Duration::from_secs(5) {
            session.schedule_sync();
            if let Ok(questions) = session.service.questions() {
                let pending = questions
                    .into_iter()
                    .filter(|question| {
                        question.target_member_id == session.member_id
                            && matches!(
                                question.delivery.as_str(),
                                "Created" | "Queued" | "Delivered"
                            )
                    })
                    .map(|question| question.request_id)
                    .collect::<std::collections::HashSet<_>>();
                if pending
                    .iter()
                    .any(|id| !session.noticed_questions.contains(id))
                {
                    session.notice = format!(
                        "{} project question(s) waiting · click a question to review",
                        pending.len()
                    );
                }
                session.noticed_questions = pending;
            }
            last_refresh = std::time::Instant::now();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::DeviceIdentity;
    use crate::team::{Device, Member, Role, Team};

    #[test]
    fn composer_dispatches_real_records_and_preserves_invalid_drafts() {
        let root = std::env::temp_dir().join(format!(
            "crewai-terminal-{}",
            crate::cli::team::generate_token()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let database = root.join("team.sqlite");
        let owner =
            crate::team_session::setup_profile(&database, "Manager", Role::Manager).unwrap();
        let owner_key =
            DeviceIdentity::load_existing(crate::team_session::identity_path(&database)).unwrap();
        let peer_key = DeviceIdentity::load_or_create(root.join("peer.identity")).unwrap();
        let peer = Member {
            member_id: "peer".into(),
            display_name: "Hemant".into(),
            role: Role::Implementer,
            active: true,
        };
        let team = Team::create("Test", &owner.member_id, 1).unwrap();
        let store = TeamStore::open(&database).unwrap();
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
        drop(store);
        let mut session = Session::open(database.clone()).unwrap();
        let team_panel = session.team_panel();
        assert!(team_panel
            .rows
            .iter()
            .any(|row| row.label == "Invite a teammate"));
        assert!(team_panel
            .rows
            .iter()
            .any(|row| row.label == "Review join requests"));
        session.composer = Composer::from_text("/broadcast Hello team".into());
        assert!(session.submit());
        session.composer = Composer::from_text("/task @Hemant Fix the test".into());
        assert!(session.submit());
        session.composer = Composer::from_text("/message @Hemant Hello privately".into());
        assert!(session.submit());
        let store = TeamStore::open(&database).unwrap();
        assert_eq!(store.broadcasts(None).unwrap()[0].message, "Hello team");
        assert_eq!(store.tasks().unwrap()[0].description, "Fix the test");
        assert_eq!(
            store.direct_messages(&team.team_id).unwrap()[0].0.body,
            "Hello privately"
        );
        let root_id = store.direct_messages(&team.team_id).unwrap()[0]
            .0
            .message_id
            .clone();
        session.selected_entry = Some(format!("message:{root_id}"));
        session.composer = Composer::from_text("A real follow-up".into());
        assert!(session.submit());
        assert!(store
            .direct_messages(&team.team_id)
            .unwrap()
            .iter()
            .any(|(message, _)| message.body == "A real follow-up"
                && message.reply_to.as_deref() == Some(root_id.as_str())));
        session.selected_entry = None;
        session.composer = Composer::from_text("Unaddressed text".into());
        assert!(!session.submit());
        assert_eq!(session.composer.text, "Unaddressed text");
        let duplicate = Member {
            member_id: "peer-two".into(),
            display_name: "Hemant".into(),
            role: Role::Implementer,
            active: true,
        };
        let duplicate_key = DeviceIdentity::load_or_create(root.join("peer-two.identity")).unwrap();
        store.save_member(&team.team_id, &duplicate, 2).unwrap();
        store
            .save_device(
                &Device {
                    device_id: duplicate_key.device_id().into(),
                    member_id: duplicate.member_id.clone(),
                    public_key: duplicate_key.public_key(),
                    active: true,
                },
                2,
            )
            .unwrap();
        session.composer = Composer::from_text("/task @Hemant Choose the right person".into());
        assert!(!session.submit());
        let picker = session
            .forced_picker
            .clone()
            .expect("ambiguous names open the picker");
        let index = picker
            .options
            .iter()
            .position(|option| option.member_id.as_deref() == Some("peer-two"))
            .unwrap();
        assert!(session.composer.choose(&picker, index));
        session.forced_picker = None;
        assert!(session.submit());
        assert!(store
            .tasks()
            .unwrap()
            .iter()
            .any(|task| task.description == "Choose the right person"
                && task.assignee_member_id == "peer-two"));
        session.composer = Composer::from_text("/task @missing Work".into());
        assert!(!session.submit());
        assert_eq!(session.composer.text, "/task @missing Work");
        session.save_draft().unwrap();
        assert_eq!(
            Session::open(database).unwrap().composer.text,
            "/task @missing Work"
        );
    }
}
