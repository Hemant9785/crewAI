crewAI code review (2026-11-23)
================================

Scope
-----
The active product is the standalone `crewai` app in app/ (TypeScript, Ink UI, Firebase),
together with app/firestore.rules, the installers in tools/install-crewai.*, and the
release workflow .github/workflows/crewai-release.yml. herdr/ and relay/ are marked in
README.md as the legacy prototype, so this review did not look at them in depth.

Baseline: `npx vitest run` in app/ passes (6 files, 58 tests). None of the problems below
is covered by an existing test.

Severity key: [HIGH] users will hit it, or it is a security or data problem.
              [MED]  a real bug with a narrower trigger, or one that confuses users.
              [LOW]  polish, or a minor inconsistency.


1. CODE AND CORRECTNESS BUGS
============================

[HIGH] 1.1 On Windows, an agent that times out is never stopped, and every later question waits behind it
  Where:   app/src/agents.ts:55-72 (exec), app/src/engine.ts:299-301 (enqueue)
  Problem: On Windows the agent starts through `shell: true`, so the real process tree is
           cmd.exe -> codex.cmd/claude.cmd -> node. When the time limit is reached,
           child.kill() ends only cmd.exe. The agent's node process keeps running and keeps
           the stdout pipe open, so the 'close' event, and with it the promise, waits until
           that process ends by itself. If it hangs (for example on a sign-in prompt or a
           network stall), the 120-second limit does nothing, and engine.queue never moves on.
           Every later teammate question then stays in 'running' or 'pending' with no answer.
           The 8-second limit on `--version` in detect() has the same weakness.
  Fix:     On Windows, end the whole tree (`taskkill /pid <pid> /T /F`), or resolve on 'exit'
           as well as 'close' and destroy the streams. Add a test with a child that never exits.

[HIGH] 1.2 Running crewai in two folders at once breaks question handling
  Where:   app/src/config.ts:109-119 (one identity per machine), app/src/engine.ts:274-281
  Problem: identity.json and settings.teamId are stored per machine, not per folder, so every
           running crewai window signs in as the same uid. Each window handles the same incoming
           questions. When window A marks a question 'running', window B sees a 'running'
           question it did not start, treats it as "left over from a previous run" and marks it
           failed with "Interrupted. Please ask again.". A's real answer is then rejected with a
           conflict. For "always allowed" askers, both windows can also try to answer. The
           teammate cannot choose which project answers, and the member's `project` field
           switches between folders on every heartbeat.
  Fix:     Allow only one active responder per uid (a lock file in ~/.crewai, or a per-instance
           claim id stored on the request), and treat 'running' as abandoned only after a
           lease expires. Tell the user when a second window is open.

[HIGH] 1.3 Clock differences between computers hide messages and show the wrong online status
  Where:   app/src/firebaseBackend.ts:278, 299, 320, 338, 380 (createdAt/updatedAt = Date.now());
           app/src/ui/App.tsx:158 (since = local Date.now()); app/src/engine.ts:215-260;
           app/src/types.ts:310-312 (isOnline)
  Problem: Event times come from the sender's clock, but the chat filters events with the
           receiver's clock (`createdAt >= since`). If a teammate's clock is two minutes behind,
           a broadcast, task or answer they send in the first two minutes of your chat is never
           shown. A sender whose clock is ahead moves session.updatedAt into the future, so when
           you resume that chat later, events are dropped. isOnline() compares server lastSeen
           with the local clock, so a laptop whose clock is about 90 seconds off shows everyone
           as online or everyone as offline. The rules accept any number for createdAt, so a
           member can also post with a far-future time and stay at the top of the
           orderBy('createdAt') limit(200) query.
  Fix:     Write serverTimestamp() for createdAt/updatedAt (the rules can require
           `== request.time`), and set `since` from a server time, or the newest timestamp seen
           when the session was attached.

