use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
};
use std::{
    path::PathBuf,
    process::Command,
    sync::mpsc,
    time::{Duration, Instant},
};

use super::{render, ClientShellConfig, ClientShellState, ShellHitMap};

const TASK_ROWS: usize = 3;
const BROADCAST_ROWS: usize = 2;

pub(super) fn render_team_panel(
    buffer: &mut Buffer,
    area: Rect,
    snapshot: &Result<crate::team_view::TeamDashboardSnapshot, String>,
    task_scroll: &mut usize,
    activity_scroll: &mut usize,
    composer_text: &str,
    composer_focused: bool,
    status_text: Option<&str>,
    pane_task_links: &std::collections::HashMap<String, String>,
    config: &ClientShellConfig,
    hits: &mut ShellHitMap,
) {
    hits.team_panel_tasks = Rect::default();
    hits.team_panel_tasks_max_scroll = 0;
    hits.team_panel_task_links.clear();
    hits.team_panel_activity = Rect::default();
    hits.team_panel_activity_max_scroll = 0;
    hits.team_panel_composer = Rect::default();
    hits.team_panel_dashboard = Rect::default();
    if area.is_empty() {
        return;
    }
    let palette = &config.palette;
    let base = Style::default().fg(palette.text).bg(palette.panel_bg);
    let muted = Style::default().fg(palette.overlay0).bg(palette.panel_bg);
    let accent = Style::default()
        .fg(palette.accent)
        .bg(palette.panel_bg)
        .add_modifier(Modifier::BOLD);

    for y in area.y..area.bottom() {
        buffer[(area.x, y)].set_symbol("|").set_style(muted);
        for x in area.x.saturating_add(1)..area.right() {
            buffer[(x, y)].set_symbol(" ").set_style(base);
        }
    }
    let x = area.x.saturating_add(2);
    let width = area.width.saturating_sub(4);
    if width < 8 || area.height < 9 {
        return;
    }

    put(buffer, x, area.y, width, "TEAM  [Open dashboard]", accent);
    hits.team_panel_dashboard = Rect::new(x, area.y, width, 1);
    match snapshot {
        Err(_) => {
            put(
                buffer,
                x,
                area.y.saturating_add(1),
                width,
                "Team setup needed",
                base,
            );
            put(
                buffer,
                x,
                area.y.saturating_add(2),
                width,
                "Open menu > team dashboard",
                muted,
            );
        }
        Ok(snapshot) => {
            if let Some(team) = snapshot.teams.first() {
                put(buffer, x, area.y.saturating_add(1), width, &team.name, base);
                let active = snapshot
                    .teammates
                    .iter()
                    .filter(|member| member.active)
                    .count();
                let summary = format!("{active} approved member(s)");
                put(buffer, x, area.y.saturating_add(2), width, &summary, muted);
            } else {
                put(
                    buffer,
                    x,
                    area.y.saturating_add(1),
                    width,
                    "No team set up",
                    base,
                );
                put(
                    buffer,
                    x,
                    area.y.saturating_add(2),
                    width,
                    "Open menu > team dashboard",
                    muted,
                );
            }
        }
    }

    let delivery_line = match snapshot {
        Ok(snapshot) => {
            let delivery = snapshot.delivery;
            format!(
                "Sync q{} r{} active{} sent{} failed{}",
                delivery.queued,
                delivery.retryable,
                delivery.syncing,
                delivery.delivered,
                delivery.failed
            )
        }
        Err(_) => "Sync status unavailable".to_owned(),
    };
    put(
        buffer,
        x,
        area.y.saturating_add(3),
        width,
        &delivery_line,
        muted,
    );

    let tasks_header_y = area.y.saturating_add(4);
    let task_count = snapshot.as_ref().map_or(0, |snapshot| snapshot.tasks.len());
    put(
        buffer,
        x,
        tasks_header_y,
        width,
        &format!("TASKS ({task_count})"),
        accent,
    );
    let task_start = tasks_header_y.saturating_add(1);
    let broadcast_header_y = area
        .bottom()
        .saturating_sub(9)
        .max(task_start.saturating_add(2));
    let tasks_lines = broadcast_header_y
        .saturating_sub(task_start)
        .saturating_sub(1);
    hits.team_panel_tasks = Rect::new(x, task_start, width, tasks_lines);
    match snapshot {
        Ok(snapshot) if !snapshot.tasks.is_empty() => {
            let max_rows = usize::from(tasks_lines / 2).min(TASK_ROWS);
            let window = feed_window(snapshot.tasks.len(), max_rows, task_scroll);
            hits.team_panel_tasks_max_scroll = snapshot.tasks.len().saturating_sub(max_rows);
            for (index, task) in snapshot.tasks[window].iter().rev().enumerate() {
                let y = task_start.saturating_add((index * 2) as u16);
                let status = task_status(&task.status);
                let linked_pane = pane_task_links
                    .iter()
                    .find(|(_, task_id)| *task_id == &task.task_id)
                    .map(|(pane_id, _)| pane_id.clone());
                let meta = format!(
                    "{status} · {}{}",
                    task.assignee_member_id,
                    if linked_pane.is_some() {
                        " · agent ↗"
                    } else {
                        ""
                    }
                );
                put(buffer, x, y, width, &meta, muted);
                put(
                    buffer,
                    x,
                    y.saturating_add(1),
                    width,
                    &task.description,
                    base,
                );
                if let Some(pane_id) = linked_pane {
                    hits.team_panel_task_links.push((
                        Rect::new(
                            x,
                            y,
                            width,
                            2.min(tasks_lines.saturating_sub(index as u16 * 2)),
                        ),
                        task.task_id.clone(),
                        pane_id,
                    ));
                }
            }
        }
        _ => put(buffer, x, task_start, width, "No tasks yet", muted),
    }

    put(buffer, x, broadcast_header_y, width, "BROADCASTS", accent);
    let broadcasts_start = broadcast_header_y.saturating_add(1);
    let composer_y = area.bottom().saturating_sub(4);
    let broadcasts_lines = composer_y
        .saturating_sub(broadcasts_start)
        .saturating_sub(1);
    hits.team_panel_activity = Rect::new(x, broadcasts_start, width, broadcasts_lines);
    match snapshot {
        Ok(snapshot) => {
            // Task completion is already represented by a signed task status
            // event. Merge that derived activity with broadcasts instead of
            // emitting a second event that could be duplicated on relay replay.
            let mut activity = snapshot
                .broadcasts
                .iter()
                .map(|broadcast| {
                    (
                        broadcast.created_at,
                        broadcast.sender_member_id.clone(),
                        broadcast.message.clone(),
                        broadcast.message_id.clone(),
                    )
                })
                .chain(snapshot.completion_announcements.iter().map(|done| {
                    (
                        done.completed_at,
                        format!("DONE · {}", done.completed_by_member_id),
                        format!("Implemented: {}", done.description),
                        done.task_id.clone(),
                    )
                }))
                .collect::<Vec<_>>();
            activity.sort_by(|left, right| {
                left.0
                    .cmp(&right.0)
                    .then_with(|| left.1.cmp(&right.1))
                    .then_with(|| left.2.cmp(&right.2))
                    .then_with(|| left.3.cmp(&right.3))
            });
            let max_rows = usize::from(broadcasts_lines / 2).min(BROADCAST_ROWS);
            let window = feed_window(activity.len(), max_rows, activity_scroll);
            hits.team_panel_activity_max_scroll = activity.len().saturating_sub(max_rows);
            for (index, (_, label, message, _)) in activity[window].iter().rev().enumerate() {
                let y = broadcasts_start.saturating_add((index * 2) as u16);
                put(buffer, x, y, width, &label, muted);
                put(buffer, x, y.saturating_add(1), width, &message, base);
            }
            if activity.is_empty() {
                put(
                    buffer,
                    x,
                    broadcasts_start,
                    width,
                    "No broadcasts yet",
                    muted,
                );
            }
        }
        Err(_) => put(
            buffer,
            x,
            broadcasts_start,
            width,
            "No broadcasts yet",
            muted,
        ),
    }

    let composer = Rect::new(
        x.saturating_sub(1),
        composer_y,
        area.width.saturating_sub(2),
        1,
    );
    hits.team_panel_composer = composer;
    let composer_style = if composer_focused {
        Style::default().fg(palette.text).bg(palette.surface_dim)
    } else {
        Style::default()
            .fg(palette.overlay0)
            .bg(palette.surface_dim)
    };
    buffer.set_style(composer, composer_style);
    let composer_text = if composer_text.is_empty() {
        if composer_focused {
            "Type a message..."
        } else {
            "Click to broadcast..."
        }
    } else {
        composer_text
    };
    put(
        buffer,
        composer.x.saturating_add(1),
        composer.y,
        composer.width.saturating_sub(2),
        composer_text,
        composer_style,
    );
    if let Some(status) = status_text {
        put(
            buffer,
            x,
            area.bottom().saturating_sub(2),
            width,
            status,
            muted,
        );
    } else {
        let hint = if composer_focused {
            "Enter send  Esc cancel"
        } else {
            "Wheel feeds: Ctrl/Alt+PgUp/Dn"
        };
        put(
            buffer,
            x,
            area.bottom().saturating_sub(2),
            width,
            hint,
            muted,
        );
    }
}

