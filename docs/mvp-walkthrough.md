# Two-teammate MVP walkthrough

## Test environment

- Windows 11 development build on the repository owner's laptop.
- Rust/Cargo and Zig are provisioned on E: through `tools\build-herdr.ps1`.
- `CARGO_INCREMENTAL=0`, one Cargo build job, and repository-local Cargo/temp/
  target paths are used; no parallel builds were run.
- The walkthrough uses local SQLite profiles and the existing signed event,
  sync, outbox, and runner tests. No repository contents are copied into a
  team record.

## Product surface verified

The final executable was rebuilt with:

```powershell
.\tools\build-herdr.ps1 -Mode build
```

The executable returned exit code 0 for both the main help and the new check
command help. The relevant command output is:

```text
herdr check <subcommand> ...
herdr check <subcommand>         Create, execute, and inspect bounded repository questions

herdr check commands:
  usage: herdr check <create|run|show> --db PATH --team TEAM_ID [options]
  create checks ReadRepository permission and queues a signed request.
  run executes a target request; this Windows build requires --confirm-unisolated.
  show reads request status and bounded output using a read-only database connection.
```

The focused workflow tests provide the reproducible two-profile evidence:

| Flow | Evidence |
| --- | --- |
| Profile/device persistence | `identity` and `onboard` tests pass |
| Team membership/pairing | `team_storage` invitation tests pass |
| Task history convergence | `team` filter: 39 passed in the final phase run |
| Offline request delivery | `outbox` filter: 3 passed |
| Reconnect/out-of-order convergence | `sync` filter: 34 passed |
| Secure peer failure behavior | `peer` filter: 14 passed |
| Repository question lifecycle | `cli::check` filter: 2 passed |

The check workflow is:

```text
Create signed request
  -> permission check
  -> Queued in local outbox
  -> sync boundary can deliver the event
  -> target rechecks permission and approved path
  -> separate runner process
  -> bounded Completed/Failed result
```

## Reproduction commands

Use E: paths for both local profiles and the approved repository:

```powershell
herdr onboard setup --db E:\team\manager.sqlite `
  --identity E:\team\manager.identity.json --member manager-1 `
  --display-name Manager --role manager --repository-id repo-1 `
  --repository-path E:\team\repo

herdr onboard setup --db E:\team\implementer.sqlite `
  --identity E:\team\implementer.identity.json --member implementer-1 `
  --display-name Implementer --role implementer --repository-id repo-1 `
  --repository-path E:\team\repo

herdr task create --db E:\team\implementer.sqlite --team TEAM_ID `
  --actor implementer-1 --identity E:\team\implementer.identity.json `
  --assignee implementer-1 --description "Review the entry point"

herdr broadcast create --db E:\team\implementer.sqlite --team TEAM_ID `
  --actor implementer-1 --identity E:\team\implementer.identity.json `
  --scope team --message "Walkthrough started"

herdr check create --db E:\team\manager.sqlite --team TEAM_ID `
  --actor manager-1 --identity E:\team\manager.identity.json `
  --target implementer-1 --repository repo-1 `
  --question "What is the project entry point?" --json
```

After a request is delivered to the implementer's database, inspect it with:

```powershell
herdr check show --db E:\team\implementer.sqlite --request REQUEST_ID --json
```

On the current Windows build, a normal run records a failed security status.
An explicitly confirmed trusted local run is:

```powershell
herdr check run --db E:\team\implementer.sqlite --team TEAM_ID `
  --identity E:\team\implementer.identity.json --request REQUEST_ID `
  --program C:\path\to\approved-agent.exe --arg --help `
  --confirm-unisolated --json
```

## Blocked or manual-only steps

The following are not claimed as completed physical product acceptance:

- The current CLI does not yet expose a complete team-create/invite/join and
  grant/revoke orchestration command; those flows are covered by domain and
  storage tests and need a thin command layer for a non-developer walkthrough.
- Automatic peer reconnect/scheduling is not a background worker. The sync
  boundary must be driven by the integration that owns the peer connection.
- Two-device mDNS discovery and firewall behavior require two laptops on the
  same LAN; automated tests only cover signed advertisement parsing and
  tamper rejection.
- The Windows build has no verified read-only filesystem/network sandbox, so
  `/check` is confirmation-gated and must not be marketed as safely read-only.

These limits mean the repository contains a real runnable local/team product
with durable state, authenticated protocol primitives, UI inspection, queued
requests, and a separate runner, but it is not yet a fully packaged two-laptop
consumer experience.
