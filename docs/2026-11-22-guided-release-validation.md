# Guided team prototype validation

Release candidate: v0.2.0, source commit `98a84db`.

## Working behavior

Create/Join starts with a name; repository setup is optional. Connections and drafts use authenticated encrypted local files. Joining a different team preserves the previous database and identity. Team home exposes Ask, Assign, Message, Invite, Connect project and Switch team. Recipient selection uses Enter. Tasks include status/history/followups; messages and answers have scrollable detail views.

Questions are advertised by project label and owner-scoped ID, without local paths. The repository owner reviews each question and explicitly confirms limited isolation. A separate native assistant captures a bounded final answer. An open Codex conversation alone is not a configured CLI.

Approved-device signatures now gate team-event reads/writes. Pairing tokens alone do not allow pending/rejected devices to read team events. Task insertion enforces authorization and deterministic concurrent event projection. Update both devices.

## Evidence

- Integrated Rust check and final development build passed.
- Dashboard tests: 15 passed, including recipient Enter selection, name-only onboarding, paste, visible invite and mouse copy/close routing.
- Connection storage: 6 passed, including encrypted restart recovery, tamper detection and non-destructive context switching.
- Team storage: 23 passed; one opt-in native mock test separately passed.
- Question/permission projection: 5 passed. Native assistant helpers: 3 passed.
- Sync/background-related filter: 14 passed. CLI filter: 146 passed.
- Relay TypeScript check and runtime tests: 11 passed.
- Native mock roundtrip: requester question -> owner explicit consent -> spawned native helper final answer -> requester Completed result passed. This verifies process/result plumbing; it is not a real model-quality test.
- Independent read-only review found mouse-footer routing and clipped consent warnings. Both were fixed and re-reviewed; no further concrete regression found in those changes.
- Live deployed relay smoke: six-digit lookup, one successful claim, automatic signed inbox, manager approval, teammate membership sync, rejection/status, task/message delivery passed. Database assertions confirmed receipt and rejected-member absence.
- Live task lifecycle: teammate accepted, started, completed and followed up; manager received Done and followup.
- Relay deployment: `84a58c7f-c22b-4169-ba85-d240f87892a7`.

Focused test commands on Windows:

```powershell
powershell -ExecutionPolicy Bypass -File .	oolsuild-herdr.ps1 -Mode test -Filter team_session::tests
powershell -ExecutionPolicy Bypass -File .	oolsuild-herdr.ps1 -Mode test -Filter client::shell::tests::team_dashboard
powershell -ExecutionPolicy Bypass -File .	oolsuild-herdr.ps1 -Mode test -Filter team_storage::
```

The full suite was attempted with a 90-second cap and did not finish: 402 tests passed before timeout; two manifest-fixture tests failed when the executable was run from the repository root. This is not a full-suite pass. Prior broad-suite failures/stalls remain unresolved.

## Limits and pending acceptance

Physical EC2/Windows UI acceptance and a real Codex query remain unverified. Linux identity confidentiality relies on an owner-only unencrypted key file. Execution isolation remains limited and is labelled in the confirmation screen. General team/member permission editing and revocation UI, project selection when a teammate has several linked projects, contextual slash-command polish and broader user testing remain pending. Task 2.0 stays unchecked.

## Release artifacts

Windows/Linux release workflow: https://github.com/Hemant9785/crewAI/actions/runs/45035221881

Workflow succeeded for Linux musl x86_64 and Windows MSVC x86_64, including Linux onboarding/0600 identity smoke and Windows ConPTY packaging. Published release: https://github.com/Hemant9785/crewAI/releases/tag/v0.2.0

Downloaded both archives and verified SHA-256:

- Linux: `bb3a21cf9cebab97c924cdb5ded08fbae467a7a1c77af57be17efc044203d614`
- Windows: `b10dbc9fb03250e358a4dc56f281fb4239dda09b63ea8fcfbabf90cb20880033`

Downloaded Windows binary reports `herdr v0.2.0` and passed the live invitation/approval/rejection/status/task/message smoke. Database assertions confirmed received task/message and rejected-member absence. Linux archive contains herdr and LICENSE. Local installed Windows binary was not replaced as part of this GitHub release request.