[HIGH] 1.4 After 500 entries, old messages come back at the bottom of the chat every 5 seconds
  Where:   app/src/config.ts:163-174 (appendEntry, MAX_ENTRIES), app/src/engine.ts:209-265
  Problem: Duplicates are detected only against entries that are still in the session. When a
           long chat passes 500 entries, the oldest entries are removed. On the next tick,
           deriveEntries() regenerates every broadcast, task or answer that still matches
           `>= since` and is within the last 200 broadcasts or tasks (requests have no limit),
           sees that its key is missing, and appends it again as a new entry at the end. That
           removes another old entry, which comes back on the next tick, and so on. Users see
           old broadcasts and answers repeated as new messages, and the session file is
           rewritten every 5 seconds.
  Fix:     Keep a separate set of seen keys (stored with the session, or a high-water
           timestamp) that is not trimmed with the entries, or never re-derive anything older
           than the oldest retained entry.

[MED] 1.5 Task board: the selection can jump to a different task, and "done" can be broadcast repeatedly
  Where:   app/src/ui/Chat.tsx:64-76, app/src/ui/Cards.tsx:462-465, app/src/engine.ts:416-429
  Problem: Rows are sorted by updatedAt, and the selection is an index. Pressing 's' changes the
           task's updatedAt, so it moves to the top, and the same index now points at another
           task. Pressing 'd' next then marks the wrong task done. Updates from teammates
           reorder the list in the same way. Pressing 'd' on a task that is already done
           updates it again and sends another "✓ T-n done" broadcast to the whole team each
           time. The board also shows only the first 12 tasks, with no way to scroll.
  Fix:     Track the selection by task id, ignore a change to the status the task already has,
           and let the list scroll.

[MED] 1.6 A failed agent can be reported as a successful answer
  Where:   app/src/agents.ts:120-125
  Problem: When the agent exits with a non-zero code but printed anything on stdout (often an
           error or usage message), run() returns ok:true, and that text is sent to the teammate
           as "answered from X's laptop".
  Fix:     Treat code !== 0 as a failure unless Codex wrote its -o output file. Show stderr in
           the failure message, trimmed and limited in length.

[MED] 1.7 Assigning a task to several people is not atomic, and retrying creates duplicates
  Where:   app/src/firebaseBackend.ts:288-317
  Problem: Each assignee gets its own transaction. If the third one fails, the first two tasks
           already exist, but submit() returns keepDraft and shows an error. The user sends it
           again, and the first assignees get duplicate tasks.
  Fix:     Create all tasks in a single transaction (one taskSeq increment of N), or report
           which assignees succeeded.

[LOW] 1.8 Requests subscription has no limit
  Where:   app/src/firebaseBackend.ts:393-416
  Problem: Every request a user has ever asked, received or had moved is downloaded at each
           start and re-processed on every tick. Start-up gets slower and Firestore reads grow
           without bound.
  Fix:     Add orderBy(updatedAt) and limit(), or filter out old finished requests.

[LOW] 1.9 /read treats any "@" in the question as a mention
  Where:   app/src/commands.ts:178-200
  Problem: `/read what sends mail to ops@example.com` takes "example.com" as the target, and
           `/read @bob what does @Injectable do` is rejected as asking two people. MENTION also
           matches in the middle of a word.
  Fix:     Count a mention only at the start or after whitespace, and only the first one as the
           target. Leave later "@" text in the question.

[LOW] 1.10 Old unpacked binaries are never deleted
  Where:   app/scripts/sea-entry.cjs:243-260
  Problem: Each version unpacks its bundle into ~/.crewai/bin/<hash>/. Old folders are never
           removed, so disk use grows with every upgrade. The demo's temporary LocalStore
           folders (app/src/demo.ts:29,35) are not removed either.


2. SECURITY AND PERMISSIONS (firestore.rules and agent execution)
=================================================================

