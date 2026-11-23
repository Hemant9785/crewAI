# crewAI tasks (standalone app)

Ordered by dependency. Decisions and assumptions: [conversation_context.md](conversation_context.md). The older `task.md` belongs to the Herdr-based prototype and is kept unchanged.

- [x] **1. Inspect and preserve context.** Inspected folder, instructions and references; saved decisions, reference images and this checklist.
- [x] **2. Review and preserve Herdr work.** Legacy `herdr/`, `relay/` and docs stay untouched; new app lives in `app/`; short legacy note added to the README.
- [x] **3. Scaffold app and core.** TypeScript + Ink + vitest project in `app/`; domain models, command parser, `Backend` interface with in-memory implementation, local config/history store, agent detection and runner for Codex and Claude.
- [x] **4. Firebase setup.** Firebase backend (device account auth, presence heartbeat, invites, tasks, messages, requests), Firestore rules and indexes, `firebase.json` with emulators, `.env` template and `--check`.
- [x] **5. Main application flow.** Chat UI: onboarding, resume picker, presence header, feed, composer with `/` menu and `@` picker, `/read` (local and teammate, offline reroute), consent card, `/task`, `/broadcast`, `/tasks` board, `/team`, `/agent`, plain-text local agent.
- [x] **6. Supporting features and polish.** Saved history and resume, `--demo` mode, README with Firebase setup, rules deploy script, error messages.
- [x] **7. Milestone verification.** Typecheck, unit/engine/UI tests, Firestore rules contract tests on the emulator, build, manual run on a real terminal.

## Completion notes (2026-11-23)

**What changed**
- 3: `app/` project with `commands.ts`, `backend.ts`, `memoryBackend.ts`, `config.ts`, `agents.ts`.
- 4: `firebaseBackend.ts`, `firestore.rules`, `firebase.json`. Firebase-tools is pinned to v13 because Java 17 is installed (v15 needs Java 21).
- 5: `engine.ts` (all behavior, no UI) and `ui/*` (Ink screens).
- 6: `app/README.md`, `crewai --demo`, `crewai --check`, `npm run deploy:rules`. Deviation: the handwritten sketch's side panels are replaced by the `/tasks` board plus the single feed.

**Verification** (from `app/`)
- `npx tsc --noEmit` clean. `npm test`: 52 tests pass. `npm run emulators`: 12 contract tests pass against the real rules. `npm run build` OK.
- Manual: `node dist/cli.js --demo` on a real Windows terminal reached the onboarding screen; later screens were checked through rendered test frames only.

**Follow-up (same day)**
- Real Codex answered one question through the runner (about 22 s, `codex exec --sandbox read-only`). Claude is not installed on this machine, so its command is untested.
- Unsent messages (unknown teammate, no agent, failed send) are put back in the message box. 54 tests pass.
- The packed tarball installs into a clean folder and `crewai --version` / `--help` run. Windows now spawns agents without Node's shell-args warning.

**Real Firebase (2026-11-23)**
- Project `crewai-team-chat` (asia-south1) is live: Firestore database, deployed rules, Email/Password sign-in. `crewai --check` connects, and all 12 contract tests pass against it (`npm run test:live` in `app/`). Test users and data were deleted afterwards.

**Binaries (2026-11-23)**
- `npm run build:binary` (in `app/`) bundles the app with esbuild into a Node single-executable file. A launcher unpacks the bundle into `~/.crewai/bin` because Node 24 can only start CommonJS from a single executable and Ink needs an ES module.
- Windows x64 binary built and tested: `--version`, `--help`, `--check` (real Firebase, no local config) and the chat screen on a real terminal. The public Firebase config is embedded at build time.
- `.github/workflows/crewai-release.yml` builds linux x64/arm64, macOS Intel/arm64 and Windows on their own runners and publishes them on a `crewai-v*` tag. **Not run yet**, so the macOS and Linux binaries are untested; macOS Intel uses the `macos-13` runner label, which may need changing. Binaries are not code-signed.

**One-line installers (2026-11-23)**
- `tools/install-crewai.ps1` and `tools/install-crewai.sh` find the newest `crewai-v*` release, verify the SHA-256 checksum, and install without admin rights. Tested against a local fake release server: Windows (good download installed and ran; tampered download refused) and the shell script with a stub binary (same, plus no duplicate PATH lines, unsupported-OS error). Not tested against a real GitHub release or on real macOS/Linux. They only work after this is pushed to `main` and a `crewai-v*` release exists.

**Split-view layout (B), 2026-11-23**
- Rebuilt the main screen from a printed log into a fixed full-screen layout: Team, Tasks (grouped, filtered Mine/All/Open, finished tasks collapsed, "more" markers, peek of the selected task) and a scrollable Conversation with a time column, colored speakers and day markers; a full task view with the wrapped description, people, timeline and actions; a one-line strip below 110 columns and one column below 80. New files: `ui/Panels.tsx`, `ui/TaskView.tsx`, `ui/feed.ts`, `ui/tasklist.ts`, `ui/textlayout.ts`, `ui/useTerminal.ts`, `testing/uiWorld.tsx`.
- Verification: typecheck clean, 108 tests (unit tests for wrapping, task grouping/windowing and feed lines; screen tests at 140/100/70 columns with 20 and 30 tasks, focus, the task view, approvals, scrolling). Also run on a real Windows terminal (120 columns): panels aligned, live broadcast and task arrived while open.
- Not verified: macOS/Linux terminals, very small heights (under about 15 rows), resizing a live window by hand, terminals without alternate-screen support. Mouse wheel is not handled.

**Flicker fix (2026-11-23)**
- Typing made the whole screen flicker: Ink's default renderer erases and redraws every line on each frame, and the terminal here does not honor synchronized output. Added `ui/frameStream.ts`, which keeps the previous frame and writes only changed lines at absolute positions (about 25 times fewer bytes per keystroke, about 4 times fewer over a whole session). Ink's own incremental mode was tried and rejected because it corrupted the screen.
- Verification: unit tests for the writer, and a replay test that records the real app's output (typing, task view, scrolling, a resize) and checks a terminal emulator shows the identical screen after every frame. 119 tests pass. Not yet confirmed by eye in the owner's terminal.

**Known limitations**
- Not tested: real Codex/Claude answers through the full flow, macOS and Linux, long pasted text.
- Not built: Google/Microsoft sign-in, admin controls, audit log, the team permission graph.
- Invite codes can be brute-forced by automated clients; add App Check or a rate-limited function before launch.
- Remote answers are not OS-sandboxed; the approval card says "Limited protection".
- Firebase's operator can read questions, answers, tasks and messages.
