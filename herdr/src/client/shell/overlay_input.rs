use super::*;

impl ClientShellState {
    pub(super) fn dismiss_product_announcement(&mut self, outcome: &mut ClientShellInput) {
        let announcement = match self.overlay.take() {
            Some(ClientShellOverlay::ProductAnnouncement(announcement)) => announcement,
            other => {
                self.overlay = other;
                return;
            }
        };
        if self.snapshot.is_none() {
            self.overlay = Some(ClientShellOverlay::ProductAnnouncement(announcement));
            return;
        }
        self.dismissed_product_announcement =
            Some((announcement.version.clone(), announcement.id.clone()));
        self.chrome_drag = None;
        self.push_endpoint_method_with_kind(
            crate::api::schema::Method::ProductAnnouncementDismiss(
                crate::api::schema::ProductAnnouncementDismissParams {
                    version: announcement.version.clone(),
                    id: announcement.id.clone(),
                },
            ),
            PendingEndpointKind::ProductAnnouncementDismiss {
                version: announcement.version,
                id: announcement.id,
            },
            outcome,
        );
        outcome.repaint = true;
    }

    pub(super) fn scroll_product_announcement(&mut self, delta: isize) {
        let max_scroll = self.hits.product_announcement_max_scroll;
        if let Some(ClientShellOverlay::ProductAnnouncement(announcement)) = self.overlay.as_mut() {
            let current = usize::from(announcement.scroll);
            let next = if delta.is_negative() {
                current.saturating_sub(delta.unsigned_abs())
            } else {
                current.saturating_add(delta as usize)
            }
            .min(max_scroll);
            announcement.scroll = u16::try_from(next).unwrap_or(u16::MAX);
        }
    }

    pub(super) fn set_product_announcement_offset_from_bottom(
        &mut self,
        offset_from_bottom: usize,
    ) {
        let max_scroll = self.hits.product_announcement_max_scroll;
        if let Some(ClientShellOverlay::ProductAnnouncement(announcement)) = self.overlay.as_mut() {
            announcement.scroll =
                u16::try_from(max_scroll.saturating_sub(offset_from_bottom.min(max_scroll)))
                    .unwrap_or(u16::MAX);
        }
    }

    pub(super) fn open_release_notes(&mut self) {
        let Some(notes) = self
            .snapshot
            .as_deref()
            .and_then(|snapshot| snapshot.release_notes.as_ref())
        else {
            return;
        };
        self.overlay = Some(ClientShellOverlay::ReleaseNotes(release_notes_state(notes)));
        self.chrome_drag = None;
    }

    pub(super) fn dismiss_release_notes(&mut self, outcome: &mut ClientShellInput) {
        let notes = match self.overlay.take() {
            Some(ClientShellOverlay::ReleaseNotes(notes)) => notes,
            other => {
                self.overlay = other;
                return;
            }
        };
        self.chrome_drag = None;
        self.mode = if self
            .snapshot
            .as_deref()
            .and_then(|snapshot| snapshot.focused_workspace_id.as_deref())
            .is_some()
        {
            ClientShellMode::Terminal
        } else {
            ClientShellMode::Navigate
        };
        self.push_endpoint_method_with_kind(
            crate::api::schema::Method::ReleaseNotesDismiss(
                crate::api::schema::ReleaseNotesDismissParams {
                    version: notes.version.clone(),
                },
            ),
            PendingEndpointKind::ReleaseNotesDismiss,
            outcome,
        );
        outcome.repaint = true;
    }

    pub(super) fn current_release_notes_input_geometry(
        &self,
    ) -> Option<(Rect, Option<Rect>, crate::pane::ScrollMetrics)> {
        let notes = match self.overlay.as_ref()? {
            ClientShellOverlay::ReleaseNotes(notes) => notes,
            _ => return None,
        };
        let (cols, rows) = self.last_composed_size?;
        let outer = crate::ui::centered_popup_rect(
            Rect::new(0, 0, cols, rows),
            crate::ui::RELEASE_NOTES_MODAL_SIZE.0,
            crate::ui::RELEASE_NOTES_MODAL_SIZE.1,
        )?;
        let inner = Rect::new(
            outer.x.saturating_add(1),
            outer.y.saturating_add(1),
            outer.width.saturating_sub(2),
            outer.height.saturating_sub(2),
        );
        if inner.height < 8 || inner.width < 20 {
            return None;
        }
        let stack = crate::ui::modal_stack_areas(inner, 2, 1, 0, 1);
        let close = crate::ui::release_notes_close_button_rect(Rect::new(
            stack.header.x,
            stack.header.y,
            stack.header.width,
            1,
        ));
        let install_command = self
            .snapshot
            .as_deref()
            .map(|snapshot| snapshot.update_install_command.as_str())
            .unwrap_or_default();
        let metrics = crate::ui::release_notes_scroll_metrics(
            notes,
            install_command,
            stack.content,
            &self.config.palette,
        );
        let track = crate::ui::release_notes_scrollbar_rect(stack.content, metrics);
        Some((close, track, metrics))
    }

    fn current_release_notes_max_scroll(&self) -> usize {
        self.current_release_notes_input_geometry()
            .map(|(_, _, metrics)| metrics.max_offset_from_bottom)
            .unwrap_or(self.hits.release_notes_max_scroll)
    }

    pub(super) fn scroll_release_notes(&mut self, delta: isize) {
        let max_scroll = self.current_release_notes_max_scroll();
        if let Some(ClientShellOverlay::ReleaseNotes(notes)) = self.overlay.as_mut() {
            let current = usize::from(notes.scroll);
            let next = if delta.is_negative() {
                current.saturating_sub(delta.unsigned_abs())
            } else {
                current.saturating_add(delta as usize)
            }
            .min(max_scroll);
            notes.scroll = u16::try_from(next).unwrap_or(u16::MAX);
        }
    }

    pub(super) fn set_release_notes_offset_from_bottom(&mut self, offset_from_bottom: usize) {
        let max_scroll = self.current_release_notes_max_scroll();
        if let Some(ClientShellOverlay::ReleaseNotes(notes)) = self.overlay.as_mut() {
            notes.scroll =
                u16::try_from(max_scroll.saturating_sub(offset_from_bottom.min(max_scroll)))
                    .unwrap_or(u16::MAX);
        }
    }

    pub(super) fn complete_onboarding(&mut self, outcome: &mut ClientShellInput) {
        if self.snapshot.is_none() {
            return;
        }
        if let Err(error) = crate::config::update_file_at(
            &self.config.local_config_path,
            "onboarding setting",
            |content| {
                Ok(crate::config::upsert_top_level_bool(
                    content,
                    "onboarding",
                    false,
                ))
            },
        ) {
            self.set_local_config_diagnostic(Some(error));
        }
        self.config.startup_onboarding = false;
        self.open_settings_overlay();
        self.select_settings_section(ClientSettingsSection::Integrations, outcome);
    }

    pub(super) fn open_navigator_overlay(&mut self) {
        let mut navigator = ClientNavigatorOverlay {
            query: TextEditor::default(),
            search_focused: false,
            selected: None,
            scroll: 0,
            filter: None,
        };
        let rows =
            render::client_navigator_rows(&self.endpoints, &self.active_endpoint_id, &navigator);
        navigator.selected = rows
            .iter()
            .find(|row| row.current)
            .map(|row| row.target.clone());
        self.overlay = Some(ClientShellOverlay::Navigator(navigator));
    }

    pub(super) fn move_navigator_selection(&mut self, delta: isize) {
        let Some(ClientShellOverlay::Navigator(navigator)) = self.overlay.as_mut() else {
            return;
        };
        let rows =
            render::client_navigator_rows(&self.endpoints, &self.active_endpoint_id, navigator);
        if rows.is_empty() {
            navigator.selected = None;
            return;
        }
        let selected =
            super::aggregate_navigation::navigator_selected_index(&rows, navigator).unwrap_or(0);
        let next =
            (selected as isize + delta).clamp(0, rows.len().saturating_sub(1) as isize) as usize;
        navigator.selected = Some(rows[next].target.clone());
    }

    pub(super) fn scroll_navigator_to(&mut self, scroll: usize, viewport_rows: usize) {
        let Some(ClientShellOverlay::Navigator(navigator)) = self.overlay.as_mut() else {
            return;
        };
        let rows =
            render::client_navigator_rows(&self.endpoints, &self.active_endpoint_id, navigator);
        let viewport_rows = viewport_rows.max(1);
        navigator.scroll = scroll.min(rows.len().saturating_sub(viewport_rows));
        let selected =
            super::aggregate_navigation::navigator_selected_index(&rows, navigator).unwrap_or(0);
        // Keep the selection in the dragged viewport so rendering does not snap back to it.
        let selected = selected.clamp(navigator.scroll, navigator.scroll + viewport_rows - 1);
        navigator.selected = rows.get(selected).map(|row| row.target.clone());
    }

