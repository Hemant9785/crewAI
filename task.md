# crewAI conversational terminal implementation

Status: implementation in progress; the conversation screen and command flows are not yet available for user testing.
Approved: 2026-11-22. This file replaces the previous task checklist at the owner's request. Earlier implementation history remains in Git and existing validation documents.

## Outcome

Inside a Herdr terminal, typing `/crewai` opens our own conversational terminal interface. It has one input, discoverable slash commands, teammate mentions, readable answers and contextual actions. Spaces, Agents and other terminal panes remain usable. Everyday work must be possible without visiting the old Team dashboard or running setup commands in another PowerShell window.

Design references:

- [Conversation inside Herdr](design/crewai-conversation-reference.png)
- [Commands, teammate picker and inline approval](design/crewai-command-flow-reference.png)
- [Reference interpretation](design/README.md)

These images define the intended experience. Their sample messages and success states are illustrative; implementation must use real domain operations, synchronized records and assistant results.

## Required command contract

| Command | Intended behavior |
| --- | --- |
| `/read <question>` | Ask about this device's connected project. Guide project/assistant setup inline if needed. |
| `/read <question> @teammate` | Ask the selected teammate's project; receive its owner's decision and actual assistant answer in the conversation. Also accept the mention before the question. |
| `/team-mates` | Show teammates and contextual profile, Create/Join, Invite, approve/reject and project-connection actions. |
| `/task @teammate <description>` | Assign a real task; show acceptance, progress, completion and follow-ups inline. |
| `/broadcast <message>` | Send a team message and show truthful queue/delivery state. |
| `/message @teammate <message>` | Send a recipient-only human message; show the real human reply in context. |

Typing `/` discovers the five commands; typing `@` chooses a person. Names are presentation labels; operations use stable signed member/device IDs internally. Do not require users to type IDs, repository identifiers, relay URLs, tokens or JSON files.

## Existing code to reuse

Inspection confirms the following foundations exist; their presence does not mean the new conversation UI or private messaging is implemented:

- `herdr/src/team_session.rs`: profile, project connection, protected credentials/drafts and saved team contexts.
- `herdr/src/cli/questions.rs`, `read.rs`, `assistant.rs`: project announcements, owner consent, requests and bounded native assistant answers.
- `herdr/src/cli/team.rs`, `join_inbox.rs`, `task.rs`, `broadcast.rs`: signed team operations, pairing, tasks and team broadcasts.
- `herdr/src/team_storage.rs`, `team_storage/permission_events.rs`: SQLite/event projections and consent validation.
- `herdr/src/cli/relay.rs`, `client/team_relay_sync.rs`, `relay/src/`: signed-device Internet synchronization.
- `herdr/src/client/shell/`: existing dashboard, terminal composition, mouse/input handling and Spaces/Agents integration.

The current broadcast service is not a private-message backend. The current Requests/dashboard UI is not the new conversation view. Reuse working services and records; keep the old UI as a compatibility path until the new flows work.

## Implementation rules

Implement one numbered task at a time, in order. Build working vertical flows, not placeholder screens. Start with domain/parser/protocol checks, then connect the UI to verified operations. All checkboxes below remain unchecked until behavior and its verification are complete.

Keep UI, command parsing, domain services, persistence, synchronization and assistant execution separated. Use migrations for schema changes. Preserve existing teams, identities, credentials, approvals, tasks and messages. Do not weaken signatures, device approval, permissions, revocation, replay protection or process isolation to simplify the interface. Repository files stay on their owner's device. Never label assistant execution read-only unless the OS boundary enforces it; limited isolation requires informed explicit confirmation.

Prefer the smallest working Windows/Linux prototype. Defer speculative edge cases and unrelated refactors; required authorization, input validation and data preservation are part of the prototype.

## Ordered implementation checklist

### 1. Lock the conversation and launch contract

