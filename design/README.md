# Approved crewAI terminal design

Approved by the product owner on 2026-11-22. These are visual references for the next implementation, not screenshots of shipped functionality. Generated with the built-in image-generation tool; copied here unchanged.

## Main conversation

[Open conversation reference](crewai-conversation-reference.png)

`/crewai` opens a conversational interface inside the active Herdr terminal pane. Keep Spaces and Agents available. Questions, assistant answers, human messages, broadcasts, task updates and their status appear in the conversation. Keep one composer at the bottom and a scrollable transcript above it.

## Commands and inline interactions

[Open command-flow reference](crewai-command-flow-reference.png)

- `/` opens a discoverable command menu.
- `@` opens a teammate picker; arrow keys and Enter select a person.
- `/team-mates` shows people and real team/invitation actions in the conversation.
- Repository-owner approval appears beside the incoming question, with visible choices and an accurate isolation warning.
- Commands: `/read`, `/team-mates`, `/task`, `/broadcast`, `/message`.

Example:

```text
/read what is this project about @hemant
crewAI: This project is a dating app.
/message @hemant what is the status of the task?
/broadcast checkout this source
/task @hemant add a search filter
```

The dating-app description, names, invite digits, agent labels and counts are illustrative content. They must be replaced by actual records and real assistant output. Human replies must remain distinct from assistant-generated project answers. A direct message must have recipient-only access, not simply be hidden from other users by the UI.

The active implementation plan and acceptance checklist is [task.md](../task.md). Preserve the trust and execution boundaries in [spec.md](../spec.md); update its UX contract before implementation. The tab drawn in the reference identifies the active terminal session and does not require launching a separate window or replacing other terminal tabs.
