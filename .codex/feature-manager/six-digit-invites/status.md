# Feature status

Overall: on track. Board: paste fix active; short-code relay protocol queued.

## Readiness

- Repository branch `main` is aligned with `origin/main`; existing untracked `aws_guide.txt`, `plan1.jpeg`, and `website/` are out of scope.
- Product/architecture/checklist docs and root/Herdr agent guidance were read. The app is Rust; relay is a Cloudflare Worker with Durable Object SQLite.
- Current paste bug reproduced from code path: `RawInputEvent::Paste` reaches `insert_overlay_text`, which has no Team Dashboard form case; the Ctrl+Shift+V clipboard fallback also does not treat Dashboard forms as a paste target.
- The deployed relay URL is documented in `docs/internet-team.md`. Existing signed invite payload includes the shared team relay token.
- User approved temporary relay storage on 2026-11-22. Do not store codes or invite payloads in logs.

## Progress

- Added Dashboard paste-event insertion and modal clipboard-paste targeting.
- Added two focused regression tests; both passed with `.\tools\build-herdr.ps1 -Mode test -Filter dashboard_join_form`.
- Next: implement the short-code relay API and client flow; run relay and Rust tests; inspect the diff and update `task.md`.

## Risks and limits

- Relay operator can read the temporary signed invite and its shared team token. Code is intended to expire after ten minutes and be claimed once.
- Do not claim the whole feature complete until relay and client flows pass.

## Implementation verification (2026-11-22)

- Dashboard paste and six-digit publish/claim client paths implemented.
- Relay tests: 8 passed; TypeScript check and deploy dry run passed.
- Dashboard regression suite: 7 passed.
- Live isolated manager/member smoke passed: signed payload retrieved unchanged, repeat claim 404, member join, manager approval, relay sync.
- Deployed Worker version 5d2b9e8e-87b5-416e-a29f-aa411a307823.
- Windows/Linux binary release v0.1.6 in progress; full Rust suite in progress (baseline has known failures).