- [x] Update `spec.md` with the approved pane-based conversation, the five commands, local/remote `/read` semantics and direct-message access boundary. Preserve its existing trust guarantees.
- [x] Define `/crewai` entry at an idle supported shell prompt, how it launches/resumes the session, and how Esc returns safely to the shell. Provide `herdr crewai` as the executable entry used by integration and automation.
- [x] Define focus rules: arrows navigate an open picker or selected transcript action; editing keys edit text; no implicit switching between dashboard sections.
- [x] Define plain-text follow-up behavior only for a visibly selected conversation/recipient; ambiguous input must not silently broadcast or start an assistant request.

Verification: walk the approved example conversation against both references, document the chosen launch mechanism and exact interaction rules before application edits.

Completion note: Added the approved conversation, command, consent and recipient-only DM contract to `spec.md`. The entry uses shell-local command registration for interactive PowerShell and non-login Bash and `herdr crewai` as the executable; unsupported shells keep the explicit command. Walked the five reference commands and inline approval against the written contract. PowerShell 5 and Git Bash accepted a shell-local function named `/crewai`; Linux runtime validation remains under task 6.

Entry implementation update: `herdr crewai` exists, and the literal PowerShell function opened it inside an isolated Windows Herdr pane and returned to the shell on Escape. Linux runtime validation remains for EC2 under task 6.

### 2. Implement command and mention parsing

- [x] Add a typed parser for the five commands, their bodies and teammate references; keep it independent of rendering and shell execution.
- [x] Accept the approved examples, including a mention at the end of `/read`, spaces, multiline paste and URLs without losing message text.
- [ ] Resolve picker choices to stable member IDs. Ambiguous or missing names open a picker rather than guessing a target.
- [x] Add focused parser tests for valid commands, unknown/empty commands, missing recipients, duplicate display names and malformed/oversized input. Invalid input must not dispatch an operation.

Verification: parser tests pass before connecting commands to application services.

Completion note: `crewai_command.rs` parses the five commands, returns explicit recipient-picker choices for missing or duplicate names and resolves selected IDs against active members. Verification: `powershell -ExecutionPolicy Bypass -File .\tools\build-herdr.ps1 -Mode test -Filter crewai_command::tests` passed 3/3, including multiline/URL, malformed/oversized and duplicate-name cases. Limitation: rendering/opening the picker is part of task 7; no UI dispatch is connected yet.

### 3. Expose shared application services

- [x] Extract/reuse callable services for profile/team actions, project setup, tasks, broadcasts and questions; do not make the new UI depend on legacy overlay state.
- [x] Expose bounded progress/result events correlated to operation IDs, with consistent domain errors and queue/delivery semantics.
- [x] Run synchronization and assistant work outside input/render loops; reuse the saved active-team connection and avoid global credentials replacing it.
- [x] Verify operations directly against isolated local profiles before rendering success messages. Preserve existing CLI/dashboard compatibility.

Verification: service tests cover real persistence, authorization and operation results; no subprocess CLI output scraping is required by the conversation renderer.

Progress note (2026-11-23): Added `crewai_service.rs` with signed task/broadcast operations, local/remote question/project wrappers, profile/create/join/invite/review methods, saved-connection sync, and correlated bounded background progress/results. Queue bookkeeping is distinct from durable save. Added team-scoped member lookup and a regression that excludes members of another team. Verification: isolated service tests 3/3; existing CLI tests 147/147; storage tests 23/23 (one existing opt-in test ignored). The isolated local Worker test passed end to end: create team, issue/claim invite, submit/approve join, sync membership, assign task, broadcast and verify both records on the teammate. An equivalent synthetic test against the deployed relay returned HTTP 500 at team bootstrap; the pre-existing released CLI reproduced it. A filtered production error trace identified `Exceeded allowed rows read in Durable Objects free tier`; `/healthz` still returned 200. Cloudflare documents a 00:00 UTC daily free-limit reset (05:30 India time). No billing changes were made. The service exposes background start/result channels for sync and assistant execution; the later UI task must consume those channels. The UI has not yet dispatched the service, and production Internet delivery remains unverified until quota resets.

Completion note: reusable team/profile, pairing, project, task, broadcast, question and sync interfaces use the same authenticated domain and encrypted saved connection. Sync and assistant start methods dispatch off-thread and return correlated bounded updates; the UI integration is deferred to its ordered tasks. Verification: `crewai_service::tests` 3 passed and local relay invite/approval/task/broadcast integration 1 passed; existing CLI 147 passed and storage 23 passed (one opt-in ignored). Limitations: cloud relay quota exceeded; no real Codex model or physical EC2/Windows run performed.

