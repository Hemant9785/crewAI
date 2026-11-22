# crewAI

crewAI is a terminal-first team coding assistant built on top of Herdr. It
combines local team coordination, signed task and broadcast records,
permission-aware repository requests, and bounded agent execution.

> Status: this repository contains a runnable Internet collaboration prototype. The Cloudflare relay is deployed and its health endpoint responds; a physical two-laptop Internet run has not yet been performed. Team sync runs in the background while Herdr is open; manual sync remains available.

For the team setup, invitation, relay deployment, `/task`, `/brodcast`, and
`/read` walkthrough, see [Internet team prototype](docs/internet-team.md).

## Install a release build

The current release is available on the GitHub Releases page. Install its
prebuilt binary on Linux with one command:

```sh
curl -fsSL https://raw.githubusercontent.com/Hemant9785/crewAI/main/tools/install-team-herdr.sh | sh
```

The installer checks the Linux architecture, verifies the SHA-256 checksum,
installs the `herdr` command to `~/.local/bin`, and adds that folder to your
Bash or Zsh startup files. Open a new terminal and run:

```sh
herdr
```

For Windows, run this in PowerShell:

```powershell
irm https://raw.githubusercontent.com/Hemant9785/crewAI/main/tools/install-team-herdr.ps1 | iex
```