    fn move_navigator_workspace(&mut self, forward: bool) {
        let Some(ClientShellOverlay::Navigator(navigator)) = self.overlay.as_mut() else {
            return;
        };
        let rows =
            render::client_navigator_rows(&self.endpoints, &self.active_endpoint_id, navigator);
        let Some(selected) =
            super::aggregate_navigation::navigator_selected_index(&rows, navigator)
        else {
            return;
        };
        let section = rows[..=selected]
            .iter()
            .rposition(|row| !matches!(row.target, ClientNavigatorTarget::Pane { .. }))
            .unwrap_or(selected);
        let mut destinations = rows.windows(2).enumerate().filter(|(index, pair)| {
            matches!(pair[0].target, ClientNavigatorTarget::Workspace { .. })
                && matches!(pair[1].target, ClientNavigatorTarget::Pane { .. })
                && if forward {
                    *index > section
                } else {
                    *index < section
                }
        });
        let destination = if forward {
            destinations.next()
        } else {
            destinations.next_back()
        };
        if let Some((_, pair)) = destination {
            navigator.selected = Some(pair[1].target.clone());
        }
    }

    pub(super) fn accept_navigator_selection(&mut self, outcome: &mut ClientShellInput) {
        let target = self.overlay.as_ref().and_then(|overlay| match overlay {
            ClientShellOverlay::Navigator(navigator) => {
                let rows = render::client_navigator_rows(
                    &self.endpoints,
                    &self.active_endpoint_id,
                    navigator,
                );
                super::aggregate_navigation::selected_navigator_target(&rows, navigator)
            }
            _ => None,
        });
        let Some(target) = target else {
            return;
        };
        let activated = match target {
            ClientNavigatorTarget::Machine { endpoint_id } => {
                self.activate_endpoint(endpoint_id, outcome)
            }
            ClientNavigatorTarget::Workspace {
                endpoint_id,
                workspace_id,
            } => self.focus_or_activate(
                endpoint_id,
                ClientEndpointFocusTarget::Workspace(workspace_id),
                outcome,
            ),
            ClientNavigatorTarget::Pane {
                endpoint_id,
                pane_id,
            } => self.focus_or_activate(
                endpoint_id,
                ClientEndpointFocusTarget::Pane(pane_id),
                outcome,
            ),
        };
        if activated {
            self.overlay = None;
        }
        outcome.repaint = true;
    }

    pub(super) fn workspace_action_id(&self) -> Option<String> {
        self.navigate_workspace_id
            .as_ref()
            .filter(|target| {
                target.endpoint_id == self.active_endpoint_id
                    && self.navigation_target_valid(target)
            })
            .map(|target| target.workspace_id.clone())
            .or_else(|| {
                self.snapshot
                    .as_deref()
                    .and_then(|snapshot| snapshot.focused_workspace_id.clone())
            })
    }

    pub(super) fn open_new_workspace_overlay(&mut self) {
        let source_workspace_id = self.workspace_action_id();
        let cwd = self.snapshot.as_deref().and_then(|snapshot| {
            let workspace_id = source_workspace_id.as_deref()?;
            snapshot
                .workspaces
                .iter()
                .find(|workspace| workspace.workspace_id == workspace_id)
                .map(|workspace| workspace.new_workspace_cwd.clone())
        });
        let suggested_name = cwd
            .as_deref()
            .map(std::path::Path::new)
            .map(crate::workspace::derive_label_from_cwd)
            .unwrap_or_else(|| "workspace".to_owned());
        self.overlay = Some(ClientShellOverlay::Rename(ClientRenameOverlay {
            title: "new workspace",
            input: TextEditor::new(&suggested_name, true),
            target: ClientRenameTarget::NewWorkspace {
                source_workspace_id,
                cwd,
                suggested_name,
            },
        }));
    }

    pub(super) fn open_rename_workspace_overlay(&mut self) {
        let Some(snapshot) = self.snapshot.as_deref() else {
            return;
        };
        let Some(workspace_id) = self.workspace_action_id() else {
            return;
        };
        let Some(workspace) = snapshot
            .workspaces
            .iter()
            .find(|workspace| workspace.workspace_id == workspace_id)
        else {
            return;
        };
        self.overlay = Some(ClientShellOverlay::Rename(ClientRenameOverlay {
            title: "rename workspace",
            input: TextEditor::new(&workspace.label, false),
            target: ClientRenameTarget::Workspace { workspace_id },
        }));
    }

    pub(super) fn open_new_tab_overlay(&mut self) {
        let Some(snapshot) = self.snapshot.as_deref() else {
            return;
        };
        let Some(workspace_id) = snapshot.focused_workspace_id.clone() else {
            return;
        };
        let default_name = (snapshot
            .tabs
            .iter()
            .filter(|tab| tab.workspace_id == workspace_id)
            .count()
            + 1)
        .to_string();
        self.overlay = Some(ClientShellOverlay::Rename(ClientRenameOverlay {
            title: "new tab",
            input: TextEditor::new(&default_name, true),
            target: ClientRenameTarget::NewTab {
                workspace_id,
                default_name,
            },
        }));
    }

    pub(super) fn open_rename_tab_overlay(&mut self) {
        let Some(snapshot) = self.snapshot.as_deref() else {
            return;
        };
        let Some(tab_id) = snapshot.focused_tab_id.as_deref() else {
            return;
        };
        let Some(tab) = snapshot.tabs.iter().find(|tab| tab.tab_id == tab_id) else {
            return;
        };
        self.overlay = Some(ClientShellOverlay::Rename(ClientRenameOverlay {
            title: "rename tab",
            input: TextEditor::new(&tab.label, false),
            target: ClientRenameTarget::Tab {
                tab_id: tab.tab_id.clone(),
                auto_name: !tab.custom_label,
                original_name: tab.label.clone(),
            },
        }));
    }

    pub(super) fn open_rename_pane_overlay(&mut self) {
        let Some(snapshot) = self.snapshot.as_deref() else {
            return;
        };
        let Some(pane_id) = snapshot.focused_pane_id.as_deref() else {
            return;
        };
        let Some(pane) = snapshot.panes.iter().find(|pane| pane.pane_id == pane_id) else {
            return;
        };
        self.overlay = Some(ClientShellOverlay::Rename(ClientRenameOverlay {
            title: "rename pane",
            input: TextEditor::new(
                pane.label.as_deref().unwrap_or_default(),
                pane.label.is_none(),
            ),
            target: ClientRenameTarget::Pane {
                pane_id: pane.pane_id.clone(),
            },
        }));
    }

    pub(super) fn insert_overlay_text(&mut self, text: &str) -> bool {
        if self.overlay.is_none() && self.team_panel_composer_focused {
            return self.team_panel_composer.insert(text);
        }
        if self.insert_worktree_overlay_text(text) {
            return true;
        }
        match self.overlay.as_mut() {
            Some(ClientShellOverlay::Rename(rename)) => {
                rename.input.insert(text);
                true
            }
            Some(ClientShellOverlay::Help(help)) if help.search_focused => {
                if help.query.insert(text) {
                    help.scroll = 0;
                }
                true
            }
            Some(ClientShellOverlay::Navigator(navigator)) if navigator.search_focused => {
                if navigator.query.insert(text) {
                    navigator.filter = None;
                    navigator.selected = None;
                }
                true
            }
            Some(ClientShellOverlay::TeamDashboard(dashboard)) => {
                let Some(form) = dashboard.form.as_mut() else {
                    return false;
                };
                if form.member_picker_active {
                    return false;
                }
                let Some((_, editor)) = form.fields.get_mut(form.current) else {
                    return false;
                };
                editor.insert(text);
                true
            }
            _ => false,
        }
    }