[HIGH] 2.1 Invite codes can be guessed, burned or exhausted by anyone
  Where:   app/firestore.rules:30-45, app/src/firebaseBackend.ts:188-208
  Problem: The API key is built into every binary, and anyone can create an email/password
           device account, so "signedIn()" means anyone.
           (a) `allow get` on /invites/{code} lets anyone look up all 1,000,000 codes. A live
               code (10-minute window) reveals the teamId and can be claimed before the
               intended person claims it.
           (b) `allow update` lets any signed-in user set used=true on any live invite without
               joining, which blocks real joins.
           (c) Invites are never deleted, and an expired code can never be reused, because
               setDoc on an existing document counts as an update and is denied. Anyone can
               create a team, become its manager, and create invites in a loop until the code
               space is used up for every team. Codes also come from Math.random.
  Fix:     Do the join in a Cloud Function or another trusted step, or at least use longer
           random codes from crypto. Remove the open update rule (let the member-create
           transaction mark the invite as used). Delete or reuse expired invites with a TTL
           policy. Turn on App Check and rate limits.

[MED] 2.2 Anyone can reserve names in any team
  Where:   app/firestore.rules:59-66
  Problem: /teams/{t}/names/{n} accepts a create from any signed-in user, not only someone who
           is joining with a valid invite in the same batch. Once a teamId is known (see 2.1a),
           an attacker can reserve names such as "bob", and the real Bob then cannot join
           ("That name may already be taken"). Names are never released.
  Fix:     Require that the matching member document is created in the same write
           (existsAfter(memberPath(teamId, request.auth.uid))).

[HIGH] 2.3 No way to remove a member or leave a team, so there is no revocation
  Where:   app/firestore.rules:68-92 (members have no delete rule); no command in the app
  Problem: A person who leaves the company keeps full read access to team broadcasts and tasks
           and can keep asking questions forever. A user who reinstalls or moves to a new laptop
           gets a new uid. Their old member entry stays as a ghost, and their name stays
           reserved, so they cannot rejoin with the same name. AGENTS.md lists revocation as a
           protection that must not be weakened.
  Fix:     Add manager-only member removal (delete the member and name documents) and
           `/team leave`. Consider "Always allow" lists that expire.

[HIGH] 2.4 A teammate's question can read any file in the repository, including secrets, and the answer goes to Firestore
  Where:   app/src/agents.ts:23-37, app/src/engine.ts:291-295
  Problem: The question goes word for word into the agent prompt, and the agent can read every
           file in the folder: .env, credentials, keys. A teammate who is always allowed (or any
           teammate who gets one "Allow once") can ask "print the contents of .env". Up to 8000
           characters of the answer are stored in Firestore and kept indefinitely. Claude runs
           with `--allowedTools Read,Grep,Glob`, but the user's own Claude settings (allow
           rules, bypassPermissions) still apply, so it is not certain that only reading is
           possible. The consent card does say "Limited protection", but nothing filters out
           secrets, and "Always allow" never expires. AGENTS.md says not to expose "secrets, full
           repository contents".
  Fix:     Remove likely secrets from answers before sending them (scan for keys, tokens and
           .env lines), pass `--disallowedTools Bash,Edit,Write,WebFetch` and
           `--permission-mode plan` (or the equivalent) for Claude, show the question with a
           clear warning, and make "Always allow" expire.


3. END-USER EXPERIENCE PROBLEMS
===============================

[HIGH] 3.1 Pressing Enter on an empty message box approves a teammate's question
  Where:   app/src/ui/Chat.tsx:53-62, 37-39
  Problem: When a consent card is on top and the message box is empty, Enter runs the selected
           option, and the selection starts at index 0, "Allow once". Pressing Enter in an empty
           box is a common habit, and the card can appear at any moment, so a user can let
           another person's agent query their repository by accident. The same key press on a
           "waiting" card moves the user's own question to another teammate.
  Fix:     Do not select a consent option until the user chooses one with an arrow key (or make
           Decline the default), and require a dedicated key such as 'y' to allow.

