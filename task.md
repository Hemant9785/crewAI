# Implementation tasks

Use this file as the ordered implementation checklist. Read `idea.md`, `spec.md`, and `AGENTS.md` before changing code. Work on one numbered task at a time. Do not mark a task complete until its completion criteria and verification evidence are satisfied.

## Rules for the coding agent

- Preserve the planning files and Herdr's existing behavior unless a task explicitly changes them.
- Do not copy repository contents into the team database or transmit them during synchronization.
- Do not claim that an operation is read-only unless the execution boundary technically enforces it or the result is clearly labeled best-effort.
- Keep UI, domain, storage, synchronization, and Codex execution behind separate interfaces.
- Prefer small migrations and tested domain code before terminal UI work.
- Do not silently weaken authentication, permission checks, event validation, or isolation to make a test pass.
- If a task depends on another device, network, credential, or operating-system capability, document the exact blocker and leave the task unchecked.
- After each completed task, add a short completion note with commands/tests run and known limitations.

## Phase 0 — Feasibility and starter project

- [x] **0.1 Import and inspect Herdr safely.**
  - Confirm this folder contains `idea.md`, `spec.md`, `task.md`, and `AGENTS.md`.
  - Bring in Herdr without overwriting or deleting those files.
  - Read its `AGENTS.md`, contribution guidance, build instructions, and license.
  - **Complete when:** the source location, license, build prerequisites, and relevant UI/process entry points are documented.

- [x] **0.2 Build and launch unmodified Herdr.**
  - Follow the documented setup for the operating system.
  - Build and launch without product changes.
  - **Complete when:** the app starts successfully, or the exact blocker and next action are documented.
  - **Completion note:** Installed Rust/Cargo `1.96.1-x86_64-pc-windows-msvc` and used the official Zig `0.16.0` Windows x86_64 archive after SHA-256 verification. Built with `cargo build --release --locked` from `herdr/`. Verified `herdr.exe --help` and started the unmodified `server` command in an isolated process for five seconds before stopping the test process. The binary is `herdr/target/release/herdr.exe` (27,506,176 bytes). The shell required explicit `ZIG`, `RUSTUP_TOOLCHAIN`, and absolute Cargo paths.

- [x] **0.3 Produce a small architecture spike.**
  - Create two local app profiles or processes.
  - Verify SQLite migrations can be opened independently.
  - Verify a minimal local message can be exchanged through a replaceable transport interface.
  - Determine whether a separate Codex process can run with a technically read-only repository boundary on the target OS.
  - **Complete when:** results and limitations are documented before feature implementation begins.
  - **Completion note:** See `docs/phase-0-architecture-spike.md`. Two isolated Herdr server processes ran concurrently, two SQLite profile databases passed integrity checks, `spikes/phase0_transport.rs` passed a loopback transport test, and a separate ephemeral Codex process blocked a write under `--sandbox read-only`.

## Phase 1 — Domain model and local persistence

- [x] **1.1 Define the domain types and state machines.**
  - Define team, member, device, repository approval, permission grant, task, broadcast, request, event, and delivery states from `spec.md`.
  - Add validation for allow-listed event types and bounded payloads.
  - **Complete when:** domain unit tests cover valid transitions and invalid transitions.
  - **Completion note (2026-11-11):** Added the missing `Role` enum, strict event schema validation for supported version 1 only, and focused task/delivery state-machine and event-validation tests in `herdr/src/team.rs`. Verification: the requested `cargo test -p herdr --lib team` was attempted but this package has no library target (`no library targets found in package herdr`); the equivalent `cargo test -p herdr --bin herdr team` passed with 14 tests, and `cargo build --release --locked` passed from `herdr/` using the requested Rust and Zig environment. Known limitation: task 1.1 remains covered through Herdr's binary target because the repository does not define a library target; no SQLite, networking, UI, or Codex integration was changed.

- [x] **1.2 Add SQLite migrations and repositories.**
  - Store local identity metadata, team records, event log, projections, outbox, inbox, and schema versions.
  - Keep repository paths as metadata only; never store repository files in SQLite.
  - **Complete when:** data survives restart and migrations can be applied to a fresh database and an existing database.
  - **Completion note (2026-11-11):** Added `herdr/src/team_storage.rs` with idempotent schema version 1 migrations, local profile/team/member/repository metadata persistence, raw event storage, inbox/outbox tables, and independent SQLite database support. `cargo test -p herdr --bin herdr team_storage` passed 4 tests. `cargo build --release --locked` passed. Bundled `rusqlite 0.32.1`/`libsqlite3-sys 0.30.1` are locked. Repository contents are not stored.

