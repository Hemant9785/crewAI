# Team product operations

## Security warning

The team product is terminal-first and stores team records locally. Repository
files are not synchronized. On the current Windows build, a `/check` request
is not safely read-only unless the operator explicitly passes
`--confirm-unisolated`; that flag means the child process can write to the
approved repository. Herdr reports this as `failed to establish isolation` and
`network isolation unavailable`. A separate process is not a filesystem
sandbox.

## Local data

Team data and identity paths are explicit rather than hidden:

| Data | Location or control | What is stored |
| --- | --- | --- |
| Team SQLite database | `herdr ... --db PATH` | Team records, signed event log, projections, requests, and outbox metadata |
| Dashboard database | `HERDR_TEAM_DB` | The database read by the local read-only dashboard |
| Device identity | `herdr ... --identity PATH` | Device ID, public key, and a local private signing key |
| Herdr configuration | `%APPDATA%\herdr\config.toml` on Windows release builds, or `HERDR_CONFIG` when set | Herdr terminal configuration |
| Herdr logs | `%APPDATA%\herdr\herdr.log` plus client/server logs on Windows release builds | Operational diagnostics, subject to Herdr's rotating log policy |

Debug builds use the `herdr-dev` application directory. `XDG_CONFIG_HOME`
and `XDG_STATE_HOME` are honored on platforms that use them; the current
logging implementation derives its session data directory from the config
directory. The team database and identity file can be placed on the owner's
chosen disk, including E:, by passing paths there.

Windows encrypts the private key with DPAPI. Linux stores it in the local
identity file with mode `0600` and creates a new identity directory with mode
`0700`; this Linux fallback is not encrypted at rest, so protect the account
and its backups.

The private key never leaves its device. Do not copy identity files between
devices or put them in source control. Display names and role labels are not
authentication credentials.

## What crosses the network

Normal synchronization sends bounded, signed team events and bounded request
results. It does not send repository files, private keys, or an unbounded
command transcript. Peer authentication uses the signed device handshake;
mDNS only discovers a possible address. See [team networking](team-networking.md)
for multicast, firewall, client-isolation, and manual-address behavior.

The intended request flow is:

```text
requester creates signed event
        -> local outbox queues it
        -> authenticated peer syncs bounded event
        -> target rechecks permission and runs a separate process
        -> bounded result event returns to requester
```

There is no automatic peer-reconnect worker yet. A request can remain in the
outbox until the existing synchronization boundary is driven again.

## Running a repository check

Create and inspect a request with explicit local paths:

```powershell
herdr check create --db E:\team\member.sqlite --team TEAM_ID --actor MEMBER_ID `
  --identity E:\team\member.identity.json --target PERSON_ID `
  --repository REPOSITORY_ID --question "What is the entry point?" --json

herdr check show --db E:\team\member.sqlite --request REQUEST_ID --json
```

The create command checks `ReadRepository` permission and queues a signed
request. The target-side `run` command rechecks the permission, requires a
locally approved repository, and records `Queued`, `Delivered`, `Running`,
then `Completed` or `Failed` status. Output is capped by the runner.

On this Windows build, normal execution is intentionally rejected because no
verified read-only filesystem boundary is installed. An operator may make the
risk explicit for a trusted local command:

```powershell
herdr check run --db E:\team\member.sqlite --team TEAM_ID `
  --identity E:\team\member.identity.json --request REQUEST_ID `
  --program C:\path\to\approved-agent.exe --arg --help `
  --confirm-unisolated --json
```

The result remains labeled `failed to establish isolation` and
`network isolation unavailable`; it must not be described as safely
read-only. Do not use `--confirm-unisolated` for an untrusted agent or prompt.

## Build and cleanup policy

Use `tools\build-herdr.ps1` for verification. It places Cargo, Rustup,
temporary files, and `target` under the repository's E: drive cache/target
paths, sets `CARGO_INCREMENTAL=0`, and limits build jobs. Do not run raw Cargo
builds in parallel with the wrapper. Reuse the E: dependency cache for normal
locked checks; remove only the repository's E: cache/target directories when a
deliberate clean build is required.

## Current guarantees and limitations

- Signed events, active-device checks, permission revocation, replay checks,
  bounded payloads, and duplicate-safe projections are enforced by code.
- The interactive Herdr session is not attached to the separate check process.
- Repository read-only and network isolation are not enforced on this Windows
  build; the status is surfaced and normal checks are confirmation-gated.
- mDNS physical discovery still needs verification on two devices and may be
  blocked by managed Wi-Fi client isolation.
- An optional Cloudflare relay supports Internet event synchronization in the
  prototype; deployment and physical cross-network testing remain manual. Run
  `herdr relay sync` to fetch events because automatic reconnect is not yet a
  background worker. Repository synchronization and automatic code changes are
  outside scope. Local `/read` uses the installed Codex CLI's explicit
  `--sandbox read-only` mode; Herdr does not add a separate OS boundary.
