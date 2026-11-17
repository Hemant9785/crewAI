# Internet team prototype

Herdr can exchange signed team events through the optional Cloudflare relay when teammates are on different networks. The relay carries bounded team events only; it never receives repository files or repository paths. Run `herdr relay sync` after reconnecting to fetch new events. Automatic background reconnect is not implemented yet.

## Deploy the relay

From `relay/`, install, check, and deploy the Worker:

```powershell
npm ci
npm test
npm run typecheck
npx wrangler login
npm run deploy
```

Keep the HTTPS Worker URL. For local development, run `npm run dev -- --ip 127.0.0.1 --port 8787`; localhost HTTP is accepted by Herdr only for this development case.

The relay holds signed event payloads in Cloudflare Durable Object SQLite storage. The current per-team limits are 20,000 events and 64 MiB; there is no expiry or compaction yet. The relay operator can read event payloads. Do not put secrets, repository contents, patches, or local paths in team messages. A team uses one shared bearer token, so send invitation files privately and keep them out of Git.

## Share the repository and create a team

Share the application repository through your normal Git host, and have the
teammate clone it on their laptop. Each person then approves their own local
checkout with `onboard setup`; repository files are never copied through the
team relay.

On each device, first create a local profile and approve that device's own repository. Use separate database and identity files on each laptop:

```powershell
herdr onboard setup --db .herdr/team.sqlite --identity .herdr/identity.json --member alice --display-name Alice --role manager --repository-id my-repo --repository-path C:\work\my-repo
```

On the owner's device, set the relay URL and a random 32-byte base64url token in the shell. For PowerShell:

```powershell
$random = New-Object System.Security.Cryptography.RNGCryptoServiceProvider
$bytes = New-Object byte[] 32
$random.GetBytes($bytes)
$env:HERDR_RELAY_TOKEN = [Convert]::ToBase64String($bytes).TrimEnd('=').Replace('+','-').Replace('/','_')
$env:HERDR_RELAY_URL = 'https://YOUR-WORKER.workers.dev'
```

Create the team, then create an invitation file:

```powershell
herdr team create --db .herdr/team.sqlite --identity .herdr/identity.json --name "My team" --url $env:HERDR_RELAY_URL
herdr team invite --db .herdr/team.sqlite --team TEAM_ID --identity .herdr/identity.json --out .herdr/invite.json
```

Copy `TEAM_ID` from `team create`. Send `invite.json` to the intended teammate through a private channel. It contains the pairing code and shared relay token.

## Join and approve the teammate

The teammate runs `onboard setup` on their laptop, using their own member ID, identity file, database, and local repository path. Then they join from the invitation and send the generated request file to the owner:

```powershell
herdr team join --db .herdr/team.sqlite --identity .herdr/identity.json --invite .\invite.json --request-out .\join-request.json
```

The owner approves that signed device request and publishes the membership events:

```powershell
herdr team approve --db .herdr/team.sqlite --team TEAM_ID --identity .herdr/identity.json --request .\join-request.json
```

The teammate reads the relay URL and token from the invite file into their current shell, then downloads the approved events:

```powershell
$invite = Get-Content .\invite.json -Raw | ConvertFrom-Json
$env:HERDR_RELAY_URL = $invite.relay_url
$env:HERDR_RELAY_TOKEN = $invite.relay_token
herdr relay sync --db .herdr/team.sqlite --team TEAM_ID --url $env:HERDR_RELAY_URL
```

Keep the environment variables set in each terminal. The invite expires after seven days. Approval is explicit: the owner's active team authority and local identity sign the member and device events. Display names and role labels are not authentication.

## Assign a task and broadcast a message

Task and broadcast creation sync immediately when `HERDR_RELAY_URL` and `HERDR_RELAY_TOKEN` are set. The receiving laptop runs `herdr relay sync` to receive them.

```powershell
herdr /task create --db .herdr/team.sqlite --team TEAM_ID --actor alice --identity .herdr/identity.json --assignee bob --description "Review the API"
herdr /brodcast create --db .herdr/team.sqlite --team TEAM_ID --actor alice --identity .herdr/identity.json --scope team --message "Review starts today"
```

`/broadcast` is also accepted. Task assignment uses the teammate's member ID from their signed join request. Teammates see received tasks in the local database, for example `herdr task show --db .herdr/team.sqlite --task TASK_ID`.

## Ask a local repository question

Approve a repository on this laptop with `onboard setup`, then run:

```powershell
herdr /read --db .herdr/team.sqlite --repository-id my-repo --question "Where is request validation implemented?"
```

Herdr starts a separate `codex.exe exec --sandbox read-only --ephemeral` process in that approved directory, with a five-minute timeout and a 64 KiB output limit. Codex's installed CLI supplies the sandbox; Herdr does not claim an additional operating-system boundary of its own. If Codex is missing or refuses the sandbox invocation, the command fails. The question and the repository context needed to answer it are sent to the Codex service; neither is sent through the team relay.

## Prototype verification and limits

The local two-profile walkthrough passed against `wrangler dev`: team creation, signed invitation, join request, authority approval, relay sync, remote task projection, and remote broadcast projection. The Rust cursor and `/read` authorization tests pass, and the relay Worker tests, TypeScript type check, and Wrangler deployment dry run pass.

No Cloudflare account deployment or physical second-laptop Internet run has been performed. Until a background reconnect service is added, run `herdr relay sync` on the receiving device to pull events. The shared bearer token must be rotated out-of-band if a teammate loses access; automated token rotation and membership revocation commands are not part of this prototype.