fn feed_window(
    total: usize,
    visible: usize,
    scroll_from_latest: &mut usize,
) -> std::ops::Range<usize> {
    let max_scroll = total.saturating_sub(visible);
    *scroll_from_latest = (*scroll_from_latest).min(max_scroll);
    let end = total.saturating_sub(*scroll_from_latest);
    end.saturating_sub(visible)..end
}

#[cfg(test)]
mod tests {
    use super::feed_window;

    #[test]
    fn feed_window_starts_at_latest_and_moves_to_older_items() {
        let mut scroll = 0;
        assert_eq!(feed_window(10, 3, &mut scroll), 7..10);
        assert_eq!(scroll, 0);

        scroll = 2;
        assert_eq!(feed_window(10, 3, &mut scroll), 5..8);
        assert_eq!(scroll, 2);

        scroll = 100;
        assert_eq!(feed_window(10, 3, &mut scroll), 0..3);
        assert_eq!(scroll, 7);
    }

    #[test]
    fn feed_window_handles_empty_and_short_lists() {
        let mut scroll = 8;
        assert_eq!(feed_window(0, 3, &mut scroll), 0..0);
        assert_eq!(scroll, 0);
        assert_eq!(feed_window(2, 3, &mut scroll), 0..2);
    }
}

pub(super) fn render_toggle(
    buffer: &mut Buffer,
    area: Rect,
    open: bool,
    config: &ClientShellConfig,
) {
    if area.is_empty() {
        return;
    }
    let style = Style::default()
        .fg(config.palette.accent)
        .bg(config.palette.panel_bg)
        .add_modifier(Modifier::BOLD);
    let label = if open { "TEAM  x" } else { "TEAM  +" };
    put(buffer, area.x, area.y, area.width, label, style);
}

