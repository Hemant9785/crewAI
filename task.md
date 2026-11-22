# Sketch-driven team workspace tasks

This is the ordered implementation checklist derived from the code-to-sketch comparison. Implement one numbered task at a time. Check off an item only after the behavior and verification below are complete.

## Current implementation baseline

Herdr retains Spaces and local agent panes beside the terminal. The Team panel shows the newest task and broadcast activity, scrolls through a bounded recent window, offers a broadcast composer, shows task completion announcements, and reports outbox delivery counts. The Team Dashboard supports profiles, teams, invite/join/approval, multi-member task assignment, task status actions, local /read, remote /check, and an Access view with permission decisions and grant chains.

A local agent pane can be explicitly linked to an active task assigned to the current member. That link stays in the current Herdr session and is not synchronized. Background relay sync starts after invite, join, or approval when relay credentials are available in memory. Manual sync remains available.

The Access view is inspection-only. The current code has no signed, synchronized grant/revoke command path, so the dashboard does not mutate permissions. On Windows, remote /check may require explicit --confirm-unisolated; Herdr does not claim an enforced read-only filesystem boundary in that case.

## Ordered implementation

- [x] **1.1 Make recent task and broadcast feeds browsable.**
  - Keep Spaces and Agents visible beside the terminal and show the active workspace label in the header.
  - Show recent tasks and broadcasts newest-first in the Team panel with mouse-wheel and keyboard scrolling. Retain a bounded recent window.
  - Show task completion announcements in the activity feed.
  - **Completion note:** Added independent task/activity scroll positions, newest-first rendering of the latest 100 records, and linked task hit targets. Workspace name remains visible in the existing Herdr header. Verification: `.\tools\build-herdr.ps1 -Mode test -Filter team_panel` (2 passed), `.\tools\build-herdr.ps1 -Mode test -Filter team_view` (6 passed); rustfmt check passed on touched shell files.

- [x] **1.2 Assign work to one or several teammates.**
  - Select active members by display name and role; confirm recipients before assignment.
  - Create one independently tracked task per selected member.
  - Keep signed identity and explicit authorization as the source of trust.
  - **Completion note:** The Dashboard task flow collects a description, then offers a Space-toggle member picker. Enter creates a separate signed task per selected member. Verification: focused `team_dashboard` tests passed (5 passed in the integrated source); `.\tools\build-herdr.ps1 -Mode check` passed.

- [x] **1.3 Update task state from the Dashboard.**
  - Let users select a task and apply supported status transitions from the UI.
  - Keep changes signed and recorded through the existing task command path.
  - **Completion note:** Tasks support selection and actions for accepted, in-progress, done, cancelled, and needs-attention states. Verification: focused `team_dashboard` tests passed (5 passed); `.\tools\build-herdr.ps1 -Mode check` passed.

- [x] **1.4 Announce completed tasks in team activity.**
  - Show a done task and its completing member in the shared activity/broadcast view.
  - Derive the announcement from the signed task-status history so relay replay does not create duplicates.
  - **Completion note:** Completion notices are derived from accepted Done task activity and merged into the broadcast feed without adding a second event. Verification: `team_view` tests passed (6 passed); duplicate delivery remains idempotent through the existing event projection.

- [x] **1.5 Link local agent panes to team tasks.**
  - Provide explicit link/unlink actions from an agent pane.
  - Show the linked task and state beside the local agent, and let users focus that pane from the task feed.
  - Do not launch an agent or grant repository access implicitly.
  - **Completion note:** Added pane context-menu actions, a picker restricted to active tasks assigned to the local profile, an Agents row annotation, and task-to-pane focus navigation. The mapping is local session state and resets when Herdr exits. Verification: `.\tools\build-herdr.ps1 -Mode test -Filter task_picker_tests` (1 passed), `.\tools\build-herdr.ps1 -Mode check` passed; touched UI files passed rustfmt.

- [x] **1.6 Show effective repository access in Team Dashboard.**
  - Display members, repository decisions, grant-chain identifiers, and denial reasons.
  - Keep role labels distinct from authorization decisions and show grants as directional.
  - **Completion note:** Added an Access section using the read-only permission decision path. No grant/revoke controls were added because the project has no signed synchronized API for permission changes. Verification: focused `team_dashboard` tests passed (5 passed); `.\tools\build-herdr.ps1 -Mode check` passed.

