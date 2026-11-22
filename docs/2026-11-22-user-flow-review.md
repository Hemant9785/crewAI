# User flow review: EC2 manager and Windows teammate

Four independent agents reviewed `/read`, profiles/teams, tasks, and broadcasts against the code. This is a code review with narrow isolated checks, not a completed physical EC2-to-Windows UI walkthrough. Findings were collected from v0.1.7 / commit `cdf96e2` and the current setup patch; line numbers in agent evidence refer to that baseline.

## Outcome

The dashboard provides the main prototype paths, but several important parts of the full user flow remain incomplete. In particular, rejecting a join does not prevent that invite holder from reading relay messages, task permissions need enforcement, and manager connection recovery after restart is missing. The setup patch below does not resolve those larger findings.

| Area | Implemented | Main remaining problem |
| --- | --- | --- |
| Team setup | Profile, team creation, six-digit invite, automatic signed request, approve/reject | Manager connection is lost on full application restart without shell credentials |
| Local `/read` | Separate bounded Codex process, approved local repo lookup, requested read-only sandbox | Executable discovery and readable answer presentation |
| Tasks | Assign multiple members, inbox, state changes, completion notices, CLI follow-ups | Permission checks and concurrent state convergence |
| Broadcast | Signed team message, local persistence, relay synchronization, recent feed | Preapproval read access, group privacy, full-message reading |

## Setup fixes included in v0.1.8

- **Missing relay configuration:** dashboard CreateTeam now supplies the project relay URL and a cryptographically random token. It passes credentials in the child environment, not command arguments. Invite uses those explicit in-memory credentials rather than looking only in the parent's environment. Evidence: `start_team_dashboard_action`, `prepare_team_dashboard_relay` in [overlay_input.rs](../herdr/src/client/shell/overlay_input.rs), and `create_invite_bundle_with_credentials` in [team.rs](../herdr/src/cli/team.rs).
- **Duplicate default member IDs:** fresh profile forms generate distinct member IDs; an existing profile's ID is preserved. The team reviewer reproduced the former `member-1` collision using isolated v0.1.7 manager/implementer profiles: join failed with `team membership conflicts with existing member` after claiming the invite. Evidence: profile form in [overlay_input.rs](../herdr/src/client/shell/overlay_input.rs), `ensure_member` in [team_storage.rs](../herdr/src/team_storage.rs).
- **Closing the dashboard during creation:** generated credentials enter the shared in-memory sink before the child starts, so dropping the overlay does not discard the only session copy.
- **Successful creation message:** the dashboard directs the user to invite a teammate instead of displaying CLI environment-configuration instructions.

These changes do not persist credentials across a full app exit. Keep the new manager session open during the current two-device test.

## Highest priority remaining findings

### Pending or rejected invite holders can read broadcasts

**Evidence:** accepting an invitation installs the shared relay token and local active member/device records (`accept_invitation` in [team_storage.rs](../herdr/src/team_storage.rs)). [relay.rs](../herdr/src/cli/relay.rs) downloads team events using that token. `TeamRelay.page` in [index.ts](../relay/src/index.ts) authenticates the shared token, without requiring an approved requesting device. Rejection in [join-inbox.ts](../relay/src/join-inbox.ts) changes inbox status without removing that credential's access.

**User impact/reproduction:** give an invite to a second isolated profile; leave it pending or reject it; let that profile sync. Authority-signed broadcasts can still be fetched and accepted. This finding is source-confirmed; the reviewer did not run this scenario against personal team data.

**Next action:** separate pairing credentials from approved event access, and authenticate approved devices for relay reads. Signature validation alone does not provide read confidentiality. Do not describe Reject as removing shared-token read access until this is fixed.

### Task authorization is incomplete

**Evidence:** [task.rs](../herdr/src/cli/task.rs) inserts signed task events. `TeamStore::insert_event` and task projection in [team_storage.rs](../herdr/src/team_storage.rs) check active membership/device but do not consistently enforce CreateTask/UpdateTask grants, task ownership, or active assignee membership. Access decisions in [team_view.rs](../herdr/src/team_view.rs) are inspection only.

**User impact/reproduction:** an approved implementer can create a task or change another member's task through the CLI even when Access reports denial. A signature identifies the actor; it does not establish the task permission.

**Next action:** enforce task-specific permissions in the common local/remote event validation path. Permit authorized recipient updates and explicit manager/lead authority; reject other actors and invalid assignees.

### Concurrent task changes can stop synchronization

**Evidence:** both devices can independently move Delivered to InProgress. After one change is projected, the other's InProgress event fails `validate_task_transition` in [team.rs](../herdr/src/team.rs). Task projection propagates the error through `apply_remote_events` in [sync.rs](../herdr/src/sync.rs), preventing relay cursor progress.

**User impact/reproduction:** manager and implementer both press `w` before syncing. The duplicate state change can block later valid tasks and broadcasts.

**Next action:** retain signed history and project causally redundant updates idempotently; define deterministic conflict handling without allowing arbitrary backwards transitions.

