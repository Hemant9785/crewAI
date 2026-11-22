use super::*;
use crate::cli::join_inbox;

impl ClientShellState {
    pub(super) fn tick_join_inbox(&mut self) -> bool {
        let Some(ClientShellOverlay::TeamDashboard(d)) = self.overlay.as_mut() else {
            return false;
        };
        let mut changed = false;
        if let Some(receiver) = &d.join_inbox.pending {
            match receiver.try_recv() {
                Ok(result) => {
                    d.join_inbox.pending = None;
                    match result {
                        Ok(inbox) => {
                            d.join_inbox.entries = inbox.requests;
                            d.join_inbox.cursor = d
                                .join_inbox
                                .cursor
                                .min(d.join_inbox.entries.len().saturating_sub(1));
                            d.join_inbox.status = inbox.status;
                            d.join_inbox.error = None;
                            d.snapshot =
                                crate::team_view::load_default().map_err(|e| e.to_string());
                        }
                        Err(error) => d.join_inbox.error = Some(error),
                    }
                    changed = true;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    d.join_inbox.pending = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        if d.pending.is_some()
            || d.join_inbox.pending.is_some()
            || d.join_inbox
                .next_poll
                .is_some_and(|next| next > std::time::Instant::now())
        {
            return changed;
        }
        let Some((url, token)) = d.relay_url.clone().zip(d.relay_token.clone()) else {
            return changed;
        };
        let Ok(snapshot) = &d.snapshot else {
            return changed;
        };
        let (Some(team), Some(profile)) = (snapshot.teams.first(), snapshot.profile.as_ref())
        else {
            return changed;
        };
        let team_id = team.team_id.clone();
        let manager = team.authority_member_id == profile.member_id;
        let database = crate::team_view::default_database_path();
        let identity = join_inbox::identity_path(&database);
        let (sender, receiver) = std::sync::mpsc::channel();
        d.join_inbox.pending = Some(receiver);
        d.join_inbox.next_poll =
            Some(std::time::Instant::now() + std::time::Duration::from_secs(5));
        std::thread::spawn(move || {
            let result = if manager {
                join_inbox::command(&url, &team_id, &token, &identity, "list", "", "")
            } else {
                crate::identity::DeviceIdentity::load_existing(&identity)
                    .map_err(|e| e.to_string())
                    .and_then(|key| {
                        join_inbox::command(
                            &url,
                            &team_id,
                            &token,
                            &identity,
                            "status",
                            key.device_id(),
                            "",
                        )
                    })
            };
            let _ = sender.send(result);
        });
        changed
    }

    pub(super) fn decide_selected_join(&mut self, approve: bool) {
        let Some(ClientShellOverlay::TeamDashboard(d)) = self.overlay.as_mut() else {
            return;
        };
        if d.pending.is_some() {
            return;
        }
        let Some(entry) = d.join_inbox.entries.get(d.join_inbox.cursor).cloned() else {
            return;
        };
        let Some((url, token)) = d.relay_url.clone().zip(d.relay_token.clone()) else {
            return;
        };
        let database = crate::team_view::default_database_path();
        let identity = join_inbox::identity_path(&database);
        let (sender, receiver) = std::sync::mpsc::channel();
        d.status = if approve {
            "Approving teammate..."
        } else {
            "Rejecting join request..."
        }
        .into();
        d.pending = Some(receiver);
        d.join_inbox.next_poll = None;
        std::thread::spawn(move || {
            let request = entry.request;
            let result = (|| -> Result<(), String> {
                join_inbox::decide(&database, &identity, &url, &token, &request, approve)
            })();
            let status = match result {
                Ok(()) => format!(
                    "{} {}",
                    if approve { "Approved" } else { "Rejected" },
                    request.member.display_name
                ),
                Err(e) => format!("Could not complete: {e}"),
            };
            let _ = sender.send(ClientTeamDashboardResult {
                status,
                relay_credentials: None,
                share_code: None,
            });
        });
    }
}
