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

## Install as a command

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
| `/tasks` | task board: s start, d done, h need help, x cancel |
| `/team`, `/team invite` | people and presence; managers create a 6-digit invite |
| `/agent codex|claude|auto` | choose your agent (auto-detected otherwise) |
| `/new` | start a new chat (past chats are listed on the next launch) |

## Connect your own Firebase project

1. Create a Firebase project. Enable Authentication > Email/Password and create a Firestore database.
2. Copy the web app config into `~/.crewai/.env` (see `.env.example`):
   `CREWAI_FIREBASE_PROJECT_ID`, `CREWAI_FIREBASE_API_KEY`, `CREWAI_FIREBASE_AUTH_DOMAIN`, `CREWAI_FIREBASE_APP_ID`.
3. Deploy the rules: `npx firebase deploy --only firestore --project <id>` (or `npm run deploy:rules -- --project <id>`).
4. Check: `node dist/cli.js --check`.

Local data lives in `~/.crewai` (`CREWAI_HOME` overrides): `identity.json` (this device's account; keep it private), `settings.json`, and `sessions/` (chat history per folder).

## Tests

```powershell
npm test               # unit, engine and UI flow tests (in-memory service)
npm run emulators      # same contract against the real Firestore rules (needs Java 11+)
```

## Known limits

- Remote answers are not sandboxed by the operating system. Codex runs with its read-only sandbox flag and Claude with read-only tools, but the approval card always says "Limited protection".
- Sign-in is a per-device account. Google/Microsoft sign-in is not built yet.
- 6-digit invite codes can be guessed by an automated client. Add Firebase App Check or a rate-limited Cloud Function before a public launch.
- Presence is a 30-second heartbeat; "online" means seen in the last 90 seconds. Device clocks should be within a minute of real time.
- Plain-text questions are independent; the agent does not remember earlier ones.
- Firebase can read team data (questions, answers, tasks, messages). Do not put secrets in them.
