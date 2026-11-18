use super::*;

fn dashboard_without_database() -> ClientShellOverlay {
    ClientShellOverlay::TeamDashboard(ClientTeamDashboardOverlay {
        section: ClientTeamDashboardSection::Teammates,
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
                section: ClientTeamDashboardSection::Requests,
                ..
            }
        ))
    ));

    for (key, expected) in [
        ('1', ClientTeamDashboardSection::Teammates),
        ('2', ClientTeamDashboardSection::Tasks),
        ('3', ClientTeamDashboardSection::Broadcast),
        ('4', ClientTeamDashboardSection::Requests),
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
