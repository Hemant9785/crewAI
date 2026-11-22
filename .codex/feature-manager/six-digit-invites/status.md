# Feature status

Overall: implemented and released as v0.1.6.

## Readiness

- Repository branch `main` is aligned with `origin/main`; existing untracked `aws_guide.txt`, `plan1.jpeg`, and `website/` are out of scope.
- Product/architecture/checklist docs and root/Herdr agent guidance were read. The app is Rust; relay is a Cloudflare Worker with Durable Object SQLite.
- Current paste bug reproduced from code path: `RawInputEvent::Paste` reaches `insert_overlay_text`, which has no Team Dashboard form case; the Ctrl+Shift+V clipboard fallback also does not treat Dashboard forms as a paste target.
- The deployed relay URL is documented in `docs/internet-team.md`. Existing signed invite payload includes the shared team relay token.
- User approved temporary relay storage on 2026-11-22. Do not store codes or invite payloads in logs.

## Progress

- Added Dashboard paste-event insertion and modal clipboard-paste targeting.
- Added two focused regression tests; both passed with `.\tools\build-herdr.ps1 -Mode test -Filter dashboard_join_form`.
- Relay/client implementation and documentation are complete.

## Risks and limits

- Relay operator can read the temporary signed invite and its shared team token. Code is intended to expire after ten minutes and be claimed once.
- Do not claim the whole feature complete until relay and client flows pass.

## Implementation verification (2026-11-22)

- Dashboard paste and six-digit publish/claim client paths implemented.
- Relay tests: 8 passed; TypeScript check and deploy dry run passed.
- Dashboard regression suite: 7 passed.
- Live isolated manager/member smoke passed: signed payload retrieved unchanged, repeat claim 404, member join, manager approval, relay sync.
- Deployed Worker version 5d2b9e8e-87b5-416e-a29f-aa411a307823.
- Release v0.1.6 published from a449469; workflow 45013945310 succeeded. Both downloaded package hashes verified, Windows version is v0.1.6, Linux onboarding/0600 smoke passed.
- Integrated Rust check passed. Full suite attempted: 2,946 passed, 9 failed, stopped after unrelated transport tests stalled. Physical cross-device UI acceptance remains pending.

Release artifact smoke (2026-11-22): downloaded Windows v0.1.6 binary passed live isolated profile setup, signed short-invite lookup, duplicate-claim rejection, join, approval, and member relay sync. Windows package includes ConPTY runtime and licenses.