    pub(super) fn route_overlay_key(
        &mut self,
        key: &crate::input::TerminalKey,
        outcome: &mut ClientShellInput,
    ) {
        use crossterm::event::KeyModifiers;

        if matches!(self.overlay, Some(ClientShellOverlay::Onboarding)) {
            if matches!(
                key.code,
                KeyCode::Enter | KeyCode::Right | KeyCode::Char('l')
            ) {
                self.complete_onboarding(outcome);
            }
            return;
        }

        if matches!(
            self.overlay,
            Some(ClientShellOverlay::ProductAnnouncement(_))
        ) {
            match key.code {
                KeyCode::Enter | KeyCode::Esc => self.dismiss_product_announcement(outcome),
                KeyCode::Up | KeyCode::Char('k') => {
                    self.scroll_product_announcement(-1);
                    outcome.repaint = true;
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.scroll_product_announcement(1);
                    outcome.repaint = true;
                }
                KeyCode::PageUp => {
                    self.scroll_product_announcement(-8);
                    outcome.repaint = true;
                }
                KeyCode::PageDown => {
                    self.scroll_product_announcement(8);
                    outcome.repaint = true;
                }
                KeyCode::Home => {
                    if let Some(ClientShellOverlay::ProductAnnouncement(announcement)) =
                        self.overlay.as_mut()
                    {
                        announcement.scroll = 0;
                    }
                    outcome.repaint = true;
                }
                KeyCode::End => {
                    if let Some(ClientShellOverlay::ProductAnnouncement(announcement)) =
                        self.overlay.as_mut()
                    {
                        announcement.scroll =
                            u16::try_from(self.hits.product_announcement_max_scroll)
                                .unwrap_or(u16::MAX);
                    }
                    outcome.repaint = true;
                }
                _ => {}
            }
            return;
        }

        if matches!(self.overlay, Some(ClientShellOverlay::ReleaseNotes(_))) {
            match key.code {
                KeyCode::Enter | KeyCode::Esc => self.dismiss_release_notes(outcome),
                KeyCode::Up | KeyCode::Char('k') => {
                    self.scroll_release_notes(-1);
                    outcome.repaint = true;
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.scroll_release_notes(1);
                    outcome.repaint = true;
                }
                KeyCode::PageUp => {
                    self.scroll_release_notes(-8);
                    outcome.repaint = true;
                }
                KeyCode::PageDown => {
                    self.scroll_release_notes(8);
                    outcome.repaint = true;
                }
                KeyCode::Home => {
                    if let Some(ClientShellOverlay::ReleaseNotes(notes)) = self.overlay.as_mut() {
                        notes.scroll = 0;
                    }
                    outcome.repaint = true;
                }
                KeyCode::End => {
                    let max_scroll = self.current_release_notes_max_scroll();
                    if let Some(ClientShellOverlay::ReleaseNotes(notes)) = self.overlay.as_mut() {
                        notes.scroll = u16::try_from(max_scroll).unwrap_or(u16::MAX);
                    }
                    outcome.repaint = true;
                }
                _ => {}
            }
            return;
        }

        if matches!(self.overlay, Some(ClientShellOverlay::GlobalMenu(_))) {
            match key.code {
                KeyCode::Esc => {
                    self.overlay = None;
                    outcome.repaint = true;
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.move_global_menu_selection(-1);
                    outcome.repaint = true;
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.move_global_menu_selection(1);
                    outcome.repaint = true;
                }
                KeyCode::Enter => {
                    let highlighted = match self.overlay.as_ref() {
                        Some(ClientShellOverlay::GlobalMenu(menu)) => menu.highlighted,
                        _ => return,
                    };
                    self.activate_global_menu_item(highlighted, outcome);
                }
                _ => {}
            }
            return;
        }

        if matches!(self.overlay, Some(ClientShellOverlay::TeamDashboard(_))) {
            if self.team_dashboard_form_key(key, outcome) {
                return;
            }
            let task_section = matches!(
                self.overlay.as_ref(),
                Some(ClientShellOverlay::TeamDashboard(dashboard))
                    if dashboard.section == ClientTeamDashboardSection::Tasks
            );
            if task_section {
                let status = match key.code {
                    KeyCode::Char('j') => {
                        self.move_team_dashboard_task(1);
                        outcome.repaint = true;
                        return;
                    }
                    KeyCode::Char('k') => {
                        self.move_team_dashboard_task(-1);
                        outcome.repaint = true;
                        return;
                    }
                    KeyCode::Char('a') => Some("delivered"),
                    KeyCode::Char('w') => Some("in_progress"),
                    KeyCode::Char('d') => Some("done"),
                    KeyCode::Char('x') => Some("cancelled"),
                    KeyCode::Char('!') => Some("needs_attention"),
                    _ => None,
                };
                if let Some(status) = status {
                    self.update_selected_team_task(status);
                    outcome.repaint = true;
                    return;
                }
            }
            match key.code {
                KeyCode::Esc => {
                    self.overlay = None;
                    outcome.repaint = true;
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.move_team_dashboard_section(-1);
                    outcome.repaint = true;
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.move_team_dashboard_section(1);
                    outcome.repaint = true;
                }
                KeyCode::Char('1') => {
                    self.select_team_dashboard_section(0);
                    outcome.repaint = true;
                }
                KeyCode::Char('2') => {
                    self.select_team_dashboard_section(1);
                    outcome.repaint = true;
                }
                KeyCode::Char('3') => {
                    self.select_team_dashboard_section(2);
                    outcome.repaint = true;
                }
                KeyCode::Char('4') => {
                    self.select_team_dashboard_section(3);
                    outcome.repaint = true;
                }
                KeyCode::Char('5') => {
                    self.select_team_dashboard_section(4);
                    outcome.repaint = true;
                }
                KeyCode::Char('p') => {
                    self.begin_team_dashboard_action(ClientTeamDashboardAction::Profile)
                }
                KeyCode::Char('c') => {
                    self.begin_team_dashboard_action(ClientTeamDashboardAction::CreateTeam)
                }
                KeyCode::Char('i') => {
                    self.begin_team_dashboard_action(ClientTeamDashboardAction::Invite)
                }
                KeyCode::Char('n') => {
                    self.begin_team_dashboard_action(ClientTeamDashboardAction::Join)
                }
                KeyCode::Char('a') => {
                    self.begin_team_dashboard_action(ClientTeamDashboardAction::Approve)
                }
                KeyCode::Char('t') => {
                    self.begin_team_dashboard_action(ClientTeamDashboardAction::Task)
                }
                KeyCode::Char('b') => {
                    self.begin_team_dashboard_action(ClientTeamDashboardAction::Broadcast)
                }
                KeyCode::Char('r') => {
                    self.begin_team_dashboard_action(ClientTeamDashboardAction::Read)
                }
                KeyCode::Char('q') => {
                    self.begin_team_dashboard_action(ClientTeamDashboardAction::Check)
                }
                KeyCode::Char('s') => self.run_team_dashboard_sync(),
                KeyCode::Char('y') => {
                    if let Some(ClientShellOverlay::TeamDashboard(dashboard)) =
                        self.overlay.as_mut()
                    {
                        if let Some(code) = dashboard.share_code.as_ref() {
                            outcome
                                .actions
                                .push(ClientShellAction::ClipboardWrite(code.as_bytes().to_vec()));
                            dashboard.status =
                                "Code copied. Share it privately with your teammate.".into();
                        } else {
                            dashboard.status =
                                "Create an invite code or join request first.".into();
                        }
                    }
                    outcome.repaint = true;
                }
                KeyCode::Char('R') => {
                    if let Some(ClientShellOverlay::TeamDashboard(dashboard)) =
                        self.overlay.as_mut()
                    {
                        dashboard.snapshot =
                            crate::team_view::load_default().map_err(|error| error.to_string());
                        dashboard.status = "Dashboard refreshed".into();
                    }
                    outcome.repaint = true;
                }
                _ => {}
            }
            return;
        }

        if self.route_settings_key(key, outcome) {
            return;
        }

        if matches!(self.overlay, Some(ClientShellOverlay::AgentTaskPicker(_))) {
            match key.code {
                KeyCode::Esc => self.overlay = None,
                KeyCode::Up | KeyCode::Char('k') => {
                    if let Some(ClientShellOverlay::AgentTaskPicker(picker)) = self.overlay.as_mut()
                    {
                        picker.highlighted = picker.highlighted.saturating_sub(1);
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if let Some(ClientShellOverlay::AgentTaskPicker(picker)) = self.overlay.as_mut()
                    {
                        picker.highlighted = picker
                            .highlighted
                            .saturating_add(1)
                            .min(picker.tasks.len().saturating_sub(1));
                    }
                }
                KeyCode::Enter => {
                    let index = match self.overlay.as_ref() {
                        Some(ClientShellOverlay::AgentTaskPicker(picker)) => picker.highlighted,
                        _ => return,
                    };
                    self.activate_agent_task_picker(index);
                }
                _ => {}
            }
            outcome.repaint = true;
            return;
        }

        if matches!(self.overlay, Some(ClientShellOverlay::ContextMenu(_))) {
            match key.code {
                KeyCode::Esc => {
                    self.overlay = None;
                    outcome.repaint = true;
                }
                KeyCode::Up => {
                    self.move_context_menu_selection(-1);
                    outcome.repaint = true;
                }
                KeyCode::Down => {
                    self.move_context_menu_selection(1);
                    outcome.repaint = true;
                }
                KeyCode::Enter => {
                    let highlighted = match self.overlay.as_ref() {
                        Some(ClientShellOverlay::ContextMenu(menu)) => menu.highlighted,
                        _ => return,
                    };
                    self.activate_context_menu_item(highlighted, outcome);
                }
                _ => {}
            }
            return;
        }

        if self.route_worktree_overlay_key(key, outcome) {
            return;
        }
        if matches!(self.overlay, Some(ClientShellOverlay::Navigator(_))) {
            let (code, modifiers) = crate::config::normalize_key_combo((key.code, key.modifiers));
            let search_focused = matches!(
                self.overlay,
                Some(ClientShellOverlay::Navigator(ClientNavigatorOverlay {
                    search_focused: true,
                    ..
                }))
            );
            if code == KeyCode::Esc {
                if search_focused {
                    if let Some(ClientShellOverlay::Navigator(navigator)) = self.overlay.as_mut() {
                        navigator.search_focused = false;
                    }
                } else {
                    self.overlay = None;
                }
                outcome.repaint = true;
                return;
            }
            if code == KeyCode::Enter {
                self.accept_navigator_selection(outcome);
                return;
            }
            if search_focused {
                if let Some(ClientShellOverlay::Navigator(navigator)) = self.overlay.as_mut() {
                    if let Some(content_changed) = navigator.query.handle_key(key) {
                        if content_changed {
                            navigator.filter = None;
                            navigator.selected = None;
                        }
                        outcome.repaint = true;
                        return;
                    }
                }
                if code == KeyCode::Up
                    || code == KeyCode::Char('p') && modifiers == KeyModifiers::CONTROL
                {
                    self.move_navigator_selection(-1);
                    outcome.repaint = true;
                    return;
                }
                if code == KeyCode::Down
                    || code == KeyCode::Char('n') && modifiers == KeyModifiers::CONTROL
                {
                    self.move_navigator_selection(1);
                    outcome.repaint = true;
                    return;
                }
                return;
            }
            if matches!(code, KeyCode::Left | KeyCode::Right) && modifiers.is_empty() {
                self.move_navigator_workspace(code == KeyCode::Right);
                outcome.repaint = true;
                return;
            }
            if code == KeyCode::Backspace && modifiers.is_empty() {
                if let Some(ClientShellOverlay::Navigator(navigator)) = self.overlay.as_mut() {
                    if navigator.filter.take().is_some() {
                        navigator.selected = None;
                    }
                }
                outcome.repaint = true;
                return;
            }
            if code == KeyCode::Home && modifiers.is_empty() {
                if let Some(ClientShellOverlay::Navigator(navigator)) = self.overlay.as_mut() {
                    navigator.selected = None;
                    navigator.scroll = 0;
                }
                outcome.repaint = true;
                return;
            }
            if matches!(code, KeyCode::End | KeyCode::Char('G')) && modifiers.is_empty() {
                let last = self.overlay.as_ref().and_then(|overlay| match overlay {
                    ClientShellOverlay::Navigator(navigator) => render::client_navigator_rows(
                        &self.endpoints,
                        &self.active_endpoint_id,
                        navigator,
                    )
                    .last()
                    .map(|row| row.target.clone()),
                    _ => None,
                });
                if let Some(ClientShellOverlay::Navigator(navigator)) = self.overlay.as_mut() {
                    navigator.selected = last;
                }
                outcome.repaint = true;
                return;
            }
            // Match the typed character so layouts where `/` needs Shift work
            // when the host reports it as a shifted key (report-all panes).
            // A shifted key needs the host's text or shifted alternate: bare
            // `CSI 47;2u` is Shift+/, which is not `/` on most layouts.
            let shifted_without_evidence = key.modifiers.contains(KeyModifiers::SHIFT)
                && key.generated_text.is_none()
                && key.shifted_codepoint.is_none();
            if !shifted_without_evidence && crate::input::keybind_help_text_char(key) == Some('/') {
                if let Some(ClientShellOverlay::Navigator(navigator)) = self.overlay.as_mut() {
                    navigator.search_focused = true;
                    navigator.filter = None;
                }
                outcome.repaint = true;
                return;
            }
            if matches!(code, KeyCode::Down | KeyCode::Char('j')) && modifiers.is_empty() {
                self.move_navigator_selection(1);
                outcome.repaint = true;
                return;
            }
            if matches!(code, KeyCode::Up | KeyCode::Char('k')) && modifiers.is_empty() {
                self.move_navigator_selection(-1);
                outcome.repaint = true;
                return;
            }
            if code == KeyCode::Char('d') && modifiers.contains(KeyModifiers::CONTROL) {
                self.move_navigator_selection(8);
                outcome.repaint = true;
                return;
            }
            if code == KeyCode::Char('u') && modifiers.contains(KeyModifiers::CONTROL) {
                self.move_navigator_selection(-8);
                outcome.repaint = true;
                return;
            }
            if let Some(filter) = match code {
                KeyCode::Char('b') if modifiers.is_empty() => Some(ClientNavigatorFilter::Blocked),
                KeyCode::Char('w') if modifiers.is_empty() => Some(ClientNavigatorFilter::Working),
                KeyCode::Char('i') if modifiers.is_empty() => Some(ClientNavigatorFilter::Idle),
                KeyCode::Char('d') if modifiers.is_empty() => Some(ClientNavigatorFilter::Done),
                _ => None,
            } {
                if let Some(ClientShellOverlay::Navigator(navigator)) = self.overlay.as_mut() {
                    navigator.query.clear();
                    navigator.filter = Some(filter);
                    navigator.selected = None;
                }
                outcome.repaint = true;
                return;
            }
            if code == KeyCode::Char('a') && modifiers.is_empty() {
                if let Some(ClientShellOverlay::Navigator(navigator)) = self.overlay.as_mut() {
                    navigator.query.clear();
                    navigator.filter = None;
                    navigator.selected = None;
                }
                outcome.repaint = true;
                return;
            }
            return;
        }

        if matches!(self.overlay, Some(ClientShellOverlay::Help(_))) {
            let text_character = crate::input::keybind_help_text_char(key);
            let (code, modifiers) = crate::config::normalize_key_combo((key.code, key.modifiers));
            let search_focused = matches!(
                self.overlay,
                Some(ClientShellOverlay::Help(ClientHelpOverlay {
                    search_focused: true,
                    ..
                }))
            );
            if search_focused {
                if let Some(ClientShellOverlay::Help(help)) = self.overlay.as_mut() {
                    if let Some(content_changed) = help.query.handle_key(key) {
                        if content_changed {
                            help.scroll = 0;
                        }
                        outcome.repaint = true;
                        return;
                    }
                }
                match code {
                    KeyCode::Esc => {
                        if let Some(ClientShellOverlay::Help(help)) = self.overlay.as_mut() {
                            help.search_focused = false;
                            help.query.clear();
                            help.scroll = 0;
                        }
                    }
                    KeyCode::Enter => self.overlay = None,
                    KeyCode::Up
                    | KeyCode::Down
                    | KeyCode::PageUp
                    | KeyCode::PageDown
                    | KeyCode::Char('n' | 'p')
                        if !matches!(code, KeyCode::Char(_))
                            || modifiers == KeyModifiers::CONTROL =>
                    {
                        let delta = match code {
                            KeyCode::Up | KeyCode::Char('p') => -1,
                            KeyCode::Down | KeyCode::Char('n') => 1,
                            KeyCode::PageUp => -8,
                            KeyCode::PageDown => 8,
                            _ => unreachable!(),
                        };
                        if let Some(ClientShellOverlay::Help(help)) = self.overlay.as_mut() {
                            help.scroll = help
                                .scroll
                                .saturating_add_signed(delta)
                                .min(self.hits.help_max_scroll);
                        }
                    }
                    _ => {}
                }
                outcome.repaint = true;
                return;
            }

            match code {
                KeyCode::Esc | KeyCode::Enter => self.overlay = None,
                KeyCode::Home => {
                    if let Some(ClientShellOverlay::Help(help)) = self.overlay.as_mut() {
                        help.scroll = 0;
                    }
                }
                KeyCode::End => {
                    if let Some(ClientShellOverlay::Help(help)) = self.overlay.as_mut() {
                        help.scroll = self.hits.help_max_scroll;
                    }
                }
                KeyCode::Up
                | KeyCode::Char('k')
                | KeyCode::Down
                | KeyCode::Char('j')
                | KeyCode::PageUp
                | KeyCode::PageDown => {
                    let delta = match code {
                        KeyCode::Up | KeyCode::Char('k') => -1,
                        KeyCode::Down | KeyCode::Char('j') => 1,
                        KeyCode::PageUp => -8,
                        KeyCode::PageDown => 8,
                        _ => unreachable!(),
                    };
                    if let Some(ClientShellOverlay::Help(help)) = self.overlay.as_mut() {
                        help.scroll = help
                            .scroll
                            .saturating_add_signed(delta)
                            .min(self.hits.help_max_scroll);
                    }
                }
                _ if text_character == Some('/') => {
                    if let Some(ClientShellOverlay::Help(help)) = self.overlay.as_mut() {
                        help.search_focused = true;
                        help.scroll = 0;
                    }
                }
                _ if text_character == Some('?') => self.overlay = None,
                _ => {}
            }
            outcome.repaint = true;
            return;
        }

        if matches!(self.overlay, Some(ClientShellOverlay::ConfirmClose(_))) {
            if key.code == KeyCode::Enter {
                self.accept_close_confirmation(outcome);
            } else if key.code == KeyCode::Esc {
                self.overlay = None;
                self.mode = ClientShellMode::Navigate;
                self.navigate_workspace_id = self.focused_navigation_target();
                self.reveal_navigation_workspace = true;
                outcome.repaint = true;
            }
            return;
        }

        let Some(ClientShellOverlay::Rename(rename)) = self.overlay.as_mut() else {
            return;
        };
        if key.code == KeyCode::Enter {
            self.save_rename_overlay(outcome);
            return;
        }
        if key.code == KeyCode::Esc {
            self.overlay = None;
            outcome.repaint = true;
            return;
        }
        if key
            .generated_text
            .as_deref()
            .is_some_and(|text| !text.is_empty())
        {
            outcome.repaint |= rename.input.handle_key(key).is_some();
            return;
        }
        if key.code == KeyCode::Char('c') && key.modifiers == KeyModifiers::CONTROL {
            rename.input.clear();
            outcome.repaint = true;
            return;
        }
        if key.code == KeyCode::Backspace && key.modifiers.contains(KeyModifiers::SUPER) {
            rename.input.clear();
            outcome.repaint = true;
            return;
        }
        if rename.input.handle_key(key).is_some() {
            outcome.repaint = true;
        }
    }

    pub(super) fn open_team_dashboard(&mut self) {
        self.overlay = Some(ClientShellOverlay::TeamDashboard(
            ClientTeamDashboardOverlay {
                section: ClientTeamDashboardSection::Teammates,
                task_cursor: 0,
                snapshot: crate::team_view::load_default().map_err(|error| error.to_string()),
                form: None,
                status: String::new(),
                pending: None,
                relay_url: std::env::var("HERDR_RELAY_URL").ok(),
                relay_token: std::env::var("HERDR_RELAY_TOKEN").ok(),
                share_code: None,
            },
        ));
    }

    fn begin_team_dashboard_action(&mut self, action: ClientTeamDashboardAction) {
        let repo_path = std::env::current_dir().unwrap_or_default();
        let repo_name = repo_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("repository");
        let fields = match action {
            ClientTeamDashboardAction::Profile => vec![
                ("Member ID".into(), TextEditor::new("member-1", false)),
                ("Your name".into(), TextEditor::default()),
                (
                    "Role (manager/group_head/part_head/implementer)".into(),
                    TextEditor::new("implementer", false),
                ),
                ("Repository ID".into(), TextEditor::new(repo_name, false)),
                (
                    "Local repository folder".into(),
                    TextEditor::new(&repo_path.to_string_lossy(), false),
                ),
            ],
            ClientTeamDashboardAction::CreateTeam => {
                vec![("Team name".into(), TextEditor::default())]
            }
            ClientTeamDashboardAction::Invite => {
                if let Err(error) = self.start_team_dashboard_action(action, Vec::new()) {
                    if let Some(ClientShellOverlay::TeamDashboard(dashboard)) =
                        self.overlay.as_mut()
                    {
                        dashboard.status = error;
                    }
                }
                return;
            }
            ClientTeamDashboardAction::Join => {
                vec![("Invite code (6 digits or paste)".into(), TextEditor::default())]
            }
            ClientTeamDashboardAction::Approve => {
                vec![("Join request code (paste)".into(), TextEditor::default())]
            }
            ClientTeamDashboardAction::Task => {
                vec![("Task description".into(), TextEditor::default())]
            }
            ClientTeamDashboardAction::TaskStatus => {
                unreachable!("status actions do not use a form")
            }
            ClientTeamDashboardAction::Broadcast => {
                vec![("Message to the team".into(), TextEditor::default())]
            }
            ClientTeamDashboardAction::Read => {
                let repository = match self.overlay.as_ref() {
                    Some(ClientShellOverlay::TeamDashboard(dashboard)) => dashboard
                        .snapshot
                        .as_ref()
                        .ok()
                        .and_then(|s| s.repositories.first())
                        .cloned()
                        .unwrap_or_default(),
                    _ => String::new(),
                };
                vec![
                    ("Question for local Codex".into(), TextEditor::default()),
                    (
                        "Approved repository ID".into(),
                        TextEditor::new(&repository, false),
                    ),
                ]
            }
            ClientTeamDashboardAction::Check => {
                let repository = match self.overlay.as_ref() {
                    Some(ClientShellOverlay::TeamDashboard(dashboard)) => dashboard
                        .snapshot
                        .as_ref()
                        .ok()
                        .and_then(|snapshot| snapshot.repositories.first())
                        .cloned()
                        .unwrap_or_default(),
                    _ => String::new(),
                };
                vec![
                    (
                        "Repository ID approved for this request".into(),
                        TextEditor::new(&repository, false),
                    ),
                    (
                        "Question for teammate's Codex".into(),
                        TextEditor::default(),
                    ),
                ]
            }
        };
        if let Some(ClientShellOverlay::TeamDashboard(dashboard)) = self.overlay.as_mut() {
            dashboard.form = Some(ClientTeamDashboardForm {
                action,
                fields,
                current: 0,
                member_picker_active: false,
                member_cursor: 0,
                selected_member_ids: Vec::new(),
            });
            dashboard.status.clear();
        }
    }

    fn team_dashboard_form_key(
        &mut self,
        key: &crate::input::TerminalKey,
        outcome: &mut ClientShellInput,
    ) -> bool {
        let picking_task_members = matches!(
            self.overlay.as_ref(),
            Some(ClientShellOverlay::TeamDashboard(dashboard))
                if dashboard.form.as_ref().is_some_and(|form|
                    matches!(form.action, ClientTeamDashboardAction::Task | ClientTeamDashboardAction::Check)
                        && form.member_picker_active)
        );
        if picking_task_members {
            return self.team_dashboard_member_picker_key(key, outcome);
        }
        let Some(ClientShellOverlay::TeamDashboard(dashboard)) = self.overlay.as_mut() else {
            return false;
        };
        let Some(form) = dashboard.form.as_mut() else {
            return false;
        };
        match key.code {
            KeyCode::Esc => {
                dashboard.form = None;
                outcome.repaint = true;
            }
            KeyCode::Enter => {
                if form.action == ClientTeamDashboardAction::Task {
                    let description = form
                        .fields
                        .first()
                        .map(|(_, editor)| editor.as_str().trim())
                        .unwrap_or_default();
                    if description.is_empty() {
                        dashboard.status = "Enter a task description first".into();
                    } else {
                        form.member_picker_active = true;
                        form.member_cursor = 0;
                        dashboard.status =
                            "Select teammates with Space, then press Enter to assign".into();
                    }
                    outcome.repaint = true;
                    return true;
                }
                if form.action == ClientTeamDashboardAction::Check {
                    if form.current + 1 < form.fields.len() {
                        form.current += 1;
                    } else if form
                        .fields
                        .iter()
                        .any(|(_, editor)| editor.as_str().trim().is_empty())
                    {
                        dashboard.status = "Enter a repository ID and question first".into();
                    } else {
                        form.member_picker_active = true;
                        form.member_cursor = 0;
                        dashboard.status =
                            "Select one teammate, then press Enter to send the request".into();
                    }
                    outcome.repaint = true;
                    return true;
                }
                if form.current + 1 < form.fields.len() {
                    form.current += 1;
                } else {
                    let action = form.action;
                    let values = form
                        .fields
                        .iter()
                        .map(|(_, editor)| editor.as_str().trim().to_owned())
                        .collect::<Vec<_>>();
                    dashboard.form = None;
                    if values.iter().any(String::is_empty) {
                        dashboard.status = "Please fill in every field".into();
                    } else if let Err(error) = self.start_team_dashboard_action(action, values) {
                        if let Some(ClientShellOverlay::TeamDashboard(dashboard)) =
                            self.overlay.as_mut()
                        {
                            dashboard.status = error;
                        }
                    }
                }
                outcome.repaint = true;
            }
            _ => {
                if let Some((_, editor)) = form.fields.get_mut(form.current) {
                    outcome.repaint |= editor.handle_key(key).unwrap_or(false);
                }
            }
        }
        true
    }

    fn team_dashboard_member_picker_key(
        &mut self,
        key: &crate::input::TerminalKey,
        outcome: &mut ClientShellInput,
    ) -> bool {
        let mut assignment = None;
        {
            let Some(ClientShellOverlay::TeamDashboard(dashboard)) = self.overlay.as_mut() else {
                return false;
            };
            let members = dashboard
                .snapshot
                .as_ref()
                .map(|snapshot| {
                    snapshot
                        .teammates
                        .iter()
                        .filter(|member| member.active)
                        .map(|member| member.member_id.clone())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let Some(form) = dashboard.form.as_mut() else {
                return false;
            };
            if !matches!(
                form.action,
                ClientTeamDashboardAction::Task | ClientTeamDashboardAction::Check
            ) || !form.member_picker_active
            {
                return false;
            }
            let is_check = form.action == ClientTeamDashboardAction::Check;
            match key.code {
                KeyCode::Esc => {
                    dashboard.form = None;
                    dashboard.status = if is_check {
                        "Repository question cancelled".into()
                    } else {
                        "Task assignment cancelled".into()
                    };
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    if !members.is_empty() {
                        form.member_cursor = form.member_cursor.saturating_sub(1);
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if !members.is_empty() {
                        form.member_cursor = (form.member_cursor + 1).min(members.len() - 1);
                    }
                }
                KeyCode::Char(' ') => {
                    if let Some(member_id) = members.get(form.member_cursor) {
                        if is_check {
                            form.selected_member_ids = vec![member_id.clone()];
                        } else if form.selected_member_ids.iter().any(|id| id == member_id) {
                            form.selected_member_ids.retain(|id| id != member_id);
                        } else {
                            form.selected_member_ids.push(member_id.clone());
                        }
                    }
                }
                KeyCode::Enter => {
                    if form.selected_member_ids.is_empty() {
                        dashboard.status = if is_check {
                            "Select one active teammate".into()
                        } else {
                            "Select at least one active teammate".into()
                        };
                    } else {
                        if is_check {
                            assignment = Some((
                                ClientTeamDashboardAction::Check,
                                vec![
                                    form.selected_member_ids[0].clone(),
                                    form.fields[0].1.as_str().trim().to_owned(),
                                    form.fields[1].1.as_str().trim().to_owned(),
                                ],
                            ));
                        } else {
                            let description = form
                                .fields
                                .first()
                                .map(|(_, editor)| editor.as_str().trim().to_owned())
                                .unwrap_or_default();
                            assignment = Some((
                                ClientTeamDashboardAction::Task,
                                vec![form.selected_member_ids.join("\n"), description],
                            ));
                        }
                        dashboard.form = None;
                    }
                }
                _ => {}
            }
        }
        if let Some((action, values)) = assignment {
            if let Err(error) = self.start_team_dashboard_action(action, values) {
                if let Some(ClientShellOverlay::TeamDashboard(dashboard)) = self.overlay.as_mut() {
                    dashboard.status = error;
                }
            }
        }
        outcome.repaint = true;
        true
    }

    fn move_team_dashboard_task(&mut self, delta: isize) {
        let Some(ClientShellOverlay::TeamDashboard(dashboard)) = self.overlay.as_mut() else {
            return;
        };
        let count = dashboard
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.tasks.len())
            .unwrap_or_default();
        if count > 0 {
            dashboard.task_cursor =
                (dashboard.task_cursor as isize + delta).rem_euclid(count as isize) as usize;
        }
    }

    fn update_selected_team_task(&mut self, status: &str) {
        let selected = match self.overlay.as_ref() {
            Some(ClientShellOverlay::TeamDashboard(dashboard)) => dashboard
                .snapshot
                .as_ref()
                .ok()
                .and_then(|snapshot| snapshot.tasks.get(dashboard.task_cursor))
                .map(|task| task.task_id.clone()),
            _ => None,
        };
        let Some(task_id) = selected else {
            if let Some(ClientShellOverlay::TeamDashboard(dashboard)) = self.overlay.as_mut() {
                dashboard.status = "Select a task first".into();
            }
            return;
        };
        if let Err(error) = self.start_team_dashboard_action(
            ClientTeamDashboardAction::TaskStatus,
            vec![task_id, status.to_owned()],
        ) {
            if let Some(ClientShellOverlay::TeamDashboard(dashboard)) = self.overlay.as_mut() {
                dashboard.status = error;
            }
        }
    }

    fn start_team_dashboard_action(
        &mut self,
        action: ClientTeamDashboardAction,
        values: Vec<String>,
    ) -> Result<(), String> {
        use ClientTeamDashboardAction as Action;
        let database = crate::team_view::default_database_path();
        let identity = database.with_file_name(format!(
            "{}.identity.json",
            database
                .file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or("team")
        ));
        let mut team = None;
        let mut actor = None;
        let mut authority = None;
        if let Some(ClientShellOverlay::TeamDashboard(dashboard)) = self.overlay.as_ref() {
            if let Ok(snapshot) = &dashboard.snapshot {
                team = snapshot.teams.first().map(|t| t.team_id.clone());
                authority = snapshot
                    .teams
                    .first()
                    .map(|t| t.authority_member_id.clone());
                actor = snapshot.profile.as_ref().map(|m| m.member_id.clone());
            }
        }
        if action == Action::Join && !crate::cli::relay::is_short_invite_code(values[0].trim()) {
            if let (Some(team_id), Some(authority_id)) = (team.as_ref(), authority.as_ref()) {
                let bundle = crate::cli::team::decode_invite_code(&values[0])
                    .map_err(|error| error.to_string())?;
                crate::cli::team::validate_invite_for_existing_team(&bundle, team_id, authority_id)
                    .map_err(|error| error.to_string())?;
                if let Some(credential_sink) = self.relay_credential_sink.as_ref() {
                    let _ =
                        credential_sink.set(bundle.relay_url.clone(), bundle.relay_token.clone());
                }
                if let Some(ClientShellOverlay::TeamDashboard(dashboard)) = self.overlay.as_mut() {
                    dashboard.relay_url = Some(bundle.relay_url);
                    dashboard.relay_token = Some(bundle.relay_token);
                    dashboard.status =
                        "Invite verified. Background relay sync is ready; press s to sync now."
                            .into();
                }
                return Ok(());
            }
        }
        if matches!(action, Action::Invite | Action::Join | Action::Approve) {
            let team_id = team;
            let (relay_url, relay_token) = match self.overlay.as_ref() {
                Some(ClientShellOverlay::TeamDashboard(dashboard)) => {
                    (dashboard.relay_url.clone(), dashboard.relay_token.clone())
                }
                _ => (None, None),
            };
            let database = database.clone();
            let identity = identity.clone();
            let credential_sink = self.relay_credential_sink.clone();
            let input_code = values.first().cloned().unwrap_or_default();
            let (sender, receiver) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let mut result = ClientTeamDashboardResult {
                    status: String::new(),
                    relay_credentials: None,
                    share_code: None,
                };
                let operation = (|| -> Result<(), String> {
                    match action {
                        Action::Invite => {
                            let team_id =
                                team_id.as_deref().ok_or("Create or join a team first")?;
                            let bundle = crate::cli::team::create_invite_bundle(
                                &database, team_id, &identity,
                            )
                            .map_err(|error| error.to_string())?;
                            let signed_code = crate::cli::team::encode_invite_code(&bundle)
                                .map_err(|error| error.to_string())?;
                            result.share_code = Some(crate::cli::relay::publish_invite_code(
                                &bundle.relay_url,
                                team_id,
                                &bundle.relay_token,
                                &signed_code,
                            )?);
                            result.relay_credentials =
                                Some((bundle.relay_url.clone(), bundle.relay_token.clone()));
                            result.status = "Six-digit invite ready (one use, expires in 10 minutes). Press y to copy and share privately.".into();
                        }
                        Action::Join => {
                            let input_code = input_code.trim();
                            let signed_code = if crate::cli::relay::is_short_invite_code(input_code)
                            {
                                let endpoint =
                                    std::env::var("HERDR_RELAY_URL").unwrap_or_else(|_| {
                                        crate::cli::relay::DEFAULT_RELAY_URL.into()
                                    });
                                crate::cli::relay::claim_invite_code(&endpoint, input_code)?
                            } else {
                                input_code.to_owned()
                            };
                            let bundle = crate::cli::team::decode_invite_code(&signed_code)
                                .map_err(|error| error.to_string())?;
                            if let (Some(team_id), Some(authority_id)) =
                                (team_id.as_ref(), authority.as_ref())
                            {
                                crate::cli::team::validate_invite_for_existing_team(
                                    &bundle,
                                    team_id,
                                    authority_id,
                                )
                                .map_err(|error| error.to_string())?;
                                result.relay_credentials =
                                    Some((bundle.relay_url, bundle.relay_token));
                                result.status =
                                    "Invite verified. Background relay sync is ready.".into();
                                return Ok(());
                            }
                            let request = crate::cli::team::accept_invite_bundle(
                                &database, &identity, &bundle,
                            )
                            .map_err(|error| error.to_string())?;
                            result.share_code = Some(
                                crate::cli::team::encode_join_code(&request)
                                    .map_err(|error| error.to_string())?,
                            );
                            result.relay_credentials = Some((bundle.relay_url, bundle.relay_token));
                            result.status = "Joined locally. Press y to copy your signed request code and send it privately to the manager.".into();
                        }
                        Action::Approve => {
                            let team_id =
                                team_id.as_deref().ok_or("Create or join a team first")?;
                            let request = crate::cli::team::decode_join_code(&input_code)
                                .map_err(|error| error.to_string())?;
                            let credentials = relay_url.as_deref().zip(relay_token.as_deref());
                            crate::cli::team::approve_join_request(
                                &database,
                                team_id,
                                &identity,
                                &request,
                                credentials,
                            )
                            .map_err(|error| error.to_string())?;
                            result.relay_credentials = relay_url
                                .as_ref()
                                .zip(relay_token.as_ref())
                                .map(|(url, token)| (url.clone(), token.clone()));
                            result.status = format!(
                                "Approved {}. Team sync completed.",
                                request.member.display_name
                            );
                        }
                        _ => unreachable!(),
                    }
                    Ok(())
                })();
                if let Err(error) = operation {
                    result.status = format!("Could not complete: {error}");
                }
                if let (Some(credential_sink), Some((url, token))) =
                    (credential_sink, result.relay_credentials.as_ref())
                {
                    let _ = credential_sink.set(url.clone(), token.clone());
                }
                let _ = sender.send(result);
            });
            if let Some(ClientShellOverlay::TeamDashboard(dashboard)) = self.overlay.as_mut() {
                dashboard.status = "Working...".into();
                dashboard.pending = Some(receiver);
            }
            return Ok(());
        }
        let db = database.to_string_lossy().into_owned();
        let id = identity.to_string_lossy().into_owned();
        let task_assignees = if action == Action::Task {
            values
                .first()
                .map(|value| {
                    value
                        .lines()
                        .map(str::trim)
                        .filter(|id| !id.is_empty())
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let args = match action {
            Action::Profile => vec![
                "onboard".into(),
                "setup".into(),
                "--db".into(),
                db.clone(),
                "--identity".into(),
                id.clone(),
                "--member".into(),
                values[0].clone(),
                "--display-name".into(),
                values[1].clone(),
                "--role".into(),
                values[2].clone(),
                "--repository-id".into(),
                values[3].clone(),
                "--repository-path".into(),
                values[4].clone(),
            ],
            Action::CreateTeam => vec![
                "team".into(),
                "create".into(),
                "--db".into(),
                db.clone(),
                "--identity".into(),
                id.clone(),
                "--name".into(),
                values[0].clone(),
            ],
            Action::Invite | Action::Join | Action::Approve => {
                unreachable!("team code actions complete through the in-memory path above")
            }
            Action::Task => vec![
                "task".into(),
                "create".into(),
                "--db".into(),
                db.clone(),
                "--team".into(),
                team.ok_or("Create or join a team first")?,
                "--actor".into(),
                actor.ok_or("Set up your local profile first with p")?,
                "--identity".into(),
                id.clone(),
                "--assignee".into(),
                task_assignees
                    .first()
                    .cloned()
                    .ok_or("Select at least one active teammate")?,
                "--description".into(),
                values[1].clone(),
            ],
            Action::TaskStatus => vec![
                "task".into(),
                "status".into(),
                "--db".into(),
                db.clone(),
                "--team".into(),
                team.ok_or("Create or join a team first")?,
                "--actor".into(),
                actor.ok_or("Set up your local profile first with p")?,
                "--identity".into(),
                id.clone(),
                "--task".into(),
                values[0].clone(),
                "--status".into(),
                values[1].clone(),
            ],
            Action::Broadcast => vec![
                "broadcast".into(),
                "create".into(),
                "--db".into(),
                db.clone(),
                "--team".into(),
                team.ok_or("Create or join a team first")?,
                "--actor".into(),
                actor.ok_or("Set up your local profile first with p")?,
                "--identity".into(),
                id.clone(),
                "--scope".into(),
                "team".into(),
                "--message".into(),
                values[0].clone(),
            ],
            Action::Read => vec![
                "read".into(),
                "--db".into(),
                db.clone(),
                "--repository-id".into(),
                values[1].clone(),
                "--question".into(),
                values[0].clone(),
            ],
            Action::Check => vec![
                "check".into(),
                "create".into(),
                "--db".into(),
                db.clone(),
                "--team".into(),
                team.ok_or("Create or join a team first")?,
                "--actor".into(),
                actor.ok_or("Set up your local profile first with p")?,
                "--identity".into(),
                id.clone(),
                "--target".into(),
                values[0].clone(),
                "--repository".into(),
                values[1].clone(),
                "--question".into(),
                values[2].clone(),
            ],
        };
        let command_batches = if action == Action::Task {
            task_assignees
                .iter()
                .map(|assignee| {
                    let mut one = args.clone();
                    if let Some(index) = one.iter().position(|value| value == "--assignee") {
                        one[index + 1] = assignee.clone();
                    }
                    one
                })
                .collect::<Vec<_>>()
        } else {
            vec![args]
        };
        let command_count = command_batches.len();
        let executable = std::env::current_exe().map_err(|e| e.to_string())?;
        let (relay_url, relay_token) = match self.overlay.as_ref() {
            Some(ClientShellOverlay::TeamDashboard(dashboard)) => {
                (dashboard.relay_url.clone(), dashboard.relay_token.clone())
            }
            _ => (None, None),
        };
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            if let Some(parent) = database.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let mut outputs = Vec::new();
            let mut succeeded = 0usize;
            for args in command_batches {
                let mut command = std::process::Command::new(&executable);
                command
                    .args(&args)
                    .env(crate::team_view::TEAM_DB_ENV, &database);
                if let Some(url) = relay_url.as_ref() {
                    command.env("HERDR_RELAY_URL", url);
                }
                if let Some(token) = relay_token.as_ref() {
                    command.env("HERDR_RELAY_TOKEN", token);
                }
                match command.output() {
                    Ok(output) => {
                        let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
                        if !output.status.success() {
                            text.push_str(&String::from_utf8_lossy(&output.stderr));
                        } else {
                            succeeded += 1;
                        }
                        if text.is_empty() {
                            text = format!("Command exited with {}", output.status);
                        }
                        outputs.push(text);
                    }
                    Err(error) => outputs.push(format!("Could not start command: {error}")),
                }
            }
            let details = outputs.join("\n").chars().take(2000).collect::<String>();
            let status = if succeeded == command_count {
                if action == Action::Task && command_count > 1 {
                    format!("Assigned to {succeeded} teammate(s).\n{details}")
                } else {
                    format!("Completed:\n{details}")
                }
            } else if action == Action::Task && succeeded > 0 {
                format!("Assigned to {succeeded} of {command_count} teammate(s).\n{details}")
            } else {
                format!("Could not complete:\n{details}")
            };
            let completion = ClientTeamDashboardResult {
                status,
                relay_credentials: None,
                share_code: None,
            };
            let _ = sender.send(completion);
        });
        if let Some(ClientShellOverlay::TeamDashboard(dashboard)) = self.overlay.as_mut() {
            dashboard.status = "Workingâ€¦".into();
            dashboard.pending = Some(receiver);
        }
        Ok(())
    }

    fn run_team_dashboard_sync(&mut self) {
        let database = crate::team_view::default_database_path();
        let team = match self.overlay.as_ref() {
            Some(ClientShellOverlay::TeamDashboard(dashboard)) => dashboard
                .snapshot
                .as_ref()
                .ok()
                .and_then(|s| s.teams.first())
                .map(|t| t.team_id.clone()),
            _ => None,
        };
        let Some(team) = team else {
            if let Some(ClientShellOverlay::TeamDashboard(d)) = self.overlay.as_mut() {
                d.status = "Create or join a team first".into();
            }
            return;
        };
        let (url, token) = match self.overlay.as_ref() {
            Some(ClientShellOverlay::TeamDashboard(d)) => (
                d.relay_url
                    .clone()
                    .or_else(|| std::env::var("HERDR_RELAY_URL").ok()),
                d.relay_token
                    .clone()
                    .or_else(|| std::env::var("HERDR_RELAY_TOKEN").ok()),
            ),
            _ => (None, None),
        };
        let Some(url) = url else {
            if let Some(ClientShellOverlay::TeamDashboard(d)) = self.overlay.as_mut() {
                d.status = "Connect using an invite code (n) before syncing".into();
            }
            return;
        };
        let Ok(executable) = std::env::current_exe() else {
            return;
        };
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let db = database.to_string_lossy().into_owned();
            let url = url;
            let mut command = std::process::Command::new(executable);
            command
                .args(["relay", "sync", "--db", &db, "--team", &team, "--url", &url])
                .env(crate::team_view::TEAM_DB_ENV, &database);
            if let Some(token) = token {
                command.env("HERDR_RELAY_TOKEN", token);
            }
            let result = command.output();
            let status = match result {
                Ok(output) if output.status.success() => format!(
                    "Sync complete:\n{}",
                    String::from_utf8_lossy(&output.stdout)
                ),
                Ok(output) => format!("Sync failed:\n{}", String::from_utf8_lossy(&output.stderr)),
                Err(e) => format!("Sync failed: {e}"),
            };
            let _ = sender.send(ClientTeamDashboardResult {
                status,
                relay_credentials: None,
                share_code: None,
            });
        });
        if let Some(ClientShellOverlay::TeamDashboard(d)) = self.overlay.as_mut() {
            d.status = "Syncingâ€¦".into();
            d.pending = Some(receiver);
        }
    }