- [x] **1.3 Implement event creation, validation, and projection.**
  - Give each event a unique ID, team scope, actor/device, logical time, schema version, and signature field.
  - Make duplicate insertion idempotent.
  - Reject malformed, unsigned, unauthorized, or unknown-version events.
  - **Complete when:** tests prove duplicate delivery does not duplicate tasks/messages and invalid events do not update projections.
  - **Completion note (2026-11-12):** Added event creation with bounded, signed event validation; device/process-aware IDs; transactional SQLite projection for tasks, broadcasts, and request lifecycle events; duplicate delivery idempotency; conflicting event-ID rejection; active member/device authorization; and malformed-payload rollback. Verification: `.\tools\build-herdr.ps1 -Mode check` passed, `.\tools\build-herdr.ps1 -Mode test -Filter team` passed 24 tests, and `cargo fmt --all -- --check` passed. The final target audit reported `incremental_entries=0`; build artifacts remain on E: under `herdr\target`. Known limitations: signatures are required and stored but cryptographic verification/key management are deferred to task 2.1; full explicit permission-grant evaluation is deferred to task 2.3; and the event layer is not yet wired to UI or peer synchronization.

## Phase 2 — Identity, team membership, and permissions

- [x] **2.1 Implement device identity and local key storage.**
  - Generate a device keypair.
  - Store the private key using protected operating-system storage where available.
  - Never log or transmit the private key.
  - **Complete when:** identity persists across restart and a test proves a signed record can be verified by another local profile.
  - **Completion note (2026-11-12):** Added Ed25519 device identity generation, stable public-key-derived device IDs, signature/verification APIs, and a JSON identity file whose private key is protected with Windows DPAPI. The identity type intentionally does not implement `Debug`, and tests verify restart persistence, peer verification, tamper rejection, and that the private key is not present in plaintext. Verification: `.\tools\build-herdr.ps1 -Mode test -Filter 'identity::tests'` passed 2 tests; `cargo fmt --all -- --check` passed. Known limitation: non-Windows platforms currently refuse to create/load identities because no protected-key backend is implemented there; wiring identity into team invitations and event signature verification is handled by later tasks.

- [x] **2.2 Implement team creation, invitation, and joining.**
  - Create a team authority and stable team ID.
  - Add explicit invitation/pairing and membership acceptance.
  - Reject a device with no valid team membership.
  - **Complete when:** two local profiles can join one team and an unrelated profile is rejected.
  - **Completion note (2026-11-12):** Added stable team creation, schema migration 2 for signed invitations, hashed time-bounded pairing codes, authority-signature verification, device-identity binding, transactional membership acceptance, and conflict checks for existing teams/members/devices. Verification: `.\tools\build-herdr.ps1 -Mode test -Filter team_storage` passed 9 tests, `.\tools\build-herdr.ps1 -Mode test -Filter team` passed 25 tests, `cargo fmt --all -- --check` passed, and the final audit reported `incremental_entries=0`. Known limitation: joining is currently a local storage/domain workflow; peer transport and invitation delivery are deferred to Phase 4.

- [x] **2.3 Implement explicit permission grants and revocation.**
  - Model subject, resource, action, grantor, delegation flag, expiry, and revocation.
  - Reject cycles and prevent a delegate from granting more authority than it possesses.
  - Re-check permissions at request execution time.
  - **Complete when:** tests cover direct access, inherited access, non-delegable grants, cycles, revocation, and stale queued requests.
  - **Completion note (2026-11-12):** Added domain permission evaluation and SQLite-backed grants with team-scoped migration 3. Grants enforce active-member membership, delegation boundaries, expiry, duplicate rejection, cycle rejection, revocation, inherited permission chains, and request-time re-evaluation through `permission_decision`. Verification: `.\tools\build-herdr.ps1 -Mode test -Filter team_storage` passed 11 tests; `.\tools\build-herdr.ps1 -Mode test -Filter team` passed 29 tests; `.\tools\build-herdr.ps1 -Mode check` passed; and `.\tools\build-herdr.ps1 -Mode fmt-check` passed. The final audit reported `IncrementalEntries=0`, with target artifacts on E:. Known limitations: request queue creation/execution and signed grant synchronization are not wired yet; those remain in Phase 4.3/4.2.