[HIGH] 3.2 Being offline at start-up sends the user back to first-run setup
  Where:   app/src/ui/App.tsx:176-185
  Problem: If getTeam() fails for any reason, including no network or Firestore being
           unavailable, the user sees the "first run" onboarding again. If they then create a
           team or enter a code, settings.teamId is overwritten, and they lose their real team
           on this machine.
  Fix:     Tell 'unavailable' apart from 'not-found'/'permission-denied'. For 'unavailable',
           show "Can't reach the team service, retrying…" and keep the saved team.

[MED] 3.3 Answers that arrive while crewai is closed are lost if the user starts a new chat
  Where:   app/src/ui/App.tsx:155-161, app/src/ui/Resume.tsx:325-330
  Problem: A new chat replays only events after "now". If a teammate answered your question
           while crewai was closed, the answer appears only when you resume the exact old chat.
           The Resume screen counts approvals and new tasks, but not answers that are waiting.
  Fix:     Show answers received since the last run that have not been seen, whichever chat is
           opened, and list them on the Resume screen.

[MED] 3.4 On a new Mac, the installer's PATH change has no effect
  Where:   tools/install-crewai.sh:84-93
  Problem: macOS uses zsh by default, and zsh does not read ~/.profile. The script writes to
           ~/.zshrc only if that file already exists, and it does not exist on a new Mac. The
           installer still prints "Added … to your PATH", but `crewai` is "command not found" in
           a new terminal. fish users are also not handled.
  Fix:     On macOS (or when $SHELL ends in zsh), always create or append to ~/.zshrc (or
           ~/.zprofile).

[MED] 3.5 A release can ship binaries that cannot connect to Firebase
  Where:   .github/workflows/crewai-release.yml:107-112, app/scripts/build-binary.mjs:172-173
  Problem: If the repository variables are missing, the build only logs "No Firebase config
           set" and still publishes. Every user who installs that release gets "Firebase is not
           set up yet…" on first run, with instructions that refer to app/.env.example, which is
           not in the binary.
  Fix:     For tag builds, fail when CREWAI_FIREBASE_PROJECT_ID/API_KEY are empty, and make the
           smoke test check that the config is embedded.

[LOW] 3.6 Esc deletes the whole message
  Where:   app/src/ui/Composer.tsx:226-230
  Problem: Pressing Esc to close the "/" or "@" menu also clears everything typed so far.
  Fix:     When a menu is open, Esc closes it only. Clear the text only when no menu is open
           (or not at all).

[LOW] 3.7 The two installers choose different releases
  Where:   tools/install-crewai.sh:46-47 vs tools/install-crewai.ps1:28
  Problem: The PowerShell installer skips pre-releases and drafts. The shell installer takes the
           first crewai-v* tag the API returns, including pre-releases, so macOS/Linux and
           Windows users can end up on different versions.

[LOW] 3.8 Misleading or missing messages
  - `/task @bob` with no text says "Start with who it is for", but the name was given; the
    missing part is the task text (app/src/commands.ts:203-204).
  - ago() can print "60m ago" and "48h ago" because of rounding (app/src/format.ts:260-268).
  - Task status changes that happen within a single chat are logged once per status key
    (`ts:<id>:<status>`), so in_progress -> needs_help -> in_progress shows the second "started"
    only once (app/src/engine.ts:231-236).
  - The team panel says "Only managers invite", but there is no way to promote a member, and
    only the creator is ever a manager.


4. SUGGESTED ORDER OF WORK
==========================
1. 3.1 (accidental consent) and 2.4 (secrets in answers): small changes that protect user data.
2. 2.1, 2.2, 2.3: tighten firestore.rules and add member removal, with tests in
   firebaseBackend.rules.test.ts for the negative cases (guessing, burning, reserving names).
3. 1.1 and 1.2: reliability of the answering side (Windows process tree, single responder).
4. 1.3 and 1.4: server timestamps and a seen-key set that survives trimming.
5. 3.2, 3.4, 3.5: first-run and install problems.
6. Everything else.


