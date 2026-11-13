# Phase 0 architecture spike

Date: 2026-11-11

Status: complete

This spike validates the local development path before product features are added. It does not constitute the production team data model or network protocol.

## Results

### Herdr process isolation

The unmodified release binary was started twice on one Windows laptop with separate configuration paths and named sessions:

- `profile-a`: process ID 34404, alive during the five-second check.
- `profile-b`: process ID 14544, alive during the five-second check.

Both processes were stopped after verification. This proves that two local Herdr server processes can coexist when their configuration/session namespaces are separated.

Herdr integration points identified from the source are:

- `herdr/src/main.rs`: process entry and command dispatch.
- `herdr/src/server/`: headless server lifecycle.
- `herdr/src/client/`: terminal client lifecycle.
- `herdr/src/ui/`: terminal rendering and navigation.
- `herdr/src/api/`: local API schema and dispatch.
- `herdr/src/ipc.rs`: platform-local socket abstraction.
- `herdr/src/session.rs`: named session and socket-path isolation.

Herdr does not currently provide an application SQLite layer. Team data should be added as a separate domain/storage module rather than mixed into terminal pane state.

### Independent SQLite profiles

Using the installed SQLite CLI, two independent databases were created under `.tools/phase0-two-profiles/` with the same initial schema:

- `manager.sqlite`: schema version 1, profile `manager-1`, event `event-a`.
- `implementer.sqlite`: schema version 1, profile `implementer-1`, event `event-b`.

Both returned `PRAGMA integrity_check = ok`. The records remained isolated, confirming that the local-per-device storage model is feasible.

The production implementation should replace this probe schema with migrations and an append-only event log as defined in `spec.md`.

### Replaceable transport seam

`spikes/phase0_transport.rs` defines a minimal `Transport` trait and a length-prefixed `TcpTransport` implementation. It was compiled with Rust 1.96.1 and executed successfully:

```text
transport trait loopback passed
```

The production sync layer should preserve this separation: domain events should depend on a transport trait, while TCP, local sockets, and future encrypted peer transports remain implementations behind that trait.

### Codex read-only execution

The installed Codex CLI is `codex-cli 0.167.0-alpha.4.1`. Its noninteractive command supports `--sandbox read-only`.

A separate ephemeral Codex process was run in `.tools/phase0-readonly-probe/` with a prompt to create `write-test.txt`. The result was:

```text
error=patch rejected: writing is blocked by read-only sandbox
The write was blocked by the read-only sandbox. No file was created and no other changes were made.
```

The probe exited successfully and the file did not exist afterward. This provides a technically enforced read-only path for `/check`, provided the product runner always invokes Codex with the read-only sandbox and treats failure to establish that mode as an execution failure.

The direct `codex sandbox` command could not be used because this local installation has no configured permission profile. The supported `codex exec --sandbox read-only --ephemeral` path worked and is the one the product should target.

## Architecture decision from the spike

The first production code slice should be a domain-first module under Herdr, with these boundaries:

1. Team domain types, state transitions, and permission evaluation.
2. SQLite event store and projections.
3. Sync transport trait and local test transport.
4. Codex runner that invokes `codex exec --sandbox read-only --ephemeral`.
5. Herdr UI and CLI adapters.

Do not modify the existing Herdr IPC protocol to carry team events. Herdr's local IPC is for the terminal client/server relationship; team synchronization needs its own authenticated protocol and event envelope.