    pub(crate) fn tick_team_dashboard(&mut self) -> bool {
        let credential_sink = self.relay_credential_sink.clone();
        let Some(ClientShellOverlay::TeamDashboard(dashboard)) = self.overlay.as_mut() else {
            return false;
        };
        let Some(receiver) = dashboard.pending.as_ref() else {
            return false;
        };
        match receiver.try_recv() {
            Ok(result) => {
                dashboard.pending = None;
                dashboard.status = result.status;
                if let Some((url, token)) = result.relay_credentials {
                    if let Some(credential_sink) = credential_sink {
                        let _ = credential_sink.set(url.clone(), token.clone());
                    }
                    dashboard.relay_url = Some(url);
                    dashboard.relay_token = Some(token);
                }
                if result.share_code.is_some() {
                    dashboard.share_code = result.share_code;
                }
                dashboard.snapshot = crate::team_view::load_default().map_err(|e| e.to_string());
                true
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                dashboard.pending = None;
                dashboard.status = "Action stopped unexpectedly".into();
                true
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => false,
        }
    }

    pub(super) fn move_team_dashboard_section(&mut self, delta: isize) {
        let Some(ClientShellOverlay::TeamDashboard(dashboard)) = self.overlay.as_mut() else {
            return;
        };
        let count = ClientTeamDashboardSection::ALL.len();
        dashboard.section = ClientTeamDashboardSection::ALL
            [(dashboard.section.index() as isize + delta).rem_euclid(count as isize) as usize];
    }

    pub(super) fn select_team_dashboard_section(&mut self, index: usize) {
        let Some(section) = ClientTeamDashboardSection::ALL.get(index).copied() else {
            return;
        };
        if let Some(ClientShellOverlay::TeamDashboard(dashboard)) = self.overlay.as_mut() {
            dashboard.section = section;
        }
    }

    pub(super) fn save_rename_overlay(&mut self, outcome: &mut ClientShellInput) {
        let Some(ClientShellOverlay::Rename(rename)) = self.overlay.take() else {
            return;
        };
        let trimmed = rename.input.trim();
        let method = match rename.target {
            ClientRenameTarget::NewWorkspace {
                source_workspace_id,
                cwd,
                suggested_name,
            } => Some(crate::api::schema::Method::WorkspaceCreate(
                crate::api::schema::WorkspaceCreateParams {
                    source_workspace_id,
                    cwd,
                    focus: true,
                    label: (!trimmed.is_empty() && trimmed != suggested_name)
                        .then(|| trimmed.to_owned()),
                    env: Default::default(),
                },
            )),
            ClientRenameTarget::Workspace { workspace_id } => (!trimmed.is_empty()).then(|| {
                crate::api::schema::Method::WorkspaceRename(
                    crate::api::schema::WorkspaceRenameParams {
                        workspace_id,
                        label: trimmed.to_owned(),
                    },
                )
            }),
            ClientRenameTarget::NewTab {
                workspace_id,
                default_name,
            } => Some(crate::api::schema::Method::TabCreate(
                crate::api::schema::TabCreateParams {
                    workspace_id: Some(workspace_id),
                    cwd: None,
                    focus: true,
                    label: (!trimmed.is_empty() && trimmed != default_name)
                        .then(|| trimmed.to_owned()),
                    env: Default::default(),
                },
            )),
            ClientRenameTarget::Tab {
                tab_id,
                auto_name,
                original_name,
            } => (!(trimmed.is_empty() || auto_name && trimmed == original_name)).then(|| {
                crate::api::schema::Method::TabRename(crate::api::schema::TabRenameParams {
                    tab_id,
                    label: trimmed.to_owned(),
                })
            }),
            ClientRenameTarget::Pane { pane_id } => Some(crate::api::schema::Method::PaneRename(
                crate::api::schema::PaneRenameParams {
                    pane_id,
                    label: Some(trimmed.to_owned()),
                },
            )),
        };
        if let Some(method) = method {
            self.push_endpoint_method(method, outcome);
        }
        outcome.repaint = true;
    }

    pub(super) fn request_workspace_close(
        &mut self,
        workspace_id: String,
        close_group: Option<bool>,
        outcome: &mut ClientShellInput,
    ) {
        if self.config.confirm_close {
            self.open_close_confirmation(workspace_id, None, close_group);
            return;
        }
        let close_group = close_group.unwrap_or_else(|| {
            self.snapshot.as_deref().is_some_and(|snapshot| {
                snapshot.workspaces.iter().any(|workspace| {
                    workspace.workspace_id == workspace_id
                        && super::sidebar::workspace_close_is_group(snapshot, workspace)
                })
            })
        });
        self.push_endpoint_method(
            crate::api::schema::Method::WorkspaceClose(crate::api::schema::WorkspaceCloseParams {
                workspace_id,
                close_group,
            }),
            outcome,
        );
    }

    pub(super) fn request_pane_close(&mut self, pane_id: String, outcome: &mut ClientShellInput) {
        let target = self.snapshot.as_deref().and_then(|snapshot| {
            let pane = snapshot.panes.iter().find(|pane| pane.pane_id == pane_id)?;
            let tab = snapshot
                .tabs
                .iter()
                .find(|tab| tab.tab_id == pane.tab_id && tab.workspace_id == pane.workspace_id)?;
            (self.config.confirm_close
                && !snapshot
                    .panes
                    .iter()
                    .any(|other| other.tab_id == tab.tab_id && other.pane_id != pane_id)
                && !snapshot.tabs.iter().any(|other| {
                    other.workspace_id == tab.workspace_id && other.tab_id != tab.tab_id
                }))
            .then(|| (tab.workspace_id.clone(), tab.tab_id.clone()))
        });
        if let Some((workspace_id, tab_id)) = target {
            if self.open_close_confirmation(workspace_id, Some(tab_id), None) {
                if let Some(ClientShellOverlay::ConfirmClose(confirm)) = self.overlay.as_mut() {
                    if let Some(target) = confirm.tab_target.as_mut() {
                        target.pane_id = Some(pane_id);
                    }
                }
                outcome.repaint = true;
                return;
            }
        }
        self.push_endpoint_method(
            crate::api::schema::Method::PaneClose(crate::api::schema::PaneTarget { pane_id }),
            outcome,
        );
    }

    pub(super) fn request_tab_close(&mut self, tab_id: String, outcome: &mut ClientShellInput) {
        let workspace_id = self.snapshot.as_deref().and_then(|snapshot| {
            let target = snapshot.tabs.iter().find(|tab| tab.tab_id == tab_id)?;
            (self.config.confirm_close
                && !snapshot
                    .tabs
                    .iter()
                    .any(|tab| tab.workspace_id == target.workspace_id && tab.tab_id != tab_id))
            .then(|| target.workspace_id.clone())
        });
        if let Some(workspace_id) = workspace_id {
            if self.open_close_confirmation(workspace_id, Some(tab_id.clone()), None) {
                outcome.repaint = true;
                return;
            }
        }
        self.push_endpoint_method(
            crate::api::schema::Method::TabClose(crate::api::schema::TabTarget { tab_id }),
            outcome,
        );
    }

    pub(super) fn accept_close_confirmation(&mut self, outcome: &mut ClientShellInput) {
        let Some(ClientShellOverlay::ConfirmClose(confirm)) = self.overlay.take() else {
            return;
        };
        outcome.repaint = true;
        let method = if let Some(target) = confirm.tab_target {
            if target.workspace.endpoint_id != self.active_endpoint_id
                || !self.navigation_target_valid(&target.workspace)
                || !self.snapshot.as_deref().is_some_and(|snapshot| {
                    snapshot.tabs.iter().any(|tab| {
                        tab.tab_id == target.tab_id
                            && tab.workspace_id == target.workspace.workspace_id
                    })
                })
                || target.pane_id.as_ref().is_some_and(|pane_id| {
                    !self.snapshot.as_deref().is_some_and(|snapshot| {
                        snapshot.panes.iter().any(|pane| {
                            pane.pane_id == *pane_id
                                && pane.tab_id == target.tab_id
                                && pane.workspace_id == target.workspace.workspace_id
                        })
                    })
                })
            {
                self.receive_endpoint_unavailable(
                    "Close target changed; try closing it again".into(),
                );
                return;
            }
            if let Some(pane_id) = target.pane_id {
                crate::api::schema::Method::PaneClose(crate::api::schema::PaneTarget { pane_id })
            } else {
                crate::api::schema::Method::TabClose(crate::api::schema::TabTarget {
                    tab_id: target.tab_id,
                })
            }
        } else {
            crate::api::schema::Method::WorkspaceClose(crate::api::schema::WorkspaceCloseParams {
                workspace_id: confirm.workspace_id,
                close_group: confirm.close_group,
            })
        };
        self.push_endpoint_method(method, outcome);
    }

    pub(super) fn open_confirm_close_overlay(&mut self, workspace_id: String) {
        self.open_close_confirmation(workspace_id, None, None);
    }

    fn open_close_confirmation(
        &mut self,
        workspace_id: String,
        tab_id: Option<String>,
        close_group: Option<bool>,
    ) -> bool {
        let Some(snapshot) = self.snapshot.as_deref() else {
            return false;
        };
        let Some(workspace) = snapshot
            .workspaces
            .iter()
            .find(|workspace| workspace.workspace_id == workspace_id)
        else {
            return false;
        };
        let group_key = workspace
            .worktree
            .as_ref()
            .filter(|_| {
                close_group.unwrap_or_else(|| {
                    super::sidebar::workspace_close_is_group(snapshot, workspace)
                })
            })
            .map(|worktree| worktree.key.as_str());
        let group = group_key
            .map(|key| {
                snapshot
                    .workspaces
                    .iter()
                    .filter(|member| {
                        member
                            .worktree
                            .as_ref()
                            .is_some_and(|worktree| worktree.key == key)
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| vec![workspace]);
        let closes_group = group.len() > 1;
        // Keep parent-group tab closes on the existing server confirmation path.
        if tab_id.is_some() && closes_group {
            return false;
        }
        let tab_target = if let Some(tab_id) = tab_id {
            let Some(workspace) = self.navigation_target(&self.active_endpoint_id, &workspace_id)
            else {
                return false;
            };
            Some(ClientTabCloseConfirmation {
                tab_id,
                workspace,
                pane_id: None,
            })
        } else {
            None
        };
        let pane_count = group
            .iter()
            .map(|member| {
                snapshot
                    .panes
                    .iter()
                    .filter(|pane| pane.workspace_id == member.workspace_id)
                    .count()
            })
            .sum::<usize>();
        let panes = if pane_count == 1 {
            "1 pane".to_owned()
        } else {
            format!("{pane_count} panes")
        };
        let scope = if closes_group {
            format!("{} workspaces, {panes}", group.len())
        } else {
            panes
        };
        self.overlay = Some(ClientShellOverlay::ConfirmClose(
            ClientConfirmCloseOverlay {
                workspace_id,
                close_group: closes_group,
                tab_target,
                title: if closes_group {
                    "Close worktree group?".to_owned()
                } else {
                    "Close workspace?".to_owned()
                },
                detail: format!("{} â€” {scope}", workspace.label),
            },
        ));
        true
    }
}