- [x] **1.7 Create and review remote repository checks in Herdr.**
  - Let a user select a teammate and repository, enter a question, and review request state and bounded result.
  - Keep remote /check distinct from local /read. Recheck permissions through the existing check command and surface the platform's isolation limitation.
  - **Completion note:** Added a Check form and bounded request result preview in the Requests section, reusing the existing /check authorization and execution path. The UI does not claim Windows unisolated checks are enforced read-only. Verification: focused `team_dashboard` tests passed (5 passed); `.\tools\build-herdr.ps1 -Mode check` passed.

- [x] **1.8 Sync team activity in the background.**
  - Start bounded background relay sync when invite/join/approval credentials become available.
  - Keep credentials in memory and out of logs, persistent storage, and command arguments.
  - Show queued, retryable, syncing, delivered, and failed event counts; preserve manual sync.
  - **Completion note:** Invite/join/approval results update an in-memory credential sink and wake the sync worker. Team panel counts come from persisted outbox records. Verification: `.\tools\build-herdr.ps1 -Mode check` passed; `delivery_summary` test passed (1); `client::team_relay_sync` tests passed (4). Physical cross-network sync is still unverified.

- [ ] **1.9 Complete a two-laptop acceptance walkthrough.**
  - Use two independent Windows clients, preferably on separate networks.
  - Create/join/approve a teammate, assign work to multiple members, update task status, verify completion activity, link a local agent pane, inspect Access, run /read and /check, and confirm background sync after a disconnect/reconnect.
  - Record screenshots or output and platform limitations.
  - **Progress note (2026-11-21):** The full Rust suite completed with 3,474 passed, 61 failed, and 12 ignored. Failures covered existing worktree, session, Windows/platform, manifest, and several shell rendering tests; focused team/dashboard/feed/sync suites and the integrated check passed. Manual acceptance is blocked until a second Windows laptop and network are available. Next action: run the documented invite/task/check workflow with a teammate laptop and verify relay reconnect.

- [x] **1.10 Ship prebuilt release binaries with one-command installers.**
  - Build Linux x86_64 and Windows x86_64 release packages from version tags.
  - Publish checksums and provide installers that download the matching binary without building source on the user's device.
  - **Completion note (2026-11-21):** Added version-tagged Linux x86_64 and Windows x86_64 packages, SHA-256 checksums, one-command installers, a Team Herdr version identity, and a guard against self-updating from upstream Herdr. GitHub Actions published release `v0.1.4`. Installers now register the `herdr` command on the current user's PATH; Linux shell startup files are updated so new terminals resolve it. Verification: release workflow run `44896414916` succeeded; both downloaded archives matched published checksums; the Windows binary reported `herdr v0.1.4`; installer syntax checks passed. Limitations: ARM64 Linux and macOS binaries are pending; installing on the EC2 instance and the two-client Internet walkthrough remain in task 1.9.

- [ ] **1.11 Enable local team profile setup on Linux.**
  - Use a persistent XDG/home database path instead of `/tmp`.
  - Support Linux signing identities without logging or transmitting their private keys; document their local storage protection.
  - Verify profile creation and identity reload in the Linux release build.
  - **Progress note (2026-11-21):** EC2 profile setup failed because Linux key storage was unsupported and the dashboard defaulted to `/tmp`. The owner explicitly approved an unencrypted, owner-only Linux identity file for this prototype. Published release `v0.1.5`; its Linux release smoke created the profile database and identity file and verified identity mode `0600`. Focused Windows-hosted tests passed for identity round-trip (2) and persistent path selection (1). Manual EC2 setup retest remains pending. Limitation: Linux identity keys are not encrypted at rest.

