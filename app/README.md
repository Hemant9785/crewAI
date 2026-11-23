# crewai (standalone app)

Type `crewai` in a project folder and a chat opens. Ask your own coding agent about the folder, ask a teammate's agent about theirs, assign tasks and broadcast messages. Repository files never leave their owner's laptop; only questions, bounded answers, tasks and messages are shared.

## Try it now (no account)

```powershell
cd app
npm install
npm run build
node dist/cli.js --demo
```

Choose "Join a team" and enter the code it prints. Hemant, Shwetank and Priya are simulated; Ravi is away.

## Install

Most people should use the standalone binaries; see the install steps in the [main README](../README.md#install-crewai-standalone-app). To build one yourself: `npm ci && npm run build:binary` (bundles the app with esbuild into a Node single-executable file under `release/`; it embeds the Firebase settings from `CREWAI_FIREBASE_*` or `~/.crewai/.env`).

## Install from source as a command

```powershell
cd app
npm install
npm run build
npm pack
npm install -g .\crewai-0.1.0.tgz
crewai --version
```

Then run `crewai` in any project folder. (Publishing to npm is not set up; the package is marked private.)

## Commands in the chat

| Input | What it does |
|---|---|
| plain text | asks your own agent about this folder |
| `/read question` | same, explicit |
| `/read @name question` | asks that teammate's agent about their folder (they approve first) |
| `/task @a @b what to do` | one task per person; marking one done broadcasts it |
| `/broadcast message` | message to everyone |
| `/tasks` | task board: s start, d done, h need help, x cancel, f ask a follow-up, t follow-ups |
| `/followup T-4 [@name] question` | ask about a task. Private: only you and the person you ask can see it (default: the other person on the task) |
| `/review <link> @name comments` | share a pull request with your comments. Reviewers reply with Comment, Approve or Request changes. `@team` asks everyone |
| `/threads` | your private conversations (follow-ups and reviews); `ctrl+o` opens a review's link in your browser, `ctrl+x` closes one you started |
| `/team`, `/team invite` | people and presence; managers create a 6-digit invite |
| `/team remove @name`, `/team promote @name` | managers remove someone (not the owner) or make them a manager |
| `/team leave` | leave the team (the owner cannot leave) |
| `/agent codex|claude|auto` | choose your agent (auto-detected otherwise) |
| `/new` | start a new chat (past chats are listed on the next launch) |

## The screen

crewai uses the whole terminal (it switches to the alternate screen and restores your shell when you leave).

| Terminal width | Layout |
|---|---|
| 110 columns or more | split view: Team and Tasks on the left, Conversation on the right, Needs you under Tasks |
| 80 to 109 | one column with a one-line task strip under the header; Tab opens the task view |
| under 80 | one column with a summary line; `/tasks` opens the task view |

Keys: `PgUp` / `PgDn` scroll the conversation (a "newer" marker shows while you are scrolled up, and your place holds when messages arrive). `Tab` (with an empty message box) moves between the message box and the Tasks panel. In the Tasks panel: `↑↓` select, `Enter` open the full task, `s` start, `d` done, `h` need help, `x` cancel, `m` switch Mine / All / Open, `v` show or hide finished tasks. Tasks are grouped Needs help, In progress, To do; long titles are cut to one line, the first lines of the selected task show under the list, and the task view shows the whole description. Mouse wheel scrolling is not supported.

## Connect your own Firebase project

1. Create a Firebase project. Enable Authentication > Email/Password and create a Firestore database.
2. Copy the web app config into `~/.crewai/.env` (see `.env.example`):
   `CREWAI_FIREBASE_PROJECT_ID`, `CREWAI_FIREBASE_API_KEY`, `CREWAI_FIREBASE_AUTH_DOMAIN`, `CREWAI_FIREBASE_APP_ID`.
3. Deploy the rules and indexes: `npx firebase deploy --only firestore --project <id>` (or `npm run deploy:rules -- --project <id>`).
   Redeploy whenever `firestore.rules` or `firestore.indexes.json` changes: the app needs the request indexes, and
   this version writes server timestamps that older rules reject (and older apps cannot write under the new rules).
4. Check: `node dist/cli.js --check`.

Local data lives in `~/.crewai` (`CREWAI_HOME` overrides): `identity.json` (this device's account; keep it private), `settings.json`, `sessions/` (chat history per folder) and `responder.lock` (which open window answers teammates' questions; only one does).

## Tests

```powershell
npm test               # unit, engine and UI flow tests (in-memory service)
npm run emulators      # same contract against the real Firestore rules (needs Java 11+)
npm run test:live      # same contract against YOUR Firebase project in ~/.crewai (creates throwaway data)
```

## Known limits

- Remote answers are not sandboxed by the operating system. Codex runs with its read-only sandbox flag and Claude with read-only tools (shell, edit and web tools explicitly denied), but the approval card always says "Limited protection".
- Answers are scanned for values that look like secrets (keys, tokens, passwords, private keys) and those are replaced before sending. This is best effort and will miss unusual formats.
- "Allow for 7 days" replaces the old "always allow"; after 7 days the owner is asked again.
- Sign-in is a per-device account. Google/Microsoft sign-in is not built yet.
- 6-digit invite codes: each device account may try one code every two seconds, and claiming a code is only possible while joining. Someone with many accounts can still guess faster; add Firebase App Check before a public launch.
- Presence is a 30-second heartbeat; "online" means seen in the last 90 seconds. Event times are server times, and the app estimates the server clock at sign-in, so a wrong laptop clock no longer hides messages.
- Plain-text questions are independent; the agent does not remember earlier ones.
- Firebase can read team data (questions, answers, tasks, messages). Do not put secrets in them.