### 4. Build a genuine direct-message backend

Design decision: recipient-scoped authenticated relay mailbox, separate from the team event stream. The relay operator can read message bodies; other approved members cannot fetch them. The signed envelope binds both participants and a stable ID. See `spec.md` for validation and local retention requirements.

- [x] Define signed direct-message records with stable IDs, sender, recipient, bounded body, reply/thread association and delivery metadata; add migrations and projections.
- [x] Implement sender/recipient access in transport and storage. Keep plaintext DMs out of the team-readable broadcast event stream; choose recipient-scoped authenticated delivery or encrypted envelopes in the architecture contract.
- [x] Implement offline queueing, reconnect delivery and real recipient replies using the existing identity/sync foundations.
- [x] Add positive two-person tests and a third-member negative test: an unrelated approved teammate cannot fetch/read the DM body. Reject forged sender, wrong-team, revoked-device and replayed/conflicting input.

Verification: private message and reply work before adding their UI; team broadcast APIs cannot substitute for this task.

Progress note (2026-11-23): Added signed bounded envelopes, migration, recipient-scoped relay mailbox, offline local queue and cursor-based synchronization. The relay operator can read message bodies. Relay tests pass (12/12), including denial to an unrelated approved teammate, conflicting signed IDs and replayed proofs; the storage validation test passes. An isolated Rust-to-local-Worker integration test passed team setup, approval, private message delivery and an actual reply. Older relays continue ordinary team synchronization; messages stay queued until a mailbox-capable relay is available. LAN parity and the conversation UI remain pending, so this is not yet a usable `/message` feature.

Additional verification: reopening the sending service before sync preserved and delivered the queued message, then the recipient's reply returned successfully. The full Herdr suite did not finish: several `server::client_transport` tests hung beyond 60 seconds in parallel, while an individual handshake test passed. A serial rerun progressed past 3,000 tests but stalled in the remote bridge tests. It also reported failures in existing agent-status, Git refresh and Windows input tests. Do not treat the full suite as passing. Next: investigate the bridge stall and rerun the full suite before closing this phase.

Additional sync fix: ordinary sync previously re-uploaded the complete event history every five seconds, which can exhaust the production relay quota and prevent message delivery. Added a migration-backed per-team/per-endpoint acknowledgement cache; only successfully acknowledged or verified downloaded events are cached, and unacknowledged events remain retryable. Verification: the isolated local relay integration passes team creation, join/approval, task, broadcast, direct message and reply; an idle sync reports zero re-uploads and different endpoints do not share acknowledgements. The schema migration test preserves an existing cursor. Production quota behavior remains unverified.

Completion note: direct messages use a separate recipient-scoped authenticated mailbox, signed envelopes, durable queues and reply associations; the relay operator can read message text. The local Worker integration and relay tests verify two-person delivery/replies and third-person denial. A full-suite run with two previously stalling bridge tests run separately finished: 3,562 passed, 12 failed, 14 ignored; both isolated bridge tests passed. Several Git failures occur because the test temporary directory is inside this repository; other failures include agent status, Windows input, terminal rendering and Windows backup tests. The suite is not green. Direct messages currently require the Internet relay; LAN-only direct-message transport and production deployment remain unverified, and the conversational UI is not implemented yet.

### 5. Add the durable conversation model

- [x] Define entries for user commands, assistant answers, human messages, broadcasts, task activity, approvals, progress and failures, with explicit speaker/recipient identity.
- [x] Associate entries with stable request/event/task IDs so synchronization and refresh update the correct item without duplicate replies or moving an approval to a different question.
- [x] Persist/reconstruct bounded history and drafts per saved team and relevant thread; migrate existing records without deleting or reassigning them. Local `/read` history stays local.
- [x] Verify restart recovery, duplicate event delivery and multiple simultaneous questions with a completed question followed by another pending question.