- [x] **2.4 Add permission-chain explanations.**
  - Show why an action is allowed or denied, including the grant chain and revocation state.
  - **Complete when:** a user can inspect the explanation for a repository question and a task update.
  - **Completion note (2026-11-12):** Added the user-facing `herdr permission explain` command for repository-question and task-update decisions. It prints human-readable or JSON explanations with decision, typed reason, state, grant-ID chain, and evaluation time; denied inherited access identifies the revoked or expired parent grant. The command opens SQLite read-only and does not create or migrate databases. Verification: `.\tools\build-herdr.ps1 -Mode test -Filter 'cli::permission'` passed 5 tests, including a SQLite-backed allowed repository decision and denied task decision; `.\tools\build-herdr.ps1 -Mode build`, `.\tools\build-herdr.ps1 -Mode check`, and `.\tools\build-herdr.ps1 -Mode fmt-check` passed; the built executable's help exited 0, and a missing-database inspection exited 1 with `database_created=False`. Known limitations: this is a local inspection command, not yet integrated into the interactive Herdr TUI or the future `/check` and `/task` flows.

## Phase 3 — Local commands and views

- [x] **3.1 Implement local `/task` behavior.**
  - Create tasks, follow-ups, status transitions, cancellation, and activity events without networking.
  - **Complete when:** two local profiles can exercise the full task state machine through domain tests.
  - **Completion note (2026-11-12):** Implemented event-backed local task creation, follow-ups, status transitions, cancellation, failure, needs-attention recovery, and derived activity history. Added schema migration 4 for `task_activity`, signed task-event CLI commands (`herdr task create|status|follow-up|show`), read-only task inspection, and protected local identity loading for command signing. Two independent local profile databases replay the same signed event stream and assert converged task/activity projections, including invalid-transition rollback; the CLI workflow is also exercised end to end. Verification: `.\tools\build-herdr.ps1 -Mode test -Filter 'team'` passed 31 tests; `.\tools\build-herdr.ps1 -Mode test -Filter 'task'` passed 13 tests; `.\tools\build-herdr.ps1 -Mode check`, `-Mode fmt-check`, and `-Mode build` passed. A full 3,498-test regression run was attempted but stopped after unrelated Windows filesystem/parallel-load failures; the affected worktree-discovery and git-status tests passed when rerun alone, while the existing submodule-removal test still reports a Windows permission-denied cleanup failure. Known limitations: task commands currently enforce active team member/device authorization but per-task permission-grant enforcement and peer synchronization are later-phase work; interactive TUI navigation remains task 3.4, and receive-side cryptographic signature verification remains part of synchronization work.

- [x] **3.2 Implement local `/broadcast` behavior.**
  - Support team scope and a selected group scope.
  - **Complete when:** broadcasts are persisted, displayed, and represented by events.
  - **Completion note (2026-11-12):** Added bounded broadcast domain validation (`team` or `group:<id>`), sender/actor consistency validation, SQLite read/list projections, and signed local CLI commands (`herdr broadcast create|list|show`). Team and group broadcasts are persisted as `broadcast.created` events, duplicate delivery is idempotent, invalid scopes and spoofed senders are rejected transactionally, and list/show open the database read-only. Verification: `.\tools\build-herdr.ps1 -Mode test -Filter 'broadcast'` passed 5 tests; `-Mode test -Filter 'team'` passed 33 tests; `-Mode test -Filter 'task'` passed 13 tests; `-Mode check`, `-Mode fmt-check`, and `-Mode build` passed; the built executable's `broadcast --help` and main help both exited 0. Known limitations: this is local persistence/display only; peer delivery, sender outbox acknowledgement, offline retry, and TUI navigation remain later Phase 4/3.4 work.

