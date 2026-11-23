//! Guided in-pane onboarding. Network work runs away from terminal input.
use crate::crewai_composer::Composer;
use crate::crewai_service::{dispatch, OperationUpdate, TeamService};
use crate::team::Role;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Layout},
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph, Wrap},
    Terminal,
};
use std::io::{self, Stdout};
use std::path::{Path, PathBuf};
use std::time::Duration;
use unicode_width::UnicodeWidthStr;

pub(crate) type PaneTerminal = Terminal<CrosstermBackend<Stdout>>;

/// Returns None on Escape. The prompt never writes invite codes into history.
pub(crate) fn prompt(
    terminal: &mut PaneTerminal,
    title: &str,
    help: &str,
    initial: &str,
) -> io::Result<Option<String>> {
    let mut input = Composer::from_text(initial.to_owned());
    loop {
        terminal.draw(|frame| {
            let area = frame.area();
            let rows = Layout::vertical([
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Min(1),
            ])
            .split(area);
            frame.render_widget(
                Paragraph::new(help)
                    .wrap(Wrap { trim: false })
                    .block(Block::default().borders(Borders::BOTTOM).title(title)),
                rows[0],
            );
            frame.render_widget(
                Paragraph::new(input.text.as_str())
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(" Enter value "),
                    )
                    .wrap(Wrap { trim: false }),
                rows[1],
            );
            frame.render_widget(
                Paragraph::new("Enter to continue  ·  Esc to cancel")
                    .style(Style::default().fg(Color::DarkGray)),
                rows[2],
            );
            if rows[1].width > 2 && rows[1].height > 2 {
                frame.set_cursor_position((
                    rows[1].x
                        + 1
                        + (input.text[..input.cursor].width().min(u16::MAX as usize) as u16)
                            .min(rows[1].width - 3),
                    rows[1].y + 1,
                ));
            }
        })?;
        if event::poll(Duration::from_millis(200))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    match (key.code, key.modifiers) {
                        (KeyCode::Esc, _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                            return Ok(None)
                        }
                        (KeyCode::Enter, _) => return Ok(Some(input.text.trim().to_owned())),
                        (KeyCode::Backspace, _) => input.backspace(),
                        (KeyCode::Delete, _) => input.delete(),
                        (KeyCode::Left, _) => input.left(),
                        (KeyCode::Right, _) => input.right(),
                        (KeyCode::Home, _) => input.home(),
                        (KeyCode::End, _) => input.end(),
                        (KeyCode::Char(ch), modifiers)
                            if !modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                                && input.text.len() + ch.len_utf8() <= 4096 =>
                        {
                            input.insert(&ch.to_string());
                        }
                        _ => {}
                    }
                }
                Event::Paste(value) => {
                    let value = value.replace(['\r', '\n'], "");
                    if input.text.len() + value.len() <= 4096 {
                        input.insert(&value);
                    }
                }
                _ => {}
            }
        }
    }
}

/// Shows progress without blocking the UI thread; work is not cancelled midway.
pub(crate) fn wait_for<T: Send + 'static>(
    terminal: &mut PaneTerminal,
    label: &str,
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> io::Result<Result<T, String>> {
    let operation = dispatch(work);
    loop {
        loop {
            match operation.updates.try_recv() {
                Ok(OperationUpdate::Started) => {}
                Ok(OperationUpdate::Finished(result)) => return Ok(result),
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    return Ok(Err(
                        "The operation stopped unexpectedly; please retry".into()
                    ))
                }
            }
        }
        terminal.draw(|frame| {
            frame.render_widget(
                Paragraph::new(format!("{label}\n\nPlease wait…"))
                    .wrap(Wrap { trim: false })
                    .block(Block::default().borders(Borders::ALL).title(" crewAI ")),
                frame.area(),
            )
        })?;
        if event::poll(Duration::from_millis(150))? {
            let _ = event::read()?;
        }
    }
}

pub(crate) fn join(
    terminal: &mut PaneTerminal,
    base: &Path,
    current: &Path,
    name: &str,
) -> io::Result<Result<Option<PathBuf>, String>> {
    let Some(code) = prompt(
        terminal,
        "Join a team",
        "Enter the six-digit code from your manager.",
        "",
    )?
    else {
        return Ok(Ok(None));
    };
    let base = base.to_path_buf();
    let current = current.to_path_buf();
    let name = name.to_owned();
    wait_for(terminal, "Joining your team", move || {
        let bundle = TeamService::preview_invite(&code)?;
        TeamService::join(&base, &current, &name, &bundle).map(Some)
    })
}

pub(crate) fn welcome(
    terminal: &mut PaneTerminal,
    base: &Path,
    current: &Path,
) -> io::Result<bool> {
    let mut note = String::new();
    loop {
        terminal.draw(|frame| {
            let body=format!("Create or join a team from this terminal.\n\n  C  Create a team\n  J  Join with an invite code\n\n{note}\n\nEsc returns to the shell.");
            frame.render_widget(Paragraph::new(body).wrap(Wrap{trim:false}).block(Block::default().borders(Borders::ALL).title(" Welcome to crewAI ")),frame.area());
        })?;
        if !event::poll(Duration::from_millis(200))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        let create = match key.code {
            KeyCode::Char('c' | 'C') => true,
            KeyCode::Char('j' | 'J') => false,
            KeyCode::Esc => return Ok(false),
            _ => continue,
        };
        let Some(name) = prompt(
            terminal,
            "Your name",
            "What should your teammates call you?",
            "",
        )?
        else {
            continue;
        };
        if name.trim().is_empty() {
            note = "Enter your name to continue.".into();
            continue;
        }
        if create {
            let Some(team_name) = prompt(terminal, "Create a team", "Choose a team name.", "")?
            else {
                continue;
            };
            let database = current.to_path_buf();
            let result = wait_for(terminal, "Creating your team", move || {
                TeamService::setup_profile(&database, &name, Role::Manager)?;
                TeamService::create_team(&database, &team_name).map(|_| ())
            })?;
            match result {
                Ok(()) => return Ok(true),
                Err(error) => note = error,
            }
        } else {
            match join(terminal, base, current, &name)? {
                Ok(Some(_)) => return Ok(true),
                Ok(None) => {}
                Err(error) => note = error,
            }
        }
    }
}