5. FIX STATUS (2026-11-23, local changes only, nothing committed or pushed)
=========================================================================
Every item above was addressed in the working tree. Verification:
  cd app; npx tsc --noEmit                 -> clean
  cd app; npx vitest run                   -> 78 tests pass (was 58)
  cd app; npm run emulators                -> 21 contract + rules tests pass against the real rules
  sh -n tools/install-crewai.sh            -> syntax OK; release picker checked on sample API output

1.1  FIXED   agents.ts: Windows kills the whole tree (taskkill /T /F), and a run always settles 3 s
             after a kill even if the process ignores it. Test: a never-ending child times out.
1.2  FIXED   config.ts/engine.ts: responder.lock lets one window per laptop answer questions and report
             presence; other windows say so and take over when it closes (a dead process's lock is reclaimed).
1.3  FIXED   All event times are server timestamps (rules require == request.time). The app estimates the
             server clock from the sign-in token and uses it for "since", presence and invite expiry.
1.4  FIXED   appendEntry keeps a "floor" (newest trimmed time) and never re-adds anything at or before it.
1.5  FIXED   Task board selection follows the task id, the list scrolls (12 rows shown), and setting the
             status a task already has does nothing (no repeated "done" broadcast).
1.6  FIXED   interpretRun(): non-zero exit = failure unless Codex wrote its answer file; the failure shows
             a short stderr excerpt.
1.7  FIXED   createTasks is one transaction for all assignees (max 8); the rules accept a taskSeq jump of
             up to 8 and numbers within the reserved range.
1.8  FIXED   Request subscriptions are ordered by updatedAt and limited to 200 each; indexes added to
             firestore.indexes.json.
1.9  FIXED   A mention must start a word, so emails are kept in the question.
1.10 FIXED   The binary removes older unpacked bundles; the demo removes its temporary folders.
2.1  MITIGATED  Reading an invite needs a join ticket for that code (one try per 2 s per account);
             claiming (used=true) is only possible in the same write that adds you to the team; used or
             expired codes can be reissued; codes come from crypto.randomInt. Limit: many accounts can
             still guess in parallel; Firebase App Check is still recommended before a public launch.
2.2  FIXED   Names can only be reserved together with the matching membership, and are released with it.
2.3  FIXED   /team remove @name (managers), /team promote @name (managers), /team leave. The owner cannot
             be removed or leave. A removed member loses all access and their name becomes free.
2.4  MITIGATED  Answers to teammates pass through redactSecrets() (keys, tokens, private keys, URL
             passwords, NAME=secret assignments); the prompt tells the agent not to reveal secrets; Claude
             runs with --permission-mode default and shell/edit/web tools denied; "always allow" now lasts
             7 days (old settings are migrated). Redaction is best effort and the card says so.
3.1  FIXED   Cards start with no option selected; Enter does nothing until an option is picked with ← →.
3.2  FIXED   If the service cannot be reached at start-up, crewai shows "Retrying…" and keeps the saved
             team; only "not a member" leads to onboarding.
3.3  FIXED   Answers to my questions that no chat has shown yet appear in whichever chat is opened, once;
             the Resume screen counts them.
3.4  FIXED   install-crewai.sh writes ~/.zshrc on macOS or when $SHELL is zsh, and fish config for fish.
3.5  FIXED   Tag builds fail without the Firebase variables; the smoke test checks `crewai --config-status`
             reports "embedded"; the setup hint no longer points at a repo file.
3.6  FIXED   Esc closes an open menu and keeps the text (a lone "/" is cleared); with no menu it clears.
3.7  FIXED   The shell installer skips drafts and pre-releases, like the PowerShell installer.
3.8  FIXED   Clear /task message; ago() rounds down; a status that comes back is logged again; managers can
             promote others.

Before releasing these changes:
  - Deploy the new rules AND indexes together with the new app version
    (npx firebase deploy --only firestore --project <id>). Older app versions write client timestamps,
    which the new rules reject, so everyone needs the new binary; the new app also needs the new rules.
  - Not tested by hand in a real terminal or on macOS/Linux; the installers were checked only for syntax
    and the release picker on sample data.