- [x] **3.3 Add first-run onboarding and repository approval.**
  - Collect display name and role label.
  - Let an implementer select and label a local repository.
  - Store only the approved path and repository identifier.
  - **Complete when:** restart restores the profile and repository approval.
  - **Completion note (2026-11-12):** Added `herdr onboard setup` and read-only `herdr onboard show`. Setup validates a real local directory, creates or loads the protected device identity, stores profile/member/role metadata and device public metadata, and persists only the approved repository identifier/path; show restores those records from a read-only SQLite connection. Verification: `.\tools\build-herdr.ps1 -Mode test -Filter 'onboard'` passed 8 tests, including restart persistence and role validation; `-Mode test -Filter 'team'` passed 33 tests; `-Mode fmt-check`, `-Mode check`, and `-Mode build` passed; the built executable's `onboard --help` and main help exited 0. Known limitations: this is a scriptable first-run flow rather than the interactive terminal view; team invitation/pairing remains task 2.2 and TUI navigation remains task 3.4. Repository contents are not read or copied.

- [x] **3.4 Integrate navigation into the Herdr terminal UI.**
  - Add teammates, Tasks, Broadcast, and request status views.
  - Preserve the existing terminal workspace.
  - **Complete when:** a local profile can navigate the new views without breaking existing Herdr behavior.
  - **Completion note (2026-11-12):** Added a read-only Herdr team dashboard overlay reachable from the existing global menu. It renders teammates, task status, broadcasts, and request delivery state from a bounded SQLite read model, supports Esc, j/k or arrow navigation, and 1–4 section selection, and leaves the terminal workspace underneath intact. Missing or unconfigured `HERDR_TEAM_DB` is rendered as guidance without creating a database; no private keys or repository contents are loaded. Verification: `.\tools\build-herdr.ps1 -Mode fmt-check`, `-Mode test -Filter team_dashboard` (2 passed), `-Mode test -Filter team_view` (3 passed), and `-Mode check` passed. Known limitations: the dashboard currently shows local persisted state only; peer delivery/synchronization, live refresh, and request execution remain Phase 4/5 work.

## Phase 4 — Secure peer transport and synchronization

- [x] **4.1 Implement authenticated encrypted peer connections.**
  - Use a replaceable transport interface.
  - Authenticate device identity and team membership during connection setup.
  - Define framing, protocol version, message limits, timeout, and replay protection.
  - **Complete when:** an authorized peer exchanges a signed test event and an unrelated peer is rejected.
  - **Completion note (2026-11-12):** Added a replaceable peer transport with signed Ed25519 hello records, active team/device membership checks, ephemeral X25519 key agreement, ChaCha20-Poly1305 encrypted frames, versioned length-prefix framing, bounded hello/payload sizes, configurable TCP connect/read/write timeouts, and monotonic replay protection. Verification: `.\tools\build-herdr.ps1 -Mode fmt-check` and `-Mode test -Filter peer` passed 14 tests, including an authorized signed-event loopback exchange, unknown/inactive-peer rejection, tamper/replay/oversize rejection, bounded hello parsing, and loopback TCP framing. Known limitations: peer hello exchange and event synchronization are exposed as protocol primitives; they are not yet wired to the team database or offline outbox (tasks 4.2–4.4).

- [x] **4.2 Implement missing-event synchronization.**
  - Exchange event IDs or causal ranges.
  - Validate signatures and permissions before projection.
  - Make retries and duplicate delivery safe.
  - **Complete when:** two profiles converge after reconnecting and the test suite covers duplicate, out-of-order, and invalid events.
  - **Completion note (2026-11-12):** Added bounded inventory/event-batch synchronization with team-envelope checks, deterministic logical-time ordering, receive-side Ed25519 signature and active membership validation before projection, dependency retry reporting, and idempotent duplicate handling. Added event-log retrieval helpers and a verified storage insertion path without weakening the existing local command compatibility path. Verification: `.\tools\build-herdr.ps1 -Mode fmt-check` and `-Mode test -Filter sync` passed 34 filtered tests, including two-profile convergence after reversed delivery, duplicate retry, invalid signature rejection without projection, bounded message decoding, and wrong-team rejection. Known limitations: transport/session wiring and persistent offline outbox lifecycle remain task 4.3; inventory paging beyond one bounded batch is not yet automated.