- [x] **1.12 Fix dashboard paste and support six-digit invites.**
  - Dashboard forms accept terminal paste and clipboard shortcuts.
  - Relay stores signed invitations temporarily; six-digit codes expire after ten minutes and are one use. Signature validation and manager approval remain required.
  - **Completion note (2026-11-22):** Fixed bracketed paste and clipboard targets in dashboard forms. Added six-digit relay lookup, one successful claim, ten-minute expiry, per-IP attempt limits, alarm cleanup, and legacy invite compatibility. Relay tests (8), typecheck, dashboard tests (7), and live isolated signed invite/join/approval/member-sync smoke passed. Deployed relay version `5d2b9e8e-87b5-416e-a29f-aa411a307823`. Published Windows/Linux release `v0.1.6` from `a449469`; workflow `45013945310` succeeded and both downloaded archives matched SHA-256 checksums. Windows reports `herdr v0.1.6`; Linux release onboarding/0600 identity smoke passed. Full suite attempted: 2,946 passed, 9 failed before two unrelated transport tests stalled; stopped the stalled run. Integrated Rust check passed. Limitations: join requests still use long signed clipboard codes; relay operator can read temporary invitations and shared relay tokens; reconnect needs a fresh short code.

- [x] **1.13 Deliver join requests automatically and decide in the dashboard.**
  - Entering an invite submits the signed join request to a bounded persistent relay inbox.
  - Manager sees pending names/devices in Requests and approves or rejects without a return code.
  - Poll requester decision status, preserve signature and invitation checks, verify replay/authorization, and publish updated binaries.
  - **Completion note (2026-11-22):** Added bounded persistent signed pairing inbox, automatic dashboard submission, authority-only request listing and decisions, five-second dashboard polling, requester decision status, and selected-request approval/rejection with no pasted return code. Decisions bind to the exact signed request; original local invitation/signature checks still gate membership. Legacy manual approval uses capital `A`. Verification: relay tests (10), typecheck, dashboard tests (9), integrated Rust build/check, and live isolated approval/rejection/status/member-sync smoke passed. Deployed relay `5edaf382-6d56-4abf-ac6a-af83bcf232af`. Published Windows/Linux release `v0.1.7` from `544ce94`; workflow `45016653224` succeeded. Both downloaded hashes verified; released Windows binary passed approval/rejection/status/task/broadcast sync smoke, and Linux release onboarding passed. Limitations: inbox polling requires an open dashboard; pairing metadata expires after seven days on access; credentials remain session-only and reconnect needs a fresh invite. Broad suite attempted with two known stalled tests excluded: 3,492 passed, 44 failed; stopped after two other transport stalls.

- [ ] **1.14 Make dashboard manager setup independent of shell relay settings.**
  - Supply the default relay URL and a random team credential; retain the credential across dashboard close/reopen in the same process.
  - Pass explicit credentials to invite creation and use distinct fresh profile IDs.
  - Verify fresh-manager create/invite/join and publish updated release binaries.
  - Review four user flows independently and record remaining issues.
  - Progress: four code reviews complete in `docs/2026-11-22-user-flow-review.md`; 12 focused dashboard tests passed; build and release in progress. Whole-app restart recovery remains outside the memory-only policy.

- [ ] **1.15 Separate pairing access from approved-device team-event reads.**
  - Review finding: pending/rejected invite holders possess a shared relay token and can fetch broadcasts. Enforce approved-device read access without weakening signatures.
- [ ] **1.16 Enforce task permissions and resolve concurrent task-state updates.**
  - Review findings: unauthorized task updates and redundant concurrent states can block synchronization.
- [ ] **1.17 Add deliberate manager credential recovery and safe profile/team editing.**
  - Preserve the current memory-only boundary until a protected persistence/recovery design is explicitly chosen.
- [ ] **1.18 Complete readable local answers and in-terminal workflows.**
  - Review findings: Codex executable discovery, clipped answers/messages, contextual slash commands, task history/followups, delivery feedback, and selection viewports.

## Known prototype limitations

- Permission inspection is available; permission grant/revoke UI needs a signed synchronized domain command.
- Agent-pane/task links reset when Herdr exits and are not shared with teammates.
- A physical two-laptop Internet acceptance run remains outstanding.
- Windows remote checks require explicit unisolated confirmation because an OS-enforced read-only boundary is unavailable.
- The one-command release installer currently supports Linux x86_64 and Windows x86_64; Linux ARM64 and macOS releases are pending.

Release artifact smoke (2026-11-22): downloaded Windows v0.1.6 binary passed live isolated profile setup, signed short-invite lookup, duplicate-claim rejection, join, approval, and member relay sync. Windows package includes ConPTY runtime and licenses.