### Manager restart recovery is missing

**Evidence:** `open_team_dashboard` and [team_relay_sync.rs](../herdr/src/client/team_relay_sync.rs) load the current in-memory sink or shell configuration. Generated relay credentials are not persisted. The setup patch fixes overlay-close loss, not full process exit.

**User impact:** a manager that created a team without shell settings cannot invite, sync, or review pending requests after restarting. Asking the manager for a fresh invite cannot recover the manager itself.

**Next action:** deliberately design protected local credential recovery. This requires updating the existing memory-only storage policy; avoid silently storing plaintext tokens as a workaround.

### `/read` does not reliably launch or display its answer

**Evidence:** [read.rs](../herdr/src/cli/read.rs) hardcodes `codex.exe`. The read reviewer confirmed it is absent from this machine's persistent Machine+User PATH, although this Codex session's injected PATH finds the extension executable. Native Linux uses `codex`, so the same hardcoded filename also fails there. Dashboard completion takes the first 2,000 output characters, and [overlays.rs](../herdr/src/client/shell/overlays.rs) renders multiline status as one row; there is no answer viewer.

**User impact:** the Windows teammate can have this working Codex conversation while Herdr cannot start Codex. A launched query can produce an answer that is clipped or displaced by execution output.

**Next action:** resolve a native configurable executable, check prerequisites, capture the final answer separately, and show a bounded wrapped/scrollable result. Validate OS isolation separately; the code requests a read-only sandbox, but the review did not prove its runtime enforcement.

## Other usability gaps

| Area | Evidence and user impact | Smallest next step |
| --- | --- | --- |
| Natural slash input | Embedded terminal sends input to PowerShell/bash. `cli.rs` exposes executable aliases requiring explicit options; `/read question`, `/task @person text`, and `/broadcast text` are not intercepted user commands. | Add a contextual command composer or clearly present dashboard shortcuts as the supported prototype flow. |
| Profile editing | `p` starts setup again; [onboard.rs](../herdr/src/cli/onboard.rs) overwrites local profile/device metadata. Editing the member ID after joining can break references; label changes are not signed shared-member edits. | Prefill current values, lock stable member identity once enrolled, and distinguish local labels from shared edits. |
| Team editing | `c` creates another team; there is no signed rename/update command. Dashboard actions select the first team. | Guard repeated creation in the one-team prototype; add a deliberate signed edit path. |
| Task delivery status | Offline task creation saves locally and returns success with a warning on stderr. Dashboard success handling discards stderr and reports Completed/Assigned. | Show saved/queued/delivered outcomes accurately. |
| Follow-ups/history | CLI supports task follow-up/show; the dashboard has no corresponding form/details view. | Add selected-task details and a follow-up action. |
| Hidden selections | Task and join-inbox cursors can move beyond the visible rows. | Render a viewport around the selected row. |
| Broadcast groups | UI composers send team scope. CLI scope is a label; all team events sync and the feed does not enforce group membership filtering. | Do not present scope labels as privacy. Implement group authorization/filtering before private groups. |
| Broadcast composer feedback | Panel draft is cleared before completion, and its child lacks in-memory relay credentials. Success may only mean saved locally. | Preserve failed drafts and distinguish queued from delivered. |
| Long broadcasts | Panel/dashboard render a message as one clipped row; no full-message viewer exists. | Provide a selected-message detail view and wrapping. |
| `/read` prerequisites | Non-Git directories can be approved, but the runner does not use Codex's `--skip-git-repo-check`. The agent subprocess also inherits relay settings unnecessarily. | Support approved ordinary folders explicitly or validate Git setup; remove relay credentials from the Codex environment. |
| Closing `/read` | Closing the dashboard drops the result receiver while the bounded worker continues. | Retain a local result or add cancellation. |

## Review verification

- Team reviewer: reproduced duplicate member-ID join failure with isolated profiles and v0.1.7. No personal profile was edited.
- Read reviewer: inspected actual launch/render paths, persistent versus injected PATH, and installed `codex exec --help`. No real model query was submitted.
- Task reviewer: existing signed task lifecycle and two-profile convergence tests each passed. These tests do not cover unauthorized mutation or concurrent-state conflicts.
- Broadcast reviewer: five existing CLI/projection/scope-validation tests passed. This does not establish group confidentiality or preapproval read isolation.
- Root: setup regressions, build, release evidence recorded in `task.md` as checks complete. Existing broad-suite failures and transport stalls remain documented there.

Recommended implementation order: approved-device relay read access; shared task authorization/convergence; manager recovery; readable `/read` with native executable discovery; profile/team edits; in-dashboard details/follow-ups and contextual slash commands.

Release verification: v0.1.8 workflow `45019140816` succeeded for Windows and Linux. Both downloaded package checksums matched. Windows v0.1.8 passed the isolated pairing/approval/rejection/status/task/broadcast smoke and was installed locally. Linux onboarding passed in the release workflow. Dashboard regression suite: 12 passed. Physical EC2-to-Windows UI retest remains pending.
