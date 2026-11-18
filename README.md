# crewAI

crewAI is a terminal-first team coding assistant built on top of Herdr. It
combines local team coordination, signed task and broadcast records,
permission-aware repository requests, and bounded agent execution.

> Status: this repository contains a runnable Internet collaboration prototype. The Cloudflare relay is deployed and its health endpoint responds; a physical two-laptop Internet run has not yet been performed. Relay sync is currently started manually.

For the team setup, invitation, relay deployment, `/task`, `/brodcast`, and
`/read` walkthrough, see [Internet team prototype](docs/internet-team.md).

## Prerequisites

The documented build workflow targets Windows 10/11 and keeps generated files
on the E: drive.

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
menu and choose **team dashboard**. The dashboard has four sections: teammates,
tasks, broadcasts, and requests. Once Herdr is open, use the dashboard to set up
your profile, create a team, invite or approve teammates, assign tasks, send
broadcasts, ask local Codex questions, and sync. You do not need a separate
PowerShell window for those team actions.

Dashboard shortcuts:

| Key | Action |
| --- | --- |
| `p` | Set up this device's profile and approved repository |
| `c` | Create the team |
| `i` | Create a signed invite code; press `y` to copy it |
| `n` | Paste an invite code, or reconnect with it after restart |
| `a` | Paste and approve a teammate's signed join request code |
| `t` | Assign a task to a member ID |
| `b` | Broadcast a team message |
| `r` | Ask Codex about this device's approved local repository |
| `s` | Sync the team through the Internet relay |
| `y` | Copy the current invite or join-request code |

For a first teammate: press `i`, then `y` to copy the invite code. Send it
privately to them. They set up their device with `p`, press `n`, and paste the
code. They press `y` to copy their signed join request and send it privately
back to you. Press `a` and paste that request to approve the new member. Invite
codes include the team's relay credential, so never commit or post one publicly.
Herdr keeps the credential in memory for the current session; after a restart,
the teammate can paste the same valid invite again to reconnect.

## Initialize a local profile from the UI

Press `p` in the Team Dashboard and enter a member ID, display name, role,
repository ID, and local repository folder. This creates the local profile,
device identity, and repository approval without copying repository contents.
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
  the signed join request code.
- Send signed `/task` assignments and `/brodcast` (or `/broadcast`) messages
  through LAN sync or the optional Cloudflare relay.
- Run `/read` locally to ask a separate Codex CLI session about this device's
  approved repository. The query and repository context do not pass through
  the team relay.
- Keep repository files, SQLite data, and private keys on their owning device.
  The relay stores bounded team events and delivery metadata.
- Run `herdr relay sync` on the receiving laptop to fetch new events.

See [the Internet team walkthrough](docs/internet-team.md) for deployment,
setup, invitation, and command examples.

## Pending work and known limits

- A physical two-laptop Internet run has not been verified. Local two-profile
  testing against `wrangler dev` is documented.
- Relay downloads currently require a manual `herdr relay sync`; there is no
  background reconnect worker.
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

The `relay/` directory contains the Cloudflare Worker and per-team Durable
Object. The Internet setup and command walkthrough is in
[`docs/internet-team.md`](docs/internet-team.md).

## Data and security notes

- Team records, identities, and databases should stay on the device that owns
  them; never commit identity files, databases, or private keys.
- The dashboard is a bounded local view and does not display private keys or
  repository contents. Signed membership and event checks remain in the
  existing team command layer.
- On the current Windows build, repository checks require explicit
  `--confirm-unisolated` because an OS-level read-only sandbox is not yet
  available. Do not treat a separate child process as a filesystem sandbox.