Verification: deterministic model/storage tests pass; histories and drafts remain separated across team switching.

Progress note (2026-11-23): Added a local schema for stable conversation entries, explicit links to related questions/tasks/messages and team/thread-scoped encrypted drafts. Updates preserve the original entry identity, reject stale or conflicting replacements, and retain at most 500 entries per thread. Four focused tests pass for restart, duplicate events, simultaneous questions, bounded retention, migration and draft isolation. The local relay integration confirms reconstruction of actual tasks, broadcasts and human replies. UI integration is still pending.

Completion note: history and protected drafts are stored locally per team/thread; signed team records and participant-only direct messages reconstruct into stable entries with related IDs. Verification: four conversation tests and the local two-profile relay/thread integration passed. The wider suite was run in two serial groups with the two interfering bridge tests run separately: 3,566 passed, 12 failed, 14 ignored, plus both bridge tests passed; the 12 existing failures match the preceding phase. Several Git failures are caused by the temporary directory being inside this repository. Local `/read` entry creation and the visible screen are part of later tasks and have not yet been manually exercised.

### 6. Launch the interface inside the terminal pane

- [ ] Implement the actual `herdr crewai` terminal session and wire literal `/crewai` entry through supported Herdr shell integration on Windows and Linux without manual alias setup.
- [x] Match the reference layout: compact team header, scrollable conversation and bottom composer inside the active pane; keep Spaces, Agents and other panes usable.
- [ ] Restore terminal input modes, screen and shell prompt on exit/error. Reopening resumes the correct team's history and draft.
- [ ] Ensure the entry hook does not capture `/crewai` text inside an editor, another agent, a pasted script or a running foreground program; do not globally steal terminal typing.

Verification: launch and exit in a real Windows terminal and Linux shell; existing pane/tab/agent behavior remains functional. Verify the literal `/crewai` entry, not only a menu or fallback command.

Progress note (2026-11-23): Added the `herdr crewai` pane process with alternate-screen cleanup, saved drafts, actual history, background sync and initial dispatch to real team, task, broadcast, private-message and remote-question services. Added a PowerShell `/crewai` function and a non-login Bash rc hook; isolated PowerShell and Git Bash accepted the function. The composer unit test passed for real broadcast/task/private-message persistence and invalid-draft recovery. An isolated Windows Herdr instance with synthetic team data accepted literal `/crewai`, displayed history inside its pane while Spaces/Agents remained visible, returned to PowerShell on Escape, and ran another shell command afterward. This is incomplete: onboarding, local `/read`, pickers and approval actions are not yet available in the conversation, and a Linux build and embedded Linux pane launch/exit have not been verified. Login Bash and other shells currently retain the `herdr crewai` fallback.

Linux verification is deferred to the owner's EC2 instance at their request; do not use Ubuntu on this laptop. A Windows-hosted Linux cross-check could not complete because the vendored terminal library needs a Linux symbolic link that Windows denied. Next action: build the Linux artifact through the release workflow and verify `/crewai` in the EC2 Herdr pane.

Windows follow-up: the shell prompt hook was tested in an isolated Herdr instance, and transcript scrolling now measures wrapped rows. The active team database is migrated once on service open; transcript refreshes use read-only database access. The focused composer and local relay integration tests passed. The full suite was run in two serial groups with two interfering bridge tests separately: 3,567 passed, 12 failed, 14 ignored and both isolated bridge tests passed. The failures match the preceding phase and the suite is not green. The shell entry is a shell function, so quoted `/crewai` text remains ordinary output; Linux and foreground/editor behavior still require EC2 validation.

Implementation handoff: Windows pane launch and the implemented shell hooks are ready for the next ordered task. The remaining task 6 checkboxes stay open for the owner's EC2 build/pane validation and any fixes it reveals; no Linux success is claimed.

### 7. Implement the composer and discoverable pickers

- [ ] Show `/read`, `/team-mates`, `/task`, `/broadcast`, `/message` with short descriptions when `/` is entered; filter as the user types.
- [ ] Open teammate autocomplete on `@`, support Up/Down, Enter, Escape and mouse selection, and preserve the rest of the input when inserting a mention.
- [ ] Support normal editing, paste, wrapped input and command history; maintain a visible destination/context for follow-ups.
- [ ] Preserve drafts on cancellation/failure and show clear inline corrections. Keep the focused row visible in pickers and long conversations.

