<div align="center">

# crewai

**Ask the codebase, not the person.**

A team chat in your terminal. Ask a teammate's coding agent about their repository, even when they are
away from their desk. Their code never leaves their laptop.

[![Latest release](https://img.shields.io/github/v/release/Hemant9785/crewAI?label=release)](https://github.com/Hemant9785/crewAI/releases)
![Platforms](https://img.shields.io/badge/platforms-Windows%20%7C%20macOS%20%7C%20Linux-informational)

[Website](https://team-chat-for-agents.vercel.app) · [Docs](https://team-chat-for-agents.vercel.app/docs) · [Releases](https://github.com/Hemant9785/crewAI/releases) · [Report a bug](https://github.com/Hemant9785/crewAI/issues)

</div>

## Why

Cross-repo questions cost time. A teammate pings you about how something in your repo behaves. You stop,
ask your own coding agent, copy the answer and paste it back. crewai removes that loop: they ask your
codebase directly, and your local agent answers.

```text
you        /read @backend what does the orders endpoint return when it's empty?
backend    approval: allow once, allow you for 7 days, or decline
backend's agent   It returns 200 with { "orders": [], "nextCursor": null }. See routes/orders.ts.
```

## What you get

- **Ask any project.** Plain text asks your own agent about the folder you opened. Add `@name` to ask a
  teammate's agent about theirs.
- **You stay in control.** The owner chooses allow once, allow that person for 7 days, or decline.
  Pre-approve someone and they get answers while you are away.
- **Tasks across repos.** Assign work to one or more people and track started, done or needs help.
- **Private follow-ups.** Ask about a task in a thread only you and the other person can read.
- **Pull request reviews.** Share a PR link with comments. Reviewers reply Comment, Approve or Request
  changes. The link is never fetched and the diff is not reviewed for you.
- **One file, three systems.** Windows, macOS and Linux. No Node.js needed. Updates itself.
- **Works with** Codex CLI and Claude Code.

## Quick start

1. Install Codex CLI or Claude Code. Without one you can still chat, assign tasks and broadcast.
2. Run the one-line installer for your system (below).
3. Open a terminal in a project folder and type `crewai`.

The first run asks for your name, then **Join a team** (enter the 6-digit code from a manager) or
**Create a team**. To look around alone first, run `crewai --demo` for simulated teammates.

## Install

**Windows (PowerShell)**

```powershell
powershell -c "irm https://raw.githubusercontent.com/Hemant9785/crewAI/main/tools/install-crewai.ps1 | iex"
```

**macOS and Linux**

```sh
curl -fsSL https://raw.githubusercontent.com/Hemant9785/crewAI/main/tools/install-crewai.sh | sh
```

The installer finds the newest release, downloads the right file for your system, checks its SHA-256
checksum (and installs nothing if it does not match), puts `crewai` on your PATH and prints the version.
It needs no admin rights: it installs to `%LOCALAPPDATA%\Programs\crewai` on Windows and
`~/.local/bin` on macOS and Linux. Open a new terminal afterwards. To pin a version, set
`CREWAI_VERSION=crewai-v0.4.0` first; to choose the folder, set `CREWAI_INSTALL_DIR`.

<details>
<summary><b>Manual install</b></summary>

Download the file for your system and `SHA256SUMS` from the
[Releases page](https://github.com/Hemant9785/crewAI/releases) (tags named `crewai-v…`).

| System | File |
|---|---|
| Windows 64-bit | `crewai-windows-x64.exe` |
| macOS, Apple silicon (M1 or newer) | `crewai-macos-arm64` |
| macOS, Intel | `crewai-macos-x64` |
| Linux 64-bit | `crewai-linux-x64` |
| Linux ARM64 | `crewai-linux-arm64` |

Windows (PowerShell, from the folder you downloaded to):

```powershell
Get-FileHash .\crewai-windows-x64.exe -Algorithm SHA256   # compare with SHA256SUMS
$dir = "$env:LOCALAPPDATA\Programs\crewai"
New-Item -ItemType Directory -Force $dir | Out-Null
Move-Item -Force .\crewai-windows-x64.exe "$dir\crewai.exe"
[Environment]::SetEnvironmentVariable("Path", [Environment]::GetEnvironmentVariable("Path", "User") + ";$dir", "User")
```

macOS and Linux (replace the file name with yours):

```sh
shasum -a 256 crewai-macos-arm64        # compare with SHA256SUMS (Linux: sha256sum)
chmod +x crewai-macos-arm64
sudo mv crewai-macos-arm64 /usr/local/bin/crewai
xattr -d com.apple.quarantine /usr/local/bin/crewai   # macOS only: allow a downloaded, unsigned file
```

No `sudo`? Use `mkdir -p ~/.local/bin && mv crewai-linux-x64 ~/.local/bin/crewai` and make sure
`~/.local/bin` is on your `PATH`.

The binaries are **not code-signed yet**. Double-clicking the Windows file may show "Windows protected
your PC" (More info, then Run anyway); running it from a terminal does not.

</details>

## Using it

| In the chat | What it does |
|---|---|
| plain text, `/read question` | asks your own coding agent about this folder |
| `/read @name question` | asks a teammate's agent about their folder (they approve first) |
| `/task @a @b what to do` | assigns work, one task per person; marking one done tells the team |
| `/tasks` | opens the task board (`s` start, `d` done, `h` need help, `x` cancel, `f` follow-up) |
| `/followup T-4 [@name] question` | asks about a task, privately |
| `/review <link> @name comments` | shares a pull request for review (`@team` asks everyone) |
| `/threads` | your private conversations |
| `/broadcast message` | messages the whole team |
| `/team`, `/team invite` | people and presence; managers create a 6-digit invite |
| `/agent codex\|claude\|auto` | chooses your agent |
| `/new` | starts a new chat (past chats are listed on the next launch) |

The full command list, keys and layouts are in [app/README.md](app/README.md).

## Update and diagnose

```sh
crewai --update       # or: crewai update   (add --dry-run to only check)
crewai doctor         # version, install path, agents found, team service, update status
```

The updater checks the newest stable release, downloads the binary for your system, verifies it
against `SHA256SUMS`, and checks that it reports the expected version before replacing the installed
program. `doctor` does not print credentials.

## How it works

```text
your laptop                      team's Firebase project              teammate's laptop
┌──────────────┐  question      ┌───────────────────────┐  request   ┌──────────────────┐
│ crewai       │ ─────────────► │ questions, tasks,     │ ─────────► │ crewai           │
│              │ ◄───────────── │ messages, short       │ ◄───────── │ runs their local │
│              │  short answer  │ answers (no repo      │  answer    │ agent in their   │
└──────────────┘                │ files)                │            │ folder           │
                                └───────────────────────┘            └──────────────────┘
```

Questions, bounded answers, tasks and messages go through the team's Firebase project, protected by
Firestore rules and team membership. Repository files stay on their owner's device. The teammate's
app has to be running for their agent to answer.

## Security and privacy

- **Repository contents stay on the owner's device.** Only team records and bounded answers are shared.
- **Approval first.** The owner decides who may ask. "Allow for 7 days" expires and asks again.
- **No OS sandbox.** Agents run with read-only options (Codex read-only sandbox flag; Claude with shell,
  edit and web tools denied), but the operating system does not enforce this and the approval card says
  so. Answers are scanned for values that look like secrets before they are sent, which is best effort.
- **Private threads** (follow-ups and reviews) are readable only by their participants, enforced by the
  Firestore rules and in the interface.
- **Names and roles are labels, not authentication.** Authorization is the Firestore rules plus team
  membership. Sign-in is a per-device account; Google and Microsoft sign-in are not built yet.
- **Links** are plain `https://` and are never fetched or opened unless you ask.
- The service operator can read team data, so do not put secrets in tasks, messages or questions.

## Status

Early release (see the [latest release](https://github.com/Hemant9785/crewAI/releases)). Most testing so
far has been on Windows; the macOS and Linux binaries are built by CI for every release and have had
less real-world use. Binaries are not code-signed yet. Plain-text questions are independent: the agent
does not remember earlier ones.

Found a problem? Run `crewai doctor` and [open an issue](https://github.com/Hemant9785/crewAI/issues)
with its output.

## Build and develop

Needs Node.js 20 or newer (22 or newer to build the standalone binary).

```sh
cd app
npm ci
npm test                  # unit, engine and screen tests (in-memory team service)
npm run emulators         # the same contract against the real Firestore rules (needs Java 11+)
npm run build && node dist/cli.js --demo
npm run build:binary      # standalone binary for your OS: app/release/crewai-<os>-<cpu>
```

To use your own Firebase project, see [app/README.md](app/README.md#connect-your-own-firebase-project).
Release binaries embed the team service settings from the repository variables
`CREWAI_FIREBASE_PROJECT_ID`, `CREWAI_FIREBASE_API_KEY`, `CREWAI_FIREBASE_AUTH_DOMAIN` and
`CREWAI_FIREBASE_APP_ID`.

Releasing: deploy any rules or index changes first (`npx firebase deploy --only firestore`), then push
a tag such as `crewai-v0.4.1`. The [workflow](.github/workflows/crewai-release.yml) builds all five
binaries and publishes them with `SHA256SUMS`. Working rules for contributors and agents are in
[AGENTS.md](AGENTS.md).

| Path | What it is |
|---|---|
| `app/` | the program: TypeScript, an Ink terminal UI, a Firebase backend, and the Firestore rules and indexes |
| `tools/` | the one-line installers |
| `.github/workflows/` | the release build for every platform |
| `docs/`, `reference_images/` | review notes and interface references |
| `conversation_context.md`, `tasks.md` | product decisions and the work log |

## License

No license has been chosen yet. Until one is added, all rights are reserved.