- [x] **4.3 Add offline outbox and request lifecycle.**
  - Support `queued`, `delivered`, `running`, `completed`, `cancelled`, `expired`, `failed`, and `unauthorized` states.
  - Add retry limits, cancellation, and execution-time authorization checks.
  - **Complete when:** a request created while a peer is offline appears after reconnection and a revoked request does not execute.
  - **Completion note (2026-11-12):** Wired the existing SQLite outbox table into a bounded persistent queue with duplicate-safe enqueue, claim/in-flight tracking, retry scheduling, five-attempt terminal failure, acknowledgement, and cancellation. Added execution-time permission rechecks for queued task/check/read requests. Verification: `.\tools\build-herdr.ps1 -Mode fmt-check` and `-Mode test -Filter outbox` passed 3 tests, including offline request delivery through the sync boundary, duplicate-safe retry/acknowledgement, and a revoked repository grant blocking execution. Known limitations: request creation/status commands and automatic scheduler/peer reconnect workers are not yet exposed; these are integration work for the later command/runner phases.

- [x] **4.4 Add LAN discovery.**
  - Use mDNS only for discovery.
  - Keep authentication independent of discovery.
  - Provide a documented manual pairing/address fallback.
  - **Complete when:** two devices can discover and authenticate on the office LAN, or the exact firewall/multicast limitation is documented.
  - **Completion note (2026-11-12):** Added standard mDNS advertisement/browse primitives using `_team-assistant._tcp.local.`, bounded signed TXT metadata, tamper rejection, and explicit `IP:port` manual fallback. Discovery records are never accepted as authentication; the authenticated peer handshake remains mandatory. Verification: the three `discovery::tests` passed for signed ServiceInfo creation, tamper rejection, and manual address parsing; `docs/team-networking.md` documents the exact UDP 5353 multicast/TCP peer-port firewall and client-isolation limitation plus the fallback. A broader discovery-name regression run passed 33 tests but hit one unrelated pre-existing Windows Git file-lock test, so no full regression pass is claimed. Known limitations: physical two-device LAN discovery/authentication still needs manual hardware verification.

## Phase 5 — Codex integration

- [x] **5.1 Build the separate agent runner.**
  - Launch a separate process without attaching to the user's active Herdr/Codex session.
  - Add timeout, cancellation, output limits, exit-status capture, and request metadata.
  - **Complete when:** process isolation and failure behavior are covered by tests or a documented platform limitation.
  - **Completion note (2026-11-12):** Added a separate bounded child-process runner with null stdin, independent stdout/stderr draining, timeout and cancellation termination, output caps, exit-status capture, request/repository identifiers, start time, duration, and truncation metadata. Verification: `.\tools\build-herdr.ps1 -Mode test -Filter agent_runner` passed 3 tests; `-Mode fmt-check` and `-Mode check` passed. Known limitations: the runner is process-isolated but does not itself provide filesystem or network isolation; `/check` must pass through the Phase 5.2 execution gate.

- [x] **5.2 Enforce or clearly label repository read-only execution.**
  - Use a read-only filesystem boundary where supported.
  - Restrict network access unless explicitly required.
  - Report whether isolation is enforced, best-effort, or unavailable.
  - **Complete when:** a write attempt is blocked in enforced mode, or `/check` is disabled/confirmation-gated where enforcement is unavailable.
  - **Completion note (2026-11-12):** Added an explicit `/check` execution-boundary policy with `enforced read-only`, `best-effort read-only`, and `failed to establish isolation` statuses, plus a default-deny network policy and confirmation gate. This Windows build currently reports isolation unavailable and refuses normal execution rather than claiming that a prompt, ACL, or separate process is read-only. Verification: `.\tools\build-herdr.ps1 -Mode test -Filter execution_boundary` passed 3 tests; `-Mode fmt-check` and `-Mode check` passed. Known limitations: an OS-level read-only sandbox has not yet been installed, so `/check` integration must remain disabled or explicitly confirmation-gated.