Verification: interaction tests and keyboard/mouse checks demonstrate command discovery and recipient selection without memorized bracket keys or section navigation.

Progress note (2026-11-23): Added command and teammate suggestion state, Unicode-aware editing, paste, command history, keyboard and mouse picker handling, wrapped input and duplicate-name selection by stable member ID. Quoted mentions support names with spaces. Focused `crewai_` tests passed (11 passed, one opt-in relay integration ignored); the duplicate-name test verifies the task reaches the chosen member. A current Windows binary is being built for interaction verification. Follow-up selection and full-suite checks remain pending.

Batching update: Added visible message selection and real private follow-ups, plus tests for ambiguous names and unaddressed text. These latest edits are not yet compiled or interaction-tested. Per the owner's request, related changes will be compiled and tested together rather than after each small edit; checkboxes remain open until verified.

### 8. Complete onboarding and `/team-mates`

- [ ] Show actual people, role labels and approval state in the conversation; do not equate approved membership with verified online presence.
- [ ] Offer inline name-only Create/Join and manager Invite actions. Display six digits with expiry, Copy feedback and manual-entry fallback; join requests and approve/reject actions arrive inline.
- [ ] Expose existing profile editing and safe saved-team switching. Offer project connection and a visible connected-project status/action; coordination-only EC2 requires no project or Codex installation.
- [ ] Restore saved connections after restart. Keep repository ownership/consent separate from joining a team and explain pending approval or failed sync in place.

Verification: fresh manager and teammate can create/join/approve and connect a project from this session without JSON handoff, external commands or opening the old dashboard.

Progress note (2026-11-23): Added an in-pane welcome flow for create/join and a `/team-mates` panel for invite, join-request review, optional project connection, local name editing, creating/joining another team and saved-team switching. Invite codes are displayed without being placed in conversation history; clipboard copy is explicitly described as a request with manual-entry fallback. Join requests are polled in the background and pending membership is not labelled approved. The combined Windows compile check passed; full in-pane pairing and user interaction verification remain pending.

Batching note: the follow-on project-question controls below have been added but not yet compiled with this setup flow. User-facing verification and checkboxes remain open.

### 9. Complete conversational `/read`

- [ ] Execute local `/read` using the selected local project and real native assistant; guide missing project/assistant/sign-in setup inline and preserve the question draft.
- [ ] Execute `/read ... @teammate` through the signed remote question service, showing the chosen owner/project and queued/waiting/running states in the conversation.
- [ ] Present incoming owner consent beside the exact question with Allow once, Allow project and Decline. Show accurate limited-isolation information and require explicit confirmation; recheck permissions before execution.
- [ ] Show the actual bounded answer inline with readable paragraphs/code and distinct assistant attribution. Show progress honestly; do not simulate streaming a canned answer. Preserve answers/errors and support a real follow-up to the selected question/project.

Verification: EC2 manager asks the Windows implementer's project, Windows approves the second of two questions, and the real answer returns to the correct EC2 conversation. Also verify denial and missing-assistant behavior without granting access implicitly.

### 10. Complete conversational `/message`

- [ ] Wire `/message @teammate <text>` to the verified DM service, showing sender, recipient and truthful state immediately.
- [ ] Display the incoming message and its actual human reply in the appropriate conversation, distinct from assistant-generated output.
- [ ] Let the recipient reply inline with a visible target; keep task-related message context when present without sending the text to Codex automatically.
- [ ] Verify offline/restart recovery and third-member isolation through the rendered flow as well as the backend.

Verification: manager sends ?what is the status of the task??; the teammate replies; both see the same thread and another teammate cannot read it.

### 11. Complete conversational `/broadcast`

- [ ] Wire the command to real signed team broadcasts and render incoming broadcasts in the conversation with author and team scope.
- [ ] Distinguish saved/queued, synchronized and recipient acknowledgement where supported; never report relay acceptance as proof a human read it.
- [ ] Preserve message drafts on failure and display reconnect/delivery updates without blocking the composer.

