use super::*;

fn dashboard_without_database() -> ClientShellOverlay {
    ClientShellOverlay::TeamDashboard(ClientTeamDashboardOverlay {
        section: ClientTeamDashboardSection::Teammates,
        task_cursor: 0,
        snapshot: Err("HERDR_TEAM_DB is not set".into()),
        form: None,
        status: String::new(),
        pending: None,
        relay_url: None,
        relay_token: None,
        share_code: None,
    })
}

#[test]
fn task_assignment_form_opens_multi_member_picker_and_toggles_selection() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    let mut dashboard = match dashboard_without_database() {
        ClientShellOverlay::TeamDashboard(dashboard) => dashboard,
        _ => unreachable!(),
    };
    dashboard.snapshot = Ok(crate::team_view::TeamDashboardSnapshot {
        profile: None,
        teams: Vec::new(),
        repositories: Vec::new(),
        teammates: vec![
            crate::team::Member {
                member_id: "member-a".into(),
                display_name: "Ada".into(),
                role: crate::team::Role::Implementer,
                active: true,
            },
            crate::team::Member {
                member_id: "member-b".into(),
                display_name: "Grace".into(),
                role: crate::team::Role::PartHead,
                active: true,
            },
        ],
        tasks: Vec::new(),
        broadcasts: Vec::new(),
        completion_announcements: Vec::new(),
        delivery: crate::team_view::DeliverySummary::default(),
        requests: Vec::new(),
        permission_graph: Vec::new(),
    });
    state.overlay = Some(ClientShellOverlay::TeamDashboard(dashboard));

    state.handle_raw_events(vec![RawInputEvent::Key(crate::input::TerminalKey::new(
        KeyCode::Char('t'),
        KeyModifiers::empty(),
    ))]);
    if let Some(ClientShellOverlay::TeamDashboard(dashboard)) = state.overlay.as_mut() {
        dashboard.form.as_mut().unwrap().fields[0].1 = TextEditor::new("Implement page", false);
    }
    state.handle_raw_events(vec![RawInputEvent::Key(crate::input::TerminalKey::new(
        KeyCode::Enter,
        KeyModifiers::empty(),
    ))]);
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::TeamDashboard(
            ClientTeamDashboardOverlay {
                form: Some(ClientTeamDashboardForm {
                    member_picker_active: true,
                    ..
                }),
                ..
            }
        ))
    ));

    state.handle_raw_events(vec![RawInputEvent::Key(crate::input::TerminalKey::new(
        KeyCode::Char(' '),
        KeyModifiers::empty(),
    ))]);
    state.handle_raw_events(vec![RawInputEvent::Key(crate::input::TerminalKey::new(
        KeyCode::Down,
        KeyModifiers::empty(),
    ))]);
    state.handle_raw_events(vec![RawInputEvent::Key(crate::input::TerminalKey::new(
        KeyCode::Char(' '),
        KeyModifiers::empty(),
    ))]);
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::TeamDashboard(ClientTeamDashboardOverlay {
            form: Some(ClientTeamDashboardForm {
                selected_member_ids,
                ..
            }),
            ..
        })) if selected_member_ids == vec!["member-a", "member-b"]
    ));
}

#[test]
fn team_dashboard_keyboard_navigation_selects_all_sections_and_wraps() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.overlay = Some(dashboard_without_database());

    state.handle_raw_events(vec![RawInputEvent::Key(crate::input::TerminalKey::new(
        KeyCode::Up,
        KeyModifiers::empty(),
    ))]);
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::TeamDashboard(
            ClientTeamDashboardOverlay {
                section: ClientTeamDashboardSection::Access,
                ..
            }
        ))
    ));

    for (key, expected) in [
        ('1', ClientTeamDashboardSection::Teammates),
        ('2', ClientTeamDashboardSection::Tasks),
        ('3', ClientTeamDashboardSection::Broadcast),
        ('4', ClientTeamDashboardSection::Requests),
        ('5', ClientTeamDashboardSection::Access),
    ] {
        state.handle_raw_events(vec![RawInputEvent::Key(crate::input::TerminalKey::new(
            KeyCode::Char(key),
            KeyModifiers::empty(),
        ))]);
        assert!(matches!(
            state.overlay,
            Some(ClientShellOverlay::TeamDashboard(ClientTeamDashboardOverlay {
                section,
                ..
            })) if section == expected
        ));
    }

    state.handle_raw_events(vec![RawInputEvent::Key(crate::input::TerminalKey::new(
        KeyCode::Esc,
        KeyModifiers::empty(),
    ))]);
    assert!(state.overlay.is_none());
}

