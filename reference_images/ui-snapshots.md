# Agreed UI snapshots (text)

Text versions of the six terminal screens agreed in chat on 2026-11-23.

## 1. First-run onboarding
```
crewai  first run
────────────────────────────────────────────────────
 Welcome. Four quick steps, about a minute.

 ✓ 1  Sign in   sonu@acme.com
 ● 2  Team
     › Join a team     I have a 6-digit code
       Create a team   I'm the manager
 ○ 3  Project   this folder: ~/code/web-app
 ○ 4  Agent     found: codex, claude
```

## 2. Resume
```
crewai · Acme · ~/code/web-app
 2 things need you   1 approval   1 new task
 Resume a chat
 › Today 10:42   Hemant: what is the login flow?     12 msgs
   Yesterday     Broadcast: release freeze Friday     3 msgs
   + Start a new chat
 ↑↓ choose   enter resume   n new chat
```

## 3. Main chat
```
crewai · Acme · ~/code/web-app · agent: codex
● you  ● Hemant  ● Shwetank  ○ Ravi (away 2h)
────────────────────────────────────────────────────
 Shwetank 09:12  assigned you a task  ▸ T-15 fix flaky login test
 Hemant 09:30    broadcast  Release freeze starts Friday.
 you 10:41       /read @Hemant what is the login flow?
 ✦ Hemant's codex 10:42
   Login uses a session cookie set in auth/session.ts.
   answered from ~/code/web-app on Hemant's laptop
────────────────────────────────────────────────────
 › /
   /read  /task  /broadcast  /team  /agent
```

## 4. Offline teammate
```
 /read @Hemant what is this project about?
 ┌ Hemant is offline · last seen 2h ago ──────────────┐
 │ Your question is saved.                            │
 │ › Wait for Hemant                                  │
 │   Ask someone online instead  ● Shwetank ● Ravi    │
 │   Cancel                                           │
 └────────────────────────────────────────────────────┘
 Moved to Shwetank. Hemant's copy was cancelled.
 ● Hemant is back. He sees "already handled by Shwetank".
```

## 5. Consent (owner side)
```
 ┌ Shwetank asks about your project ──────────────────┐
 │ "What is the login flow?"                          │
 │ Your claude will answer from ~/code/web-app.       │
 │ Limited protection: it can read files in this      │
 │ folder and use the network. Only the answer is     │
 │ shared.                                            │
 │ › Allow once   Always allow Shwetank   Decline     │
 └────────────────────────────────────────────────────┘
 › /task @Ravi review the PR for the nav bar   draft kept
```

## 6. Tasks
```
 /task @Hemant @Shwetank implement submit button
 ✓ Created 2 tasks
   T-16 Hemant    ○ waiting, offline
   T-17 Shwetank  ● delivered
 Tasks · To do / In progress / Done
 ↑↓ choose   enter open   s start   d done   h need help
```
