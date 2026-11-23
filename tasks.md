# crewAI tasks (standalone app)

Ordered by dependency. Decisions and assumptions: [conversation_context.md](conversation_context.md). The older `task.md` belongs to the Herdr-based prototype and is kept unchanged.

- [x] **1. Inspect and preserve context.** Inspected folder, instructions and references; saved decisions, reference images and this checklist.
- [x] **2. Review and preserve Herdr work.** Legacy `herdr/`, `relay/` and docs stay untouched; new app lives in `app/`; short legacy note added to the README.
- [x] **3. Scaffold app and core.** TypeScript + Ink + vitest project in `app/`; domain models, command parser, `Backend` interface with in-memory implementation, local config/history store, agent detection and runner for Codex and Claude.
- [x] **4. Firebase setup.** Firebase backend (device account auth, presence heartbeat, invites, tasks, messages, requests), Firestore rules and indexes, `firebase.json` with emulators, `.env` template and `--check`.
- [x] **5. Main application flow.** Chat UI: onboarding, resume picker, presence header, feed, composer with `/` menu and `@` picker, `/read` (local and teammate, offline reroute), consent card, `/task`, `/broadcast`, `/tasks` board, `/team`, `/agent`, plain-text local agent.
- [x] **6. Supporting features and polish.** Saved history and resume, `--demo` mode, README with Firebase setup, rules deploy script, error messages.
- [x] **7. Milestone verification.** Typecheck, unit/engine/UI tests, Firestore rules contract tests on the emulator, build, manual run on a real terminal.
- [x] **8. Ship the updater and fixes.** (Shipped as 0.3.4: 0.3.2 and 0.3.3 built on most platforms but failed CI on a screen-test race, fixed by deciding Tab inside the message box.) A real 0.3.3 binary updated itself to the published 0.3.4 with checksum verification, then reported up to date. Original scope: Test the self-updater, verify Windows replacement, document `--update` and `doctor`, bump the app version, push `main`, tag `crewai-v0.3.2`, and verify all five release assets plus a real 0.3.1-to-0.3.2 update.

Release progress (2026-11-23): Added updater coverage for rate-limit feed fallback, up-to-date and dry-run behavior, checksum rejection, and version validation before replacement. Verification passed: `npm.cmd exec -- vitest run` (124 tests), `npm.cmd exec -- tsc --noEmit`, and `npm.cmd run build`. A disposable Windows 0.3.1 executable updated itself from local 0.3.2 release assets, verified the checksum/version, replaced the running image, reported 0.3.2, and removed `.old` on the next start. README documents update and doctor; package and lockfile are 0.3.2. The changes are in local commit `e8f7ddf`.

Blocked release actions: `git push origin main` was rejected by automatic approval review because it would publish the repository's private contents and mutate shared `main`; the reviewer requires explicit authorization naming the destination and payload. No push or release tag was made. Next action: obtain approval to push the local release commits to `https://github.com/Hemant9785/crewAI` on `main`, then push `crewai-v0.3.2`, monitor all five builds, and test an installed 0.3.1 binary against the real release. The visual flicker fix also remains unconfirmed by eye in the owner's terminal; the automated terminal replay test passed earlier.

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

## Task discussions and pull request reviews (2026-11-23, in progress)

Owner request: the task area was too static (start/done only). Wanted: ask a follow-up about a task that only the person asked can see, and paste a pull request link with comments so chosen teammates can review it, built to market-ready quality.

- [x] **9. Data layer.** Private `threads` (follow-up or review) with `messages`, participants-only access in Firestore rules, an index for the thread list, a PR link validator (`pr.ts`, https only), in-memory and Firebase backends. 11 shared contract tests run against both the in-memory service and the real rules on the emulator (27 emulator tests pass).
- [x] **10. Engine and commands.** `/followup`, `/review` (`@team` supported), `/threads`; notifications in the chat feed for new questions, replies, approvals and change requests; per-conversation unread tracking; helpers for the screens. 10 engine and parser tests.
- [x] **11. Screens.** Threads view (list, messages, reply box, review verdict picker, open-link and close keys), unread badge in the header, "ask a follow-up" and a follow-up count in the task view. 6 screen tests. 150 tests pass.
- [ ] **12. Deploy and release.** The new rules and index must be deployed to the Firebase project before a release (`npx firebase deploy --only firestore`), then release as 0.4.0. Not done: not seen yet in a real terminal, no macOS/Linux check, and `ctrl+o` (open in browser) is covered only by a unit test of the link check.
- Not built: attaching more reviewers to an existing review, editing or deleting messages, notifications while crewai is closed, fetching PR details from GitHub (links are never opened or fetched by crewai itself).
