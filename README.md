# crewAI

crewAI is a terminal-first team coding assistant built on top of Herdr. It
combines local team coordination, signed task and broadcast records,
permission-aware repository requests, and bounded agent execution.

> Status: this repository contains a runnable developer MVP. The complete
> two-device onboarding flow, automatic reconnect workers, and an enforced
> Windows read-only execution sandbox are not yet production-ready.

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

The application opens the existing Herdr terminal workspace. Open the
top-right global menu and choose **team dashboard** to view the team overlay.
The dashboard has four sections: teammates, tasks, broadcasts, and requests.

## Initialize a local profile

Onboarding stores a local display name, role label, protected device identity,
and approved repository path. It does not copy repository contents.

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

## Current team-product limitation

The current MVP does not yet expose a complete user-facing team
create/invite/join workflow. The domain and storage layers include membership
and invitation primitives, but the command/UI layer for creating a team and
adding other devices is still future work. See
[`docs/mvp-walkthrough.md`](docs/mvp-walkthrough.md) and
[`docs/team-operations.md`](docs/team-operations.md) for the exact guarantees
and limitations.

## Repository layout

- `idea.md` — product idea and goals
- `spec.md` — architecture and security requirements
- `task.md` — implementation checklist and verification notes
- `herdr/` — application source and vendored terminal dependencies
- `docs/` — operations, networking, and MVP walkthrough documentation
- `tools/build-herdr.ps1` — E:-drive build and test wrapper for Windows

## Data and security notes

- Team records, identities, and databases should stay on the device that owns
  them; never commit identity files, databases, or private keys.
- The dashboard is a bounded local read-only view and does not load private
  keys or repository contents.
- On the current Windows build, repository checks require explicit
  `--confirm-unisolated` because an OS-level read-only sandbox is not yet
  available. Do not treat a separate child process as a filesystem sandbox.
