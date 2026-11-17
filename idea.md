A terminal-first tool that connects managers, team leads, and implementers to their local coding agents and repositories. LAN peer sync is available, and an optional Internet relay lets teammates collaborate when they are on separate networks.
Team and permissions
A manager creates a team and adds members with roles, such as implementers, part heads, and group heads. Each implementer approves access to their local workspace.

Permissions inherit transitively. For example, if Ravi inherits all of the manager’s permissions, Ravi receives the same access the manager has to the implementers, including the ability to ask questions and assign tasks. The permission chain should be visible to team members.
Commands
- /read <question>
  Starts a separate Codex session on this terminal to answer a question about this device's locally approved repository. It uses Codex's read-only sandbox mode and does not use the team relay.
- /check @person <question>
  Starts a separate, read-only Codex session to inspect the target person’s local repo and answer a natural-language question. Access follows the permission rules.
- /task @person <task>
  Sends a task to the person’s terminal inbox. They can mark it in progress or done, or send a follow-up. Authorized managers and leads can view and update the task.
- /broadcast <message>
  Sends a message to the team or a selected group. Anyone can broadcast.
Offline behavior and synchronization
Every teammate’s laptop stores its own local copy of team messages, tasks, permissions, and updates. Laptops sync directly over LAN or through the optional HTTPS relay across the Internet.

If a command targets someone who is offline, it stays in the local event log until the devices sync. The receiver currently runs `herdr relay sync` to fetch new Internet events. `/check` remains a separate remote repository request with its existing execution boundary. `/read` runs on the current device and asks about its own approved local repository.

Updates created while someone is offline sync when they reconnect. The design should use unique event IDs and ordering metadata so copies can converge, rather than relying only on laptop timestamps.
Initial scope
Build a terminal-first version for a small team. Each member runs the app on their laptop, links their local repo, and explicitly approves the access granted to the team. LAN peer sync remains supported; Internet sync uses an optional Cloudflare relay so teammates do not need to share a network. Repository contents stay on their owner's laptop. The core workflow is assigning tasks, broadcasting messages, and using `/read` to start a separate Codex session on this terminal to answer a local query.




