# Feature brief: pasteable six-digit team invites

## Goal

Let a teammate join from Herdr by typing a short numeric code, and make terminal clipboard paste work in Team Dashboard forms.

## Included

- Route terminal paste events and Ctrl+Shift+V into the active Team Dashboard form.
- Keep the existing signed invite and signed join-request verification.
- Resolve a six-digit, short-lived, one-use invite through the configured Cloudflare relay.
- Keep manager approval as an explicit step.

## Assumptions and decision

- The joining client can use `HERDR_RELAY_URL` or the project’s default deployed relay URL.
- The relay may temporarily store the signed invite bundle, including its shared relay token. The owner approved this on 2026-11-22.
- Proposed code lifetime: 10 minutes; one successful claim; limit failed claims per source IP.
- The six digits are a temporary transport lookup, not team authentication. Existing signed invitation and join approval remain authoritative.

## Success

- A user can paste long text into a Dashboard invite or approval form.
- A manager creates and shares six digits; a teammate enters them, receives a signed join request, and still needs manager approval.
- Relay tests cover issuance, successful one-time claim, expiry, malformed codes, and rate limiting.

## Not included

- Shortening the signed join-request code.
- Repository transfer or changing membership/permission verification.
