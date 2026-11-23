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

**Known limitations**
- Not tested: a real Firebase project (needs the owner's project and web config), real Codex/Claude answers through the full flow, macOS and Linux, long pasted text.
- Not built: Google/Microsoft sign-in, admin controls, audit log, the team permission graph.
- Invite codes can be brute-forced by automated clients; add App Check or a rate-limited function before launch.
- Remote answers are not OS-sandboxed; the approval card says "Limited protection".
- Firebase's operator can read questions, answers, tasks and messages.
