A terminal-first tool that connects managers, team leads, and implementers to their local coding agents and repositories—without a central server. It is designed initially for teammates on the same office network.
Team and permissions
A manager creates a team and adds members with roles, such as implementers, part heads, and group heads. Each implementer approves access to their local workspace.

Permissions inherit transitively. For example, if Ravi inherits all of the manager’s permissions, Ravi receives the same access the manager has to the implementers, including the ability to ask questions and assign tasks. The permission chain should be visible to team members.
Commands
- /read @person <prompt> <source link>
  Planned later command. It starts a separate Codex session on the target person’s computer to summarize a source and explain how it may relate to the local repo. It requires explicit network, content, permission, and read-only execution controls before it is enabled.
- /check @person <question>
  Starts a separate, read-only Codex session to inspect the target person’s local repo and answer a natural-language question. Access follows the permission rules.
- /task @person <task>
  Sends a task to the person’s terminal inbox. They can mark it in progress or done, or send a follow-up. Authorized managers and leads can view and update the task.
- /broadcast <message>
  Sends a message to the team or a selected group. Anyone can broadcast.
Offline behavior and synchronization
Every teammate’s laptop stores its own local copy of team messages, tasks, permissions, and updates. Laptops sync directly when teammates are online on the office network.

If a command targets someone who is offline, it waits in a queue. When the target and sender are both online, the request can run on the target’s computer. /check, when enabled, inspects the target’s local repo using a separate Codex session and must report whether read-only execution is technically enforced. It must not modify files or interrupt the person’s active Codex session. /read is deferred until its additional source and security controls are defined.

Updates created while someone is offline sync when they reconnect. The design should use unique event IDs and ordering metadata so copies can converge, rather than relying only on laptop timestamps.
Initial scope
Build a terminal-first version for a small team on the same office network. Each member runs the app on their laptop, links their local repo, and explicitly approves the access granted to the team. The first version should focus on permission-aware commands, task inboxes, queued requests, direct laptop-to-laptop sync, and read-only repo questions.