impl ClientShellState {
    pub(crate) fn set_relay_credential_sink(
        &mut self,
        credential_sink: crate::client::team_relay_sync::RelayCredentialSink,
    ) {
        self.relay_credential_sink = Some(credential_sink);
    }

    pub(crate) fn tick_team_panel(&mut self, now: Instant) -> bool {
        let mut repaint = false;
        if let Some(receiver) = self.team_panel_refresh_pending.as_ref() {
            match receiver.try_recv() {
                Ok(snapshot) => {
                    self.team_panel_refresh_pending = None;
                    self.team_panel_snapshot = snapshot;
                    self.team_panel_next_refresh = now + Duration::from_secs(8);
                    repaint = true;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.team_panel_refresh_pending = None;
                    self.team_panel_next_refresh = now + Duration::from_secs(8);
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if let Some(receiver) = self.team_panel_send_pending.as_ref() {
            match receiver.try_recv() {
                Ok(result) => {
                    self.team_panel_send_pending = None;
                    if result.is_ok() {
                        self.team_panel_activity_scroll = 0;
                        self.team_panel_composer.clear();
                    }
                    self.team_panel_status = Some(match result {
                        Ok(()) => {
                            "Broadcast recorded. Background sync will deliver when connected."
                                .into()
                        }
                        Err(error) => format!("Broadcast failed: {}", compact(&error, 28)),
                    });
                    self.team_panel_next_refresh = now;
                    repaint = true;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.team_panel_send_pending = None;
                    self.team_panel_status = Some("Broadcast process stopped".into());
                    repaint = true;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if self.team_panel_refresh_pending.is_none()
            && self.team_panel_send_pending.is_none()
            && now >= self.team_panel_next_refresh
        {
            self.refresh_team_panel();
            self.team_panel_next_refresh = now + Duration::from_secs(8);
        }
        repaint
    }

    pub(super) fn toggle_team_panel(&mut self, outcome: &mut super::ClientShellInput) {
        let can_show = self
            .last_composed_size
            .map(|(cols, rows)| !self.layout(cols, rows).team_panel_toggle.is_empty())
            .unwrap_or(false);
        if !can_show {
            self.team_panel_status = Some("Widen the window to show the team panel".into());
            outcome.repaint = true;
            return;
        }
        self.team_panel_collapsed = !self.team_panel_collapsed;
        self.team_panel_composer_focused = false;
        self.invalidate_pane_surface();
        self.team_panel_status = None;
        outcome.repaint = true;
        outcome.resize = true;
    }

    pub(super) fn send_team_panel_broadcast(&mut self) {
        let message = self.team_panel_composer.as_str().trim().to_owned();
        self.team_panel_composer_focused = false;
        if message.is_empty() {
            self.team_panel_status = Some("Type a message first".into());
            return;
        }
        if self.team_panel_send_pending.is_some() {
            self.team_panel_status = Some("A broadcast is already being sent".into());
            return;
        }
        let Some((team_id, actor)) = self.team_panel_snapshot.as_ref().ok().and_then(|snapshot| {
            Some((
                snapshot.teams.first()?.team_id.clone(),
                snapshot.profile.as_ref()?.member_id.clone(),
            ))
        }) else {
            self.team_panel_status = Some("Create or join a team first".into());
            return;
        };
        let database = crate::team_view::default_database_path();
        let identity = database.with_file_name(format!(
            "{}.identity.json",
            database
                .file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or("team")
        ));
        let Ok(executable) = std::env::current_exe() else {
            self.team_panel_status = Some("Could not locate Herdr".into());
            return;
        };
        let (sender, receiver) = mpsc::channel();
        thread_broadcast(
            executable,
            database.clone(),
            identity,
            team_id,
            actor,
            message,
            sender,
        );
        self.team_panel_status = Some("Recording broadcast...".into());
        self.team_panel_send_pending = Some(receiver);
    }

    fn refresh_team_panel(&mut self) {
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = crate::team_view::load_default().map_err(|error| error.to_string());
            let _ = sender.send(result);
        });
        self.team_panel_refresh_pending = Some(receiver);
    }
}

fn thread_broadcast(
    executable: PathBuf,
    database: PathBuf,
    identity: PathBuf,
    team_id: String,
    actor: String,
    message: String,
    sender: std::sync::mpsc::Sender<Result<(), String>>,
) {
    std::thread::spawn(move || {
        let args = [
            "broadcast".to_owned(),
            "create".to_owned(),
            "--db".to_owned(),
            database.to_string_lossy().into_owned(),
            "--team".to_owned(),
            team_id,
            "--actor".to_owned(),
            actor,
            "--identity".to_owned(),
            identity.to_string_lossy().into_owned(),
            "--scope".to_owned(),
            "team".to_owned(),
            "--message".to_owned(),
            message,
        ];
        let result = Command::new(executable)
            .args(args)
            .env(crate::team_view::TEAM_DB_ENV, database)
            .output()
            .map_err(|error| error.to_string())
            .and_then(|output| {
                if output.status.success() {
                    Ok(())
                } else {
                    Err(String::from_utf8_lossy(&output.stderr).into_owned())
                }
            });
        let _ = sender.send(result);
    });
}

fn compact(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

pub(super) fn task_status(status: &crate::team::TaskStatus) -> &'static str {
    match status {
        crate::team::TaskStatus::Queued => "Queued",
        crate::team::TaskStatus::Delivered => "Delivered",
        crate::team::TaskStatus::InProgress => "In progress",
        crate::team::TaskStatus::Done => "Done",
        crate::team::TaskStatus::Cancelled => "Cancelled",
        crate::team::TaskStatus::Failed => "Failed",
        crate::team::TaskStatus::NeedsAttention => "Needs attention",
    }
}

fn put(buffer: &mut Buffer, x: u16, y: u16, width: u16, text: &str, style: Style) {
    if y < buffer.area.y || y >= buffer.area.bottom() || width == 0 {
        return;
    }
    render::put_text(
        buffer,
        x,
        y,
        width.min(buffer.area.right().saturating_sub(x)),
        text,
        style,
    );
}
