# crewAI

Team chat for coding agents, in your terminal. Type `crewai` in a project folder and a chat opens:
ask your own agent about the folder, ask a teammate's agent about theirs, assign tasks, broadcast
messages, ask private follow-up questions about a task, and share a pull request with your
comments so teammates can review it.

Repository files never leave their owner's laptop. Only questions, bounded answers, tasks, and
messages are shared through the team's Firebase project.

| In the chat | What it does |
|---|---|
| plain text, `/read question` | asks your own coding agent about this folder |
| `/read @name question` | asks a teammate's agent about their folder (they approve first) |
| `/task @a @b what to do` | assigns work; marking it done tells the team |
| `/followup T-4 question` | asks about a task, privately |
| `/review <link> @name comments` | shares a pull request for review (Comment, Approve, Request changes) |
| `/broadcast message`, `/tasks`, `/threads`, `/team` | team messages, the task board, your private conversations, people and invites |

Full command list and keys: [app/README.md](app/README.md).

## Install crewai (standalone app)

Type `crewai` in a project folder and a chat opens. Ask your agent about the folder, ask a
teammate's agent about theirs, assign tasks, broadcast messages. It runs on Windows, macOS and
Linux as a single file; you do not need Node.js.

**Before you start**

- Have Codex CLI or Claude Code installed to answer questions about your folder. Without one you
  can still chat, assign tasks and broadcast.