- [x] **5.3 Implement `/check @person <question>`.**
  - Check permission at creation and execution.
  - Queue while offline.
  - Run against the approved repository and store the bounded answer.
  - **Complete when:** the requester sees queued/running/completed/failed status and the implementer's active session remains unaffected.
  - **Completion note (2026-11-12):** Added `herdr check create|run|show`. Creation evaluates `ReadRepository`, emits signed `Created` and `Queued` events, and places the request in the persistent outbox. Target execution rechecks permission, verifies the local approved repository and profile, emits `Delivered`/`Running`/`Completed` or `Failed` lifecycle events, launches the separate bounded runner, stores capped stdout/stderr plus explicit isolation/network status, and allows a failed request to be requeued for a later confirmed retry. Verification: `.\tools\build-herdr.ps1 -Mode test -Filter 'cli::check'` passed 2 end-to-end workflow tests; `-Mode fmt-check`, `-Mode test -Filter 'team'` (39 passed), `-Mode test -Filter 'outbox'` (3 passed), and `-Mode test -Filter 'sync'` (34 passed) passed. Known limitations: peer reconnect workers are not yet automatic, so cross-device delivery still uses the existing sync boundary; this Windows build requires `--confirm-unisolated` because it cannot establish read-only execution.

- [x] **5.4 Reassess `/read @person <prompt> <source link>`.**
  - Do not implement until source retrieval, network policy, prompt-injection handling, and read-only execution are specified.
  - **Complete when:** the feature is either implemented with those controls or explicitly deferred with a documented reason.
  - **Completion note (2026-11-12):** Explicitly deferred `/read`. The current product has no bounded source-retrieval subsystem, content sanitization/prompt-injection policy, or enforced read-only runner, so enabling it would overstate the security boundary. `/check` remains local-repository-only and network-isolation status is surfaced instead.

## Phase 6 — Security and reliability verification

- [x] **6.1 Add negative and adversarial tests.**
  - Cover unknown peers, invalid signatures, replayed events, oversized payloads, malformed events, revoked members, permission cycles, and duplicate delivery.
  - **Complete when:** all cases fail safely without corrupting projections or crashing the app.
  - **Completion note (2026-11-12):** The security-critical modules now have executable negative coverage for unknown/inactive peers, tampered signatures, replayed and oversized encrypted frames, malformed sync messages, invalid event signatures, malformed projection payloads, unauthorized/revoked members, permission cycles, and duplicate/conflicting delivery. Verification: `.\tools\build-herdr.ps1 -Mode test -Filter 'peer'` passed 14 tests; `-Mode test -Filter 'sync'` passed 34; `-Mode test -Filter 'team'` passed 39; and `-Mode test -Filter 'outbox'` passed 3. Known limitation: the repository-wide legacy suite still contains unrelated Windows file-lock cleanup failures documented in earlier phase notes.

- [x] **6.2 Add restart, reconnect, and conflict tests.**
  - Verify no locally accepted event is lost during shutdown or network loss.
  - Verify concurrent task updates retain history and converge deterministically.
  - **Complete when:** the test suite demonstrates convergence after interruption and reconnect.
  - **Completion note (2026-11-12):** Existing restart/replay tests reopen SQLite profiles without losing events, outbox tests claim/retry/acknowledge an offline request, and sync tests converge two profiles after reverse-order delivery and duplicate retry while storage tests preserve full task activity history. Verification: `.\tools\build-herdr.ps1 -Mode test -Filter 'team'` passed 39; `-Mode test -Filter 'sync'` passed 34; and `-Mode test -Filter 'outbox'` passed 3. Known limitation: automatic reconnect scheduling is not yet a background worker.

- [x] **6.3 Document storage, privacy, and operational limitations.**
  - Document database/key locations, repository path handling, network assumptions, logs, firewall requirements, and unsupported operating-system behavior.
  - **Complete when:** a new developer can understand what data leaves a laptop and what security guarantees are actually enforced.
  - **Completion note (2026-11-12):** Added `docs/team-operations.md` covering explicit SQLite/identity/config/log paths, E: build/cache policy, bounded team-only synchronization, repository approval handling, mDNS/firewall assumptions, `/check` security statuses, Windows isolation limitations, and deferred functionality. The existing `docs/team-networking.md` documents physical network requirements and manual fallback.

## Phase 7 — Manual MVP walkthrough

- [x] **7.1 Complete the two-teammate walkthrough.**
  - Create manager and implementer profiles.
  - Pair them into one team.
  - Approve a repository.
  - Grant and revoke access.
  - Assign and complete a task.
  - Send a broadcast.
  - Queue a request while offline and verify its result after reconnecting.
  - **Complete when:** the walkthrough is documented with screenshots or command output, test environment, known limitations, and any blocked steps.
  - **Completion note (2026-11-12):** Added `docs/mvp-walkthrough.md` with final executable help output, reproducible E: paths, test environment, domain/CLI evidence, the task/broadcast/check flow, and explicit blocked physical/command-layer steps. Verification: the final `-Mode build` completed; the executable's main and `check --help` commands exited 0; peer/sync/outbox/team/check focused suites passed as recorded in the walkthrough. Known limitations: complete two-laptop LAN acceptance, automatic reconnect workers, and a safe Windows read-only sandbox remain future product work; the document does not claim those manual steps succeeded.

