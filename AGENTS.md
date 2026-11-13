# Repository instructions

## Before implementation

Read `idea.md`, `spec.md`, and `task.md` in that order. `spec.md` is the architecture source of truth. `task.md` is the ordered checklist; do not skip tasks or check them off early.

## Scope and safety

- Preserve the planning documents and existing Herdr behavior unless the active task explicitly requires a change.
- Keep repository contents on the repository owner's device. Team synchronization is limited to team records and bounded command results.
- Treat display names and roles as labels, not authentication.
- Do not trust mDNS discovery as authentication.
- Do not weaken signature validation, permission checks, revocation, replay protection, or process isolation to make a feature work.
- Never describe a Codex process as read-only unless the operating-system boundary enforces it. Clearly label best-effort behavior.
- Do not expose private keys, secrets, full repository contents, or unbounded command output in logs or events.

## Engineering workflow

- Inspect the repository, Herdr instructions, license, and current behavior before editing code.
- Implement one numbered task at a time.
- Prefer domain and protocol tests before UI changes.
- Keep UI, domain, SQLite, synchronization, and Codex execution in separate modules or interfaces.
- Use migrations for schema changes.
- Add negative tests for malformed, unauthorized, duplicated, replayed, and out-of-order input.
- Run the narrowest relevant tests after each change, then the full test suite before completing a phase.
- Do not modify unrelated files or reformat large areas without need.

## Completion notes

When a task is complete, update its checkbox in `task.md` and add a short note containing:

- What changed.
- Verification commands or manual steps.
- Known limitations.

If blocked, leave the checkbox unchecked and document the exact missing prerequisite and the next action.