- The release binaries already contain the team service settings (a Firebase project). If you
  built your own binary without them, see [app/README.md](app/README.md#connect-your-own-firebase-project).

**Quick install (one command)**

Windows (PowerShell):

```powershell
powershell -c "irm https://raw.githubusercontent.com/Hemant9785/crewAI/main/tools/install-crewai.ps1 | iex"
```

macOS and Linux:

```sh
curl -fsSL https://raw.githubusercontent.com/Hemant9785/crewAI/main/tools/install-crewai.sh | sh
```

The installer finds the newest `crewai-v…` release, downloads the right file for your system,
checks its SHA-256 checksum (and installs nothing if it does not match), puts `crewai` on your
PATH and prints the version. It needs no admin rights: it installs to
`%LOCALAPPDATA%\Programs\crewai` on Windows and `~/.local/bin` on macOS and Linux. Open a new
terminal afterwards. To pin a version, set `CREWAI_VERSION=crewai-v0.4.0` first; to choose the
folder, set `CREWAI_INSTALL_DIR`.

**Manual install**

**1. Download** the file for your system from the
[Releases page](https://github.com/Hemant9785/crewAI/releases) (tags named `crewai-v…`), plus `SHA256SUMS`:

| System | File |
|---|---|
| Windows 64-bit | `crewai-windows-x64.exe` |
| macOS, Apple silicon (M1 or newer) | `crewai-macos-arm64` |
| macOS, Intel | `crewai-macos-x64` |
| Linux 64-bit | `crewai-linux-x64` |
| Linux ARM64 | `crewai-linux-arm64` |

**2. Install**

Windows (PowerShell, from the folder you downloaded to):

```powershell
Get-FileHash .\crewai-windows-x64.exe -Algorithm SHA256   # compare with SHA256SUMS
$dir = "$env:LOCALAPPDATA\Programs\crewai"
New-Item -ItemType Directory -Force $dir | Out-Null
Move-Item -Force .\crewai-windows-x64.exe "$dir\crewai.exe"
[Environment]::SetEnvironmentVariable("Path", [Environment]::GetEnvironmentVariable("Path", "User") + ";$dir", "User")
```

Open a new terminal afterwards. The file is not code-signed, so double-clicking may show a
"Windows protected your PC" warning (More info, then Run anyway); running it from a terminal does not.

macOS and Linux (replace the file name with yours):

```sh
shasum -a 256 crewai-macos-arm64        # compare with SHA256SUMS (Linux: sha256sum)
chmod +x crewai-macos-arm64
sudo mv crewai-macos-arm64 /usr/local/bin/crewai
xattr -d com.apple.quarantine /usr/local/bin/crewai   # macOS only: allow a downloaded, unsigned file
```

No `sudo`? Use `mkdir -p ~/.local/bin && mv crewai-linux-x64 ~/.local/bin/crewai` and make sure
`~/.local/bin` is on your `PATH`.

**3. Run it**

```sh
crewai --version
cd path/to/your/project
crewai
```

The first run asks for your name, then "Join a team" (enter the 6-digit code from your manager) or
"Create a team". Try `crewai --demo` first if you want to explore with simulated teammates.

## Update and diagnose crewai

Installed releases can update themselves:

```sh
crewai --update
# or
crewai update
```

The updater checks the newest stable `crewai-v*` release, downloads the binary for your system,
verifies it against `SHA256SUMS`, and checks that it reports the expected version before replacing
the installed program. On Windows, the running executable is renamed during replacement; the next
start removes that old copy. Close and reopen other crewai windows to use the updated version.
`crewai update --dry-run` checks whether an update is available without downloading it. If the
installed folder is not writable, run the update with the permissions required for that folder or
reinstall with the installer.

For a support summary, run:

```sh
crewai doctor
```

It prints the version and install path, local data folder, detected Codex/Claude commands, team
name, Firebase connection status, and whether a newer release is available. It does not print
credentials.

**Build the binaries yourself**

```sh
cd app
npm ci
npm run build:binary        # builds for the OS you are on: app/release/crewai-<os>-<cpu>
```

To publish all five: set the repository variables `CREWAI_FIREBASE_PROJECT_ID`,
`CREWAI_FIREBASE_API_KEY`, `CREWAI_FIREBASE_AUTH_DOMAIN` and `CREWAI_FIREBASE_APP_ID`
(Settings, Secrets and variables, Actions, Variables), then run
`git tag crewai-v0.1.0 && git push origin crewai-v0.1.0`. The
[workflow](.github/workflows/crewai-release.yml) builds each binary on its own OS and attaches
them, with `SHA256SUMS`, to a GitHub release.

**Status:** the Windows x64 binary is built and tested (it starts and connects to Firebase). The
macOS and Linux binaries are built by the workflow, which has not run yet, so they are untested.

## Repository layout

| Path | What it is |
|---|---|
| `app/` | the crewai program: TypeScript, an Ink terminal UI, and a Firebase backend (see [app/README.md](app/README.md)) |
| `app/firestore.rules`, `app/firestore.indexes.json` | the team service's security rules and indexes |
| `tools/install-crewai.ps1`, `tools/install-crewai.sh` | the one-line installers |
| `.github/workflows/crewai-release.yml` | builds the binaries for every platform and publishes a release on a `crewai-v*` tag |
| `docs/` | notes such as the 2026-11-23 code review |
| `reference_images/` | the interface references the design follows |
| `conversation_context.md`, `tasks.md` | product decisions and the work log |

An earlier prototype built on a fork of the Herdr terminal multiplexer, with a Cloudflare relay, was
removed from the tree on 2026-11-23. It is still in this repository's git history, before the
commit titled "Remove the Herdr prototype".

## Development

Needs Node.js 20 or newer (22 or newer to build the standalone binary).

```sh
cd app
npm ci
npm test              # unit, engine and screen tests (in-memory team service)
npm run emulators     # the same contract against the real Firestore rules (needs Java 11+)
node dist/cli.js --demo   # after: npm run build
```

Releasing: deploy any rules or index changes first (`npx firebase deploy --only firestore`), then
push a tag such as `crewai-v0.4.1`. The workflow builds all five binaries and publishes them with
`SHA256SUMS`. See [AGENTS.md](AGENTS.md) for the working rules.

## Security notes

- Remote answers are not sandboxed by the operating system, and the approval card says so. They are
  scanned for values that look like secrets before they are sent, which is best effort.
- Private conversations (task follow-ups and reviews) are readable only by the people in them, enforced
  by the Firestore rules. The service operator can still read team data, so do not put secrets in tasks,
  messages or answers.
- Pull request links must be plain `https://` links. crewai never fetches or opens a link by itself.
- Device sign-in is a per-device account; Google or Microsoft sign-in is not built yet.

## License

No license has been chosen yet. Until one is added, all rights are reserved.
