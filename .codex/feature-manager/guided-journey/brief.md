# Guided terminal team journey

User approved implementing the researched journey: launch -> Create/Join -> name -> optional connect project/assistant -> team home -> first message/task/question. Preserve terminal/Spaces/Agents and old databases. No IDs or relay variables should be required in normal UI. App restart restores connections. Joining another team must keep the previous team's data separate. Membership acceptance and repository consent remain separate. Named actions are primary; shortcuts optional. Show truthful delivery states and full bounded results. Local assistant uses a separate process, and limited remote isolation requires informed explicit confirmation.

Root owns UI/state/rendering/integration. Independent workers use isolated worktrees for (a) relay/task authorization and convergence, (b) remembered connections/profile/domain onboarding/switching, (c) question permissions/native assistant execution. Do not edit root UI files in worker worktrees. Root merges, verifies and documents actual results. No personal runtime databases are test targets.

Known baseline: v0.1.9; full suite has unrelated failures and stalls; focused suites/build and isolated network smoke passed. User untracked aws_guide.txt, plan1.jpeg, website/ are unrelated. Custom fork Hemant9785/crewAI; do not submit upstream issues/PRs.