It installs the Windows binary and bundled ConPTY files under
`%LOCALAPPDATA%\Programs\TeamHerdr`, adds that directory to your user `PATH`,
and makes `herdr` available in the current PowerShell session and new
terminals. Linux x86_64 and Windows x86_64 release assets are built from
version tags and published on the GitHub [Releases](https://github.com/Hemant9785/crewAI/releases)
page. The installers
download the published binary without compiling source on the user's machine.
Release binaries do not self-update from upstream Herdr; rerun the matching
installer when a new Team Herdr release is published.

## Upgrading an existing v0.1.x team

Update both devices before testing v0.2.0; older event clients cannot use the new signed-device relay access. Rerun the installer, then close and relaunch Herdr.

v0.1.x kept connection credentials only in memory. Before closing an existing manager session, create and record a fresh invite. After upgrading that manager, use Join with that code once to restore the same team's connection. Its database and identity are preserved. Then issue a separate fresh invite for any teammate that needs to reconnect. Codes expire after ten minutes and are one use. Connections saved by v0.2.0 restore automatically on subsequent restarts.

## Prerequisites

The local development build workflow targets Windows 10/11 and keeps generated
files on the E: drive. End users can install a published Linux or Windows
release without setting up Rust or Zig.

- Git
- PowerShell 5.1 or PowerShell 7+
- Rust toolchain specified by `herdr/rust-toolchain.toml`
- Zig, provisioned by the build wrapper when required
- Enough free space for Rust dependencies, the vendored terminal library, and
  the build target

The wrapper uses the repository-local `.cache` and `herdr/target` directories.
These are ignored by Git and can be reused between builds. Incremental Cargo
compilation is disabled to avoid accumulating incremental outputs.

## Clone the repository

```powershell
git clone https://github.com/Hemant9785/crewAI.git
Set-Location .\crewAI
```

If you clone to another drive, keep the same relative layout and run all
commands from the repository root.

## Build and verify

Use the wrapper instead of running Cargo directly. It configures Cargo,
Rustup, temporary files, Zig, the target directory, one build job, and
non-incremental compilation on E:.

```powershell
.\tools\build-herdr.ps1 -Mode build
.\tools\build-herdr.ps1 -Mode check
.\tools\build-herdr.ps1 -Mode fmt-check
```

Run the focused team tests when changing coordination or security code:

```powershell
.\tools\build-herdr.ps1 -Mode test -Filter team
.\tools\build-herdr.ps1 -Mode test -Filter peer
.\tools\build-herdr.ps1 -Mode test -Filter sync
.\tools\build-herdr.ps1 -Mode test -Filter outbox
```

The final executable is written to:

```text
herdr\target\debug\herdr.exe
```

## Launch the terminal UI

```powershell
$env:HERDR_CONFIG_PATH = 'E:\crewAI-data\config.toml'
& '.\herdr\target\debug\herdr.exe'
```

The application opens the Herdr terminal workspace. Open the top-right global
menu and choose **team dashboard**. The dashboard has five sections: teammates,
tasks, broadcasts, requests, and access. Use it to set up your profile, create a
team, invite or approve teammates, assign work to one or more members, update
task status, send broadcasts, inspect repository access, ask local or remote
Codex questions, and sync. You do not need a separate PowerShell window for
those team actions.

The workspace also has a Team panel beside the terminal. It shows scrollable,
newest-first task and broadcast activity, task completion announcements, and
sync counts. Use the mouse wheel, `Ctrl+PageUp/PageDown` over Tasks, or
`Alt+PageUp/PageDown` over Broadcasts to browse history. Click a linked task to
focus its terminal pane. The Spaces and Agents sidebar stays in place. Use
`F6` or the Team tab in the top bar to hide or show the panel.

Dashboard shortcuts:

| Key | Action |
| --- | --- |
| `p` | Enter or edit your display name; device identity is generated automatically |
| `c` | Create the team |
| `i` | Create a six-digit invite (10 minutes, one use); `y` copies it |
| `n` | Type six digits or paste a legacy invite code |
| `a` | Open join requests; in Requests, approve the selected teammate |
| `x` | Reject the selected join request in Requests |
| `t` | Choose a teammate with Enter, write the task, then Enter to send |
| `b` | Broadcast a team message |
| `/` | Ask this computer or a teammate about a connected project |
| `q` | Ask a teammate about their project (remote shortcut) |
| `o` | Connect an optional local project folder |
| `g` | Switch between saved teams without deleting their data |
| `r` | Ask Codex about this device's approved local repository |
| `s` | Sync the team manually (also available as a fallback to background sync) |
| `y` | Copy the current invite or join-request code |
| `F6` | Show or hide the Team panel |

In the Tasks section, use `j`/`k` to select a task, `a` to accept, `w` to mark
it in progress, `d` to complete, `x` to cancel, and `!` to flag it for
attention. Right-click an agent pane and choose **Link team task** to show its
task in Agents; this pane-to-task link is local to the current Herdr session.

Choose **Create** or **Join** from Team home and enter your name when prompted. Team creation configures the Internet relay automatically. Connections are saved in authenticated encrypted local sidecars and restored on restart. A saved team connection takes precedence over old global `HERDR_RELAY_URL`/`HERDR_RELAY_TOKEN` settings, which are used only to bootstrap unsaved connections. Keep your local identity and database files together. On Linux the encryption key depends on an owner-only, unencrypted identity file; protect your OS account and backups.

For a first teammate: press `i`, then `y` to share the six-digit invite privately.
The six digits stay visible in the dashboard header. In an EC2/browser terminal, clipboard forwarding may not update your computer clipboard; type the visible digits manually if paste shows old content. A copy request is not a confirmation from the browser.
The teammate presses `n`, enters their name if needed, types the six digits, and confirms the team name. Existing teams stay saved.
Their signed join request is sent automatically. On the manager dashboard, open
**4 Requests**, select the teammate with `j`/`k`, then press `a` to approve or `x`
to reject. The teammate sees pending, approved, or rejected status under Requests;
background sync fetches approved membership. No return code or JSON file is needed.

The relay operator can read temporary invitations and pairing metadata. Pending or rejected invite holders cannot read team events. Update both devices for this release: event access now requires a signed approved-device proof. Legacy manual approval remains available with capital `A`.

## Initialize a local profile from the UI

Press `p` to enter your name. Create/Join also asks for it automatically. IDs are generated internally; a repository and Codex are optional for team coordination. Press `o` when you want to connect a project folder. Repository files stay on this computer.
The CLI form below remains available for scripted setup.

```powershell
$data = 'E:\crewAI-data\manager'
New-Item -ItemType Directory -Force -Path $data | Out-Null

& '.\herdr\target\debug\herdr.exe' onboard setup `
  --db "$data\manager.sqlite" `
  --identity "$data\manager.identity.json" `
  --member manager-1 `
  --display-name "Your Name" `
  --role manager `
  --repository-id crewAI-repository `
  --repository-path (Get-Location).Path
```

Point the dashboard at that database before launching the UI:

```powershell
$env:HERDR_TEAM_DB = 'E:\crewAI-data\manager\manager.sqlite'
& '.\herdr\target\debug\herdr.exe'
```

The supported roles are `manager`, `group-head`, `part-head`, and
`implementer`.

## Implemented prototype

- Create a team, issue an invitation, join from another laptop, and approve
  the automatically delivered join request in Requests.
- Assign signed tasks to a selected teammate, update task status
  from the dashboard, and see completion announcements in team activity.
- Browse newest-first task and broadcast feeds, link local agent panes to tasks,
  inspect permission decisions, and create remote `/check` requests in Herdr.
- Send signed `/task` assignments and `/brodcast` (or `/broadcast`) messages
  through LAN sync or the optional Cloudflare relay.
- Run `/read` locally to ask a separate Codex CLI session about this device's
  approved repository. The query and repository context do not pass through
  the team relay.
- Keep repository files, SQLite data, and private keys on their owning device.
  The relay stores bounded team events and delivery metadata.
- Background relay sync starts after invite/join/approval when relay credentials
  are restored from the protected local connection store. Manual sync remains available in the
  dashboard.

See [the Internet team walkthrough](docs/internet-team.md) for deployment,
setup, invitation, and command examples.

## Asking project questions

Press `/`, choose **This computer** or an approved teammate, then write your question. The project owner connects their folder with `o`. An EC2 manager can coordinate tasks/messages and ask the Windows implementer's project without installing Codex on EC2.

Remote questions appear in **4 Requests**. When no join requests are pending, use Up/Down to select a question and Enter to review it, or click the question. `[` / `]` also selects questions when the join inbox has entries. The owner can allow once, allow the project, or deny. Execution requires a separate explicit limited-isolation choice; use Up/Down to read the warning. A completed answer is saved under Requests; Enter opens the scrollable result. Codex CLI must be installed and signed in on the computer answering the question. An existing Codex conversation alone is insufficient.

In **Tasks**, Enter opens history and `f` adds a follow-up. In **Messages**, Enter reads the full message. Drafts survive cancellation. Saved/queued feedback means the event is waiting for synchronization, not proof that an offline person has seen it.

## Pending work and known limits

- A physical two-laptop Internet run has not been verified. Local two-profile
  testing against `wrangler dev` is documented.
- The permission graph is read-only in the dashboard. Grant/revoke UI is pending
  because permission changes do not yet have a signed, synchronized command
  path.
- Pane-to-task links are session-local and reset when Herdr exits.
- A physical two-laptop run of background Internet sync has not yet been
  verified.
- The prototype has no CLI for member revocation or relay-token rotation.
- The relay is not end-to-end encrypted; its operator can read event payloads.
  Keep secrets, repository contents, and local paths out of team messages.
- `/check` on Windows still requires `--confirm-unisolated`; Herdr cannot
  enforce an OS-level read-only boundary for that command. `/read` depends on
  the installed Codex CLI's `--sandbox read-only` behavior.

## Repository layout

- `idea.md` — product idea and goals
- `spec.md` — architecture and security requirements
- `task.md` — implementation checklist and verification notes
- `herdr/` — application source and vendored terminal dependencies
- `docs/` — operations, networking, and MVP walkthrough documentation
- `tools/build-herdr.ps1` — E:-drive build and test wrapper for Windows
- `.github/workflows/release.yml` — version-tagged Linux and Windows release builds
- `tools/install-team-herdr.sh` — Linux release installer
- `tools/install-team-herdr.ps1` — Windows release installer

The `relay/` directory contains the Cloudflare Worker and per-team Durable
Object. The Internet setup and command walkthrough is in
[`docs/internet-team.md`](docs/internet-team.md).

## Data and security notes

- Team records, identities, and databases should stay on the device that owns
  them; never commit identity files, databases, or private keys.
- Windows protects the signing key with DPAPI. Linux stores the identity file
  with owner-only permissions, but the key is not encrypted at rest; protect
  the EC2 account and its backups.
- The dashboard is a bounded local view and does not display private keys or
  repository contents. Signed membership and event checks remain in the
  existing team command layer.
- On the current Windows build, repository checks require explicit
  `--confirm-unisolated` because an OS-level read-only sandbox is not yet
  available. Do not treat a separate child process as a filesystem sandbox.

For connection diagnostics, `herdr relay sync` uses the active saved team without copying tokens. An HTTP 401 connection mismatch now has a different explanation from a signature or device-approval failure.