#[test]
fn remote_check_form_separates_question_from_local_read_and_selects_one_target() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    let mut dashboard = match dashboard_without_database() {
        ClientShellOverlay::TeamDashboard(dashboard) => dashboard,
        _ => unreachable!(),
    };
    let mut snapshot = crate::team_view::TeamDashboardSnapshot::empty();
    snapshot.repositories.push("api-repo".into());
    snapshot.teammates.push(crate::team::Member {
        member_id: "member-b".into(),
        display_name: "Grace".into(),
        role: crate::team::Role::Implementer,
        active: true,
    });
    dashboard.snapshot = Ok(snapshot);
    state.overlay = Some(ClientShellOverlay::TeamDashboard(dashboard));

    state.handle_raw_events(vec![RawInputEvent::Key(crate::input::TerminalKey::new(
        KeyCode::Char('q'),
        KeyModifiers::empty(),
    ))]);
    if let Some(ClientShellOverlay::TeamDashboard(dashboard)) = state.overlay.as_mut() {
        let form = dashboard.form.as_mut().unwrap();
        form.fields[1].1 = TextEditor::new("Review the API", false);
    }
    state.handle_raw_events(vec![RawInputEvent::Key(crate::input::TerminalKey::new(
        KeyCode::Enter,
        KeyModifiers::empty(),
    ))]);
    state.handle_raw_events(vec![RawInputEvent::Key(crate::input::TerminalKey::new(
        KeyCode::Enter,
        KeyModifiers::empty(),
    ))]);
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::TeamDashboard(
            ClientTeamDashboardOverlay {
                form: Some(ClientTeamDashboardForm {
                    action: ClientTeamDashboardAction::Check,
                    member_picker_active: true,
                    ..
                }),
                ..
            }
        ))
    ));

    state.handle_raw_events(vec![RawInputEvent::Key(crate::input::TerminalKey::new(
        KeyCode::Char(' '),
        KeyModifiers::empty(),
    ))]);
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::TeamDashboard(ClientTeamDashboardOverlay {
            form: Some(ClientTeamDashboardForm {
                selected_member_ids,
                ..
            }),
            ..
        })) if selected_member_ids == vec!["member-b"]
    ));
}

#[test]
fn access_dashboard_renders_member_repository_grant_chain() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    let mut dashboard = match dashboard_without_database() {
        ClientShellOverlay::TeamDashboard(dashboard) => dashboard,
        _ => unreachable!(),
    };
    let mut team_snapshot = crate::team_view::TeamDashboardSnapshot::empty();
    team_snapshot
        .permission_graph
        .push(crate::team_view::PermissionGraphRow {
            member_id: "member-b".into(),
            display_name: "Grace".into(),
            role: crate::team::Role::Implementer,
            resource: "repository api-repo".into(),
            action: "read repository".into(),
            active: true,
            allowed: true,
            reason: "Allowed".into(),
            grant_chain: vec!["grant-manager-reader".into()],
        });
    dashboard.section = ClientTeamDashboardSection::Access;
    dashboard.snapshot = Ok(team_snapshot);
    let overlay = ClientShellOverlay::TeamDashboard(dashboard);
    let mut buffer = Buffer::empty(Rect::new(0, 0, 106, 30));
    render::render_client_overlay(
        &mut buffer,
        &overlay,
        state.snapshot.as_deref().expect("snapshot"),
        &state.endpoints,
        &state.active_endpoint_id,
        &state.config.keybinds,
        &state.config.palette,
    )
    .expect("dashboard render");
    let text = buffer
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(text.contains("access"));
    assert!(text.contains("member-b"));
    assert!(text.contains("grant-manager-reader"));
}

#[test]
fn team_dashboard_renders_missing_database_without_creating_one() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    let overlay = dashboard_without_database();
    let mut buffer = Buffer::empty(Rect::new(0, 0, 106, 30));
    let rendered = render::render_client_overlay(
        &mut buffer,
        &overlay,
        state.snapshot.as_deref().expect("snapshot"),
        &state.endpoints,
        &state.active_endpoint_id,
        &state.config.keybinds,
        &state.config.palette,
    )
    .expect("dashboard render");

    let text = buffer
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(text.contains("Team dashboard"));
    assert!(text.contains("Team data unavailable"));
    assert!(text.contains("HERDR_TEAM_DB"));
    assert_eq!(rendered.primary, rendered.area);
}
