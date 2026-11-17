# Team AI Cloudflare relay

The relay carries bounded, signed team events between devices that cannot reach each other directly. It does not synchronize repository files or provide repository access. Each client still checks event signatures, local team membership, revocation, permissions, replay, and projections before accepting an event.

## Local development

Requirements: Node.js 22 or newer and npm.

```powershell
npm ci
npm test
npm run typecheck
npm run dev
```

The local endpoint serves `GET /healthz` and the versioned `/v1/teams/{team_id}/events` API. `wrangler dev` uses a local Durable Objects database; it does not require a Cloudflare account.

## Deploy

1. Create or sign in to a Cloudflare account and install Wrangler (`npm ci` above).
2. From this directory, authenticate with `npx wrangler login`.
3. Deploy with `npm run deploy` and record the HTTPS Worker URL.
4. Configure that URL in each Herdr profile using the team relay setup command documented in the repository walkthrough.
5. Create a team and share its one-time invitation with the teammate through a trusted channel.

The Durable Object SQLite migration is declared in `wrangler.jsonc` and Wrangler applies it on deployment. The relay creates one Durable Object namespace instance per team ID. No account credentials or team bearer token belong in source control.

## Data and limits

Requests use HTTPS in deployment. The Worker rejects malformed event envelopes, invalid Ed25519 signatures, mismatched team/device identities, oversized batches and events, repository-material keys, conflicting event IDs, and invalid team tokens. A bootstrap event must be a signed `team.created` event. The Worker exposes an authority-signed token rotation endpoint, but the current Herdr CLI does not expose a rotation command.

Each team's event mailbox is capped at 20,000 events and 64 MiB, and individual event payloads are bounded to 256 KiB. The relay does not currently expire events or compact history. When a team reaches either cap, writes fail with HTTP 507 until an operator-managed retention/compaction feature is added. Keep team messages free of secrets and repository content: event payloads are readable by the relay operator and are not end-to-end encrypted. The bearer token is shared by team clients. The prototype has no member-revocation or token-rotation CLI, so use only with teammates you trust with the shared credential.

Worker logs contain request metadata only; the implementation does not log event bodies, signatures, tokens, repository paths, or command output. Cloudflare account deployment and a physical second-device Internet check require the account owner and a second device.
