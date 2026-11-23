# Repository instructions

## Before you change anything

Read `conversation_context.md` (product decisions and assumptions), then `tasks.md` (the work log and what
is open), then `app/README.md`. The app lives in `app/`. Everything else in the repository supports it.

## What this product is

`crewai` is a terminal chat for teams of coding agents. Each person runs it on their own laptop. Team data
(tasks, messages, questions, bounded answers) lives in a Firebase project; repository files never do.

## Safety rules (do not weaken these to make a feature work)

- Repository contents stay on the owner's device. Team synchronization carries only team records and bounded
  results.
- Display names and roles are labels, not authentication. Authorization is the Firestore rules plus membership.
- Never describe an agent run as read-only unless the operating system enforces it. Label best-effort behavior.
- Private conversations must stay readable only by their participants, in the rules and in the UI.
- Never log or store private keys, tokens or full repository contents. Never put credentials in notes or commits.
- Links are plain `https://` only and are never fetched or opened by the program without the person asking.

## Engineering workflow

- The app talks to the team service only through the `Backend` interface. Add a method there, implement it in
  `memoryBackend.ts` and `firebaseBackend.ts`, add the matching rules in `firestore.rules`, and add a shared
  contract test (`src/testing/`) that runs against both, including the emulator.
- Keep the UI (`src/ui/`), the engine (`src/engine.ts`), parsing (`src/commands.ts`) and the backends separate.
- Add negative tests: malformed, unauthorized, duplicated and out-of-order input.
- Run `npx tsc --noEmit` and `npm test` in `app/` before committing. Run `npm run emulators` when rules change.
  CI runs the tests with a CI flag set; `vitest.config.ts` handles that.
- The terminal draws full-screen with a line-diffing writer (`src/ui/frameStream.ts`). Test screens with the
  helpers in `src/testing/uiWorld.tsx`; compare text with line breaks and box edges removed when panels wrap.
- Do not rewrite or delete a published tag. If a release build fails, fix it and release the next version.
- Deploy rules and indexes before releasing anything that needs them.

## Completion notes

When a piece of work is done, record it in `tasks.md`: what changed, how it was verified, and what is not
verified or not built. If blocked, leave it open and write the exact missing prerequisite and the next step.