Verification: `/broadcast checkout this source` reaches the teammate over the Internet and remains visible after restart, with no duplicate item after another sync.

### 12. Complete conversational `/task`

- [ ] Assign a real signed task using a mention and description; show the assignee, task text and current state in context.
- [ ] Provide visible inline Accept/Start/Done/Need help/Follow up actions appropriate to the actor and current task state.
- [ ] Reflect actual teammate updates and follow-ups in the sender's conversation; correlate actions to stable task IDs and preserve history.
- [ ] Reuse existing task authorization/convergence and show failures honestly; no fake assignment/completion responses.

Verification: manager assigns, teammate starts/completes/follows up, and manager sees the synchronized result without leaving `/crewai`.

### 13. Integrate asynchronous arrivals and focused navigation

- [ ] Surface incoming questions, approvals, messages and task updates while another command is being composed, without overwriting drafts or stealing focus.
- [ ] Make keyboard and mouse invoke the same actions. Use visible focused selection and consistent review/confirm behavior for multiple requests.
- [ ] Keep scrollback readable when new events arrive, and retain a clear way back to the latest activity.
- [ ] Test reconnect and team switching with pending work so late results stay associated with their original team/question/thread.

Verification: replay the user's completed-first-question/pending-second-question scenario and approve the second using ordinary selection; no Requests/Access/Broadcast tab switching is needed.

### 14. Run end-to-end user acceptance

- [ ] Use two isolated profiles and a third unauthorized message reader for automated domain/protocol checks. Run relevant tests after each change and the full suite at phase completion, recording known unrelated failures/stalls accurately.
- [ ] Have independent QA/review exercise the complete new journey against the saved images and actual code. Fix verified blockers before calling the feature complete.
- [ ] Test Linux EC2 manager and Windows implementer: `/crewai`, onboarding/invite, project connection, two `/read` questions with inline approval, message/reply, broadcast, task lifecycle, restart and reconnect.
- [ ] Verify installation and normal use need no developer explanation, external setup terminal, IDs/tokens/files or dashboard detours. Document exact prerequisites and limitations that remain.

Verification: save concise evidence for each flow. A mock assistant proves process plumbing only; separately verify a real signed-in Codex answer. Do not claim a physical Windows/EC2 run from local-only tests.

### 15. Package and ship the complete prototype

- [ ] Update README/install instructions to the actual conversation flow and five commands, including literal `/crewai` launch, setup and upgrade behavior.
- [ ] Build Windows/Linux release packages including the new entry/session integration. Preserve existing data and working Herdr behavior; no source build is required on the user's device.
- [ ] Publish authorized source/release updates, download both packages, verify checksums/version and smoke-test their actual entry and core flows.
- [ ] Record what is implemented, tested and pending; update task checkboxes only with evidence and confirm that the shipped UI matches the saved references.

Verification: the user can install on Windows and Linux, enter `/crewai`, and complete question/message/task/broadcast workflows using the published binaries.

## Definition of done

All five commands perform real operations. `/crewai` opens inside Herdr, `/` and `@` make the interface discoverable, answers and human replies appear in context, approvals work beside the correct question, and no normal flow depends on the old dashboard. The Windows/Linux manager-teammate run is verified; restart preserves data; private messages are recipient-only; trust and execution boundaries remain intact. No placeholder success messages, fabricated answers or UI-only skeleton qualify as completion.

## Completion notes

For every numbered task, append: what changed; the verification commands/manual steps and observed results; known limitations. If blocked, leave the item unchecked and record the exact missing prerequisite and next action. Current state: plan and references prepared only; all implementation items remain pending.
# Release snapshot (v0.2.2)

The conversational workspace, guided team setup, tasks, broadcasts, direct
messages, and project-question flows are being packaged for prototype testing.
Relay type checking and its 12 tests passed. The Windows-to-EC2 user journey
and Linux terminal integration still require testing on the actual devices;
the unchecked items below remain open. Codex execution is best-effort and is
not protected by an operating-system read-only boundary.