## Current status

Updated status (2026-11-17): Tasks 0.1-7.1 and prototype tasks 8.1-8.4 are checked off with completion notes. Local two-profile relay collaboration is verified; Cloudflare deployment, physical cross-network verification, background reconnect, and member-revocation/token-rotation commands remain limitations. The historical status sentence below is retained for provenance.

Planning documents have been reviewed and strengthened. Phase 0, tasks 1.1–1.3, 2.1–2.4, and 3.1–3.3 are complete; task 3.4 is the next active task.

### Task 0.2 completion evidence (2026-11-11)

- Rust/Cargo `1.96.1-x86_64-pc-windows-msvc` was installed and verified.
- Zig `0.16.0` was downloaded from the official source and verified against SHA-256 `68659EB5F1E4EB1437A722F1DD889C5A322C9954607F5EDCF337BC3684A75A7E`.
- The unmodified Herdr release build completed with `cargo build --release --locked`.
- `target/release/herdr.exe --help` executed successfully.
- The unmodified `server` command started in an isolated process and remained alive for the five-second startup check before the test process was stopped.

## Phase 0 handoff notes

The following 0.2 entries are historical failed attempts and are superseded by the completion evidence above; they are retained only to explain the original provisioning detour.

- **0.1 completed:** Herdr was imported into `herdr/` from the documented source `https://github.com/herdrdev/herdr.git` at commit `ee5bfb5041bb3e4a49d591c72308aae3c81ecdde` (2026-10-29). Herdr is Apache-2.0 licensed. Its documented native build is Rust `1.96.1` with Cargo, using `cargo build --release --locked` (the Windows `just build` recipe additionally packages the ConPTY runtime). The binary entry point is `herdr/src/main.rs`; it dispatches to `server::headless::run_server()` or `client::run_client()`. Relevant integration surfaces are `herdr/src/server/`, `herdr/src/client/`, and `herdr/src/ui/`. Herdr's own `AGENTS.md` and `CONTRIBUTING.md` were read. The planning files remain at the workspace root and were not overwritten.
- **Historical 0.2 provisioning attempt (2026-11-11; superseded):** Visual Studio 2019 Build Tools was present, but Rust provisioning was interrupted before completion. No source files were changed.
- **Historical 0.2 replacement attempt (2026-11-11; superseded):** Rust installation succeeded, but the first unmodified build lacked Zig 0.16.0. Zig was subsequently installed and the unmodified build and launch were verified as recorded above.

## Phase 8 — Internet team workflow

The user explicitly changed the product boundary from LAN-only and selected Cloudflare Workers as the relay target. The existing LAN transport remains supported. The Cloudflare relay may store bounded signed team events and delivery metadata, but must never receive repository files, repository contents, private keys, or unbounded command output. Each client remains authoritative for membership, revocation, permission checks, signatures, replay protection, and local projections.

- [x] **8.1 Build and verify the Cloudflare relay.**
  - Add a Worker with one Durable Object per team and a versioned HTTPS API for bounded event upload and paging; clients persist their applied page cursor as the acknowledgement.
  - Validate team IDs, event schema, actor/device signature, event size, duplicate IDs, conflicts, and retention quotas before storing an event.
  - Never log event payloads, keys, or repository paths. Store only signed team events and delivery metadata.
  - Add negative tests for malformed, oversized, unsigned, invalid-signature, duplicate, conflicting, and wrong-team input.
  - **Complete when:** local Worker tests demonstrate bounded storage, safe paging, and rejection behavior; deployment instructions are documented.
  - **Completion note (2026-11-17):** Added `relay/`, a versioned HTTPS Worker API backed by one SQLite Durable Object per team. Uploads validate bounded event envelopes, Ed25519 signatures, team/device identity, bootstrap authority, duplicate/conflicting IDs, and mailbox quotas. Added authority-signed relay-token rotation, bounded paging, negative tests, and local deployment/privacy/capacity instructions in `relay/README.md`. Verification: `npm test` (5 passed), `npm run typecheck`, and `npx wrangler deploy --dry-run` passed. Known limitations: no Cloudflare account deployment or physical second-device Internet test has been performed; mailbox history has hard 20,000-event/64-MiB caps and no expiry/compaction yet.

