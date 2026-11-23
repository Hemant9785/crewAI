# crewAI conversation context and decisions

Last updated: 2026-11-23. No secrets, keys or credentials are stored here.

## Product (from the owner)

- Type `crewai` in any terminal and a chat opens, like `codex` or `claude`.
- Everything happens in that chat: onboarding, `/read`, `/task`, `/broadcast`, plain text.
- Launch target: enterprise teams (so admin controls, audit and SSO matter later).
- Priority: short, good-looking UX. Value to team members comes first. Herdr's value was UI + agent config, not the multiplexer.
- Repository files never leave the owner's laptop; only questions, bounded answers, tasks and messages are shared.

## Decisions

| Topic | Decision |
|---|---|
| Entry | Standalone `crewai` command. Not inside Herdr, no shell `/crewai` hook. |
| Backend | Firebase (Auth + Firestore). Behind a swappable `Backend` interface. |
| Scope of answers | Answers always come from the folder where `crewai` was opened. |
| `/read @name q` | Asks that teammate's agent about their folder. No name = your own folder. |
| Plain text | Talks to your own local agent about your own folder. |
| Consent | Owner chooses Allow once, Always allow (per asker), or Decline. |
| Offline target | Question is saved. Asker may wait, ask someone online, or cancel. A question has one owner at a time; rerouting cancels the old copy. Returning user sees "already handled". |
| `/task @a @b text` | One separate task per assignee. Assignee marks started/done/help. Done also posts a broadcast. |
| `/broadcast` | Message to the whole team. |
| Agents | Codex and Claude Code, auto-detected on each laptop. Owner can switch; nothing breaks if the old one is gone. |
| Platforms | Windows, Linux, macOS. |
| Sign-in | Both wanted: Google and name + invite code. See assumptions. |
| History | Chats are saved per folder, with resume or start new (like `codex resume`). |
| Invite approval | A manager-issued 6-digit code (10 min, single use) is the approval. |

## Reference images (`reference_images/`)

- `handwritten-plan-dashboard.jpeg`: owner's sketch. Left Agents panel (agents the user works on), centre terminal, right Tasks (latest first, scrollable, check/cross) and Broadcast (latest first). Marking a task done broadcasts "implemented". `/task` then `@` lists members with multi-select. A team dashboard shows a graph of who can assign tasks to whom (manager at top; transitive; bidirectional an open question). Note "local @" is an unresolved idea.
- `crewai-conversation-reference.png`, `crewai-command-flow-reference.png`: earlier Herdr-based concept art (one conversation, `/` command menu, `@` teammate picker, inline owner approval with accurate isolation warning). Visual style still applies.
- `ui-snapshots.md`: the six text snapshots agreed in chat (onboarding, resume, main chat, offline teammate, consent, tasks).

## Technical assumptions (smallest reasonable choices)

1. Stack: TypeScript on Node 20+, Ink (React for terminals), Firebase JS SDK, vitest. Chosen over Rust because Firebase has an official JS SDK, Ink is the standard way to build chat TUIs, and Node runs on all three platforms. The earlier Rust proposal is superseded.
2. Identity: each device gets a stable Firebase email/password account with random generated credentials stored in `~/.crewai/identity.json` (owner-only permissions). This keeps the same uid across runs because the Node SDK does not persist anonymous sessions. Google/Microsoft sign-in is added later via account linking and needs an OAuth client ID from the owner.
3. Firebase project: not created by this work. Needs the owner's project and web config (see Prerequisites). Tests run against an in-memory backend and, where possible, the Firestore emulator.
4. Authorization is Firestore Security Rules plus membership documents (no per-event signatures). Roles: `manager` and `member`. Managers issue invites; anyone can assign tasks in the MVP. Grant graph is deferred; the team graph view shows managers and members only.
5. Presence is a heartbeat `lastSeen` timestamp; online means seen in the last 90 seconds.
6. Agent isolation is not OS-enforced. The consent card always says "Limited protection". Codex runs with its read-only sandbox flag and Claude with read-only tools, both labelled best-effort.
7. Legacy `herdr/`, `relay/`, `docs/`, `spec.md`, `task.md`, `idea.md` are kept unchanged. New app lives in `app/`. Nothing is deleted; removal is an owner decision.
8. `aws_guide.txt` and `website/` are untracked and out of scope; not read or copied.

## Prerequisites still needed from the owner

- A Firebase project with Email/Password sign-in enabled and a Firestore database, plus its web config values in `app/.env` (template provided).
- Later: Google OAuth client ID for Google sign-in; enterprise SSO choice.

## Status (2026-11-23)

- Milestones 1-7 done; see tasks.md notes. Side panels from the handwritten sketch were replaced by the /tasks board and one feed (smallest change for a scrollback chat UI). Name rule: no spaces, so @mentions are unambiguous.