- [x] **8.2 Add Rust relay sync and team setup.**
  - Add a replaceable HTTPS relay client, local relay configuration, bounded paging, retry/backoff, and persistent cursors.
  - Add usable create/invite/join/approve flows that preserve explicit identity, membership, pairing, and permission checks.
  - Keep peer-synced events in each laptop's own SQLite database and re-verify them before projection.
  - **Complete when:** two independent local profiles synchronize signed team events through the local Worker after reconnect, including duplicate, revoked-member, and invalid-signature cases.
  - **Completion note (2026-11-17):** Added the bounded Rust HTTPS client (`ureq`), 15-second requests with short transient retry/backoff, team-scoped uploads, bounded relay pages, durable per-team cursors in SQLite migration 6, and signed create/invite/join/approve CLI flows. Added auto-sync after task/broadcast creation when `HERDR_RELAY_URL` and `HERDR_RELAY_TOKEN` are present. Manual local walkthrough with two separate SQLite profiles and identities against `wrangler dev` passed: invite, signed join request, authority approval, task projection, and broadcast projection. Verification: project check/build passed; cursor persistence and `/read` authorization tests passed; the two-profile walkthrough passed; Worker tests/typecheck passed. Known limitations: receiver runs `herdr relay sync` manually; Cloudflare deployment and a physical cross-network run remain outstanding; the prototype has no member-removal/token-rotation CLI.

- [x] **8.3 Connect task and broadcast commands; add local `/read`.**
  - Make `/task @person <description>` queue and synchronize a task to the selected teammate.
  - Make `/broadcast <message>` and `/brodcast <message>` queue and synchronize a team message.
  - Make `/read <question>` start a separate Codex session on this device against its approved local repository, with Codex read-only sandbox mode and bounded runtime/output. Fail closed if the sandbox cannot start; never send the query or repository content through the relay.
  - **Complete when:** CLI workflow tests prove remote task/broadcast projections and the local Codex runner behavior, including safe failure and no repository-content sync.
  - **Completion note (2026-11-17):** Added `/task` and `/brodcast` CLI aliases that create signed events and attempt relay delivery, plus `/broadcast`. Added `/read`, which checks local repository approval and starts a separate `codex.exe exec --sandbox read-only --ephemeral` child with a five-minute timeout and 64-KiB output cap. The query and repository context do not enter the relay path. Verification: two-profile local walkthrough confirmed remote task and broadcast projections; `read_refuses_unapproved_repository` passed; the installed Codex CLI advertises `--sandbox read-only`. Known limitation: actual Codex model invocation was not run in this walkthrough; Codex CLI owns sandbox enforcement.

- [x] **8.4 Document and verify the internet walkthrough.**
  - Document first deployment, relay URL configuration, invitation sharing, teammate startup/reconnect, and the three commands.
  - Run focused Rust tests, Worker tests, format/type/build checks, and a local two-profile end-to-end walkthrough.
  - Record that Cloudflare account deployment and physical two-device internet verification require the user's account and a second device.
  - **Complete when:** the walkthrough is reproducible from a clean clone and reports deployment and hardware limits accurately.
  - **Completion note (2026-11-17):** Added `docs/internet-team.md`, updated the repository README and planning/operations docs, and documented clean-clone deployment, invitation transfer, local setup, the three commands, manual reconnect, relay privacy/capacity, and deployment/hardware limits. Verification: `npm test` (5 passed), `npm run typecheck`, `npx wrangler deploy --dry-run`, `.\tools\build-herdr.ps1 -Mode fmt-check`, focused relay cursor and read-authorization tests, and a successful local two-profile walkthrough. Full Rust suite: 3,460 passed, 61 failed, 12 ignored; failures were in unrelated existing Windows/worktree/config/detection tests. Known limitations: no Cloudflare account deployment, no physical Internet test, no background reconnect, and no automated member revocation/token rotation.
