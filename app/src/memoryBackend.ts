import { randomInt, randomUUID } from 'node:crypto';
import { BackendError, type Backend, type Unsubscribe } from './backend.js';
import {
  INVITE_TTL_MS,
  validName,
  LIMITS,
  type Broadcast,
  type Invite,
  type Member,
  type PresenceInfo,
  type ReadRequest,
  type Task,
  type TaskStatus,
  type Team,
} from './types.js';

interface TeamRow extends Team {
  taskSeq: number;
}
interface InviteRow {
  teamId: string;
  expiresAt: number;
  used: boolean;
  createdBy: string;
}

/**
 * Shared in-memory "cloud" for tests and demos. It enforces the same rules the
 * Firestore security rules enforce, so flow tests catch permission mistakes.
 */
export class MemoryStore {
  teams = new Map<string, TeamRow>();
  members = new Map<string, Map<string, Member>>();
  invites = new Map<string, InviteRow>();
  broadcasts = new Map<string, Broadcast[]>();
  tasks = new Map<string, Task[]>();
  requests = new Map<string, ReadRequest[]>();
  private listeners = new Set<() => void>();

  constructor(public now: () => number = Date.now) {}

  listen(fn: () => void): Unsubscribe {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }
  changed(): void {
    for (const l of [...this.listeners]) l();
  }
  backendFor(uid: string): Backend {
    return new MemoryBackend(this, uid);
  }
}

const checkName = (s: string, what: string): string => {
  const t = s.trim();
  if (!validName(t)) throw new BackendError('invalid', `${what} can use letters, numbers, dots, dashes and underscores (no spaces).`);
  return t;
};

const bounded = (s: string, max: number, what: string): string => {
  const t = s.trim();
  if (!t) throw new BackendError('invalid', `${what} can't be empty.`);
  if (t.length > max) throw new BackendError('invalid', `${what} is too long.`);
  return t;
};

class MemoryBackend implements Backend {
  constructor(
    private s: MemoryStore,
    public readonly uid: string,
  ) {}

  private member(teamId: string, uid = this.uid): Member | undefined {
    return this.s.members.get(teamId)?.get(uid);
  }
  private requireMember(teamId: string): Member {
    const m = this.member(teamId);
    if (!m) throw new BackendError('permission-denied', 'You are not in this team.');
    return m;
  }
  private nameTaken(teamId: string, name: string): boolean {
    return [...(this.s.members.get(teamId)?.values() ?? [])].some(
      (m) => m.name.toLowerCase() === name.toLowerCase() && m.uid !== this.uid,
    );
  }

  async createTeam(teamName: string, displayName: string): Promise<Team> {
    const name = bounded(teamName, LIMITS.name, 'Team name');
    const me = checkName(displayName, 'Your name');
    const id = randomUUID();
    const now = this.s.now();
    const team: TeamRow = { id, name, ownerUid: this.uid, createdAt: now, taskSeq: 0 };
    this.s.teams.set(id, team);
    this.s.members.set(
      id,
      new Map([[this.uid, { uid: this.uid, name: me, role: 'manager', lastSeen: now, agent: null, project: '', joinedAt: now }]]),
    );
    this.s.changed();
    return { id, name, ownerUid: this.uid, createdAt: now };
  }

  async createInvite(teamId: string): Promise<Invite> {
    const me = this.requireMember(teamId);
    if (me.role !== 'manager') throw new BackendError('permission-denied', 'Only a manager can invite people.');
    let code = '';
    do {
      code = String(randomInt(0, 1_000_000)).padStart(6, '0');
    } while (this.s.invites.has(code) && this.s.invites.get(code)!.expiresAt > this.s.now());
    const expiresAt = this.s.now() + INVITE_TTL_MS;
    this.s.invites.set(code, { teamId, expiresAt, used: false, createdBy: this.uid });
    this.s.changed();
    return { code, teamId, expiresAt };
  }

  async joinTeam(code: string, displayName: string): Promise<Team> {
    const me = checkName(displayName, 'Your name');
    const inv = this.s.invites.get(code.trim());
    if (!inv) throw new BackendError('not-found', "That code isn't valid.");
    if (inv.used) throw new BackendError('conflict', 'That code was already used.');
    if (inv.expiresAt <= this.s.now()) throw new BackendError('expired', 'That code has expired. Ask for a new one.');
    if (this.member(inv.teamId)) throw new BackendError('conflict', "You're already in this team.");
    if (this.nameTaken(inv.teamId, me)) throw new BackendError('conflict', 'That name is taken in this team. Pick another.');
    inv.used = true;
    const now = this.s.now();
    this.s.members.get(inv.teamId)!.set(this.uid, {
      uid: this.uid, name: me, role: 'member', lastSeen: now, agent: null, project: '', joinedAt: now,
    });
    this.s.changed();
    const t = this.s.teams.get(inv.teamId)!;
    return { id: t.id, name: t.name, ownerUid: t.ownerUid, createdAt: t.createdAt };
  }

  async getTeam(teamId: string): Promise<Team | null> {
    this.requireMember(teamId);
    const t = this.s.teams.get(teamId);
    return t ? { id: t.id, name: t.name, ownerUid: t.ownerUid, createdAt: t.createdAt } : null;
  }

  async heartbeat(teamId: string, info: PresenceInfo): Promise<void> {
    const me = this.requireMember(teamId);
    me.lastSeen = this.s.now();
    me.agent = info.agent;
    me.project = info.project.slice(0, 120);
    this.s.changed();
  }

  private sub<T>(teamId: string, read: () => T, cb: (v: T) => void): Unsubscribe {
    let last = '';
    const push = () => {
      if (!this.member(teamId)) return;
      const v = read();
      const j = JSON.stringify(v);
      if (j === last) return;
      last = j;
      cb(structuredClone(v));
    };
    push();
    return this.s.listen(push);
  }

  subscribeMembers(teamId: string, cb: (m: Member[]) => void): Unsubscribe {
    return this.sub(teamId, () => [...(this.s.members.get(teamId)?.values() ?? [])], cb);
  }

  async sendBroadcast(teamId: string, text: string): Promise<Broadcast> {
    const me = this.requireMember(teamId);
    const b: Broadcast = {
      id: randomUUID(), fromUid: this.uid, fromName: me.name,
      text: bounded(text, LIMITS.text, 'Message'), createdAt: this.s.now(),
    };
    const list = this.s.broadcasts.get(teamId) ?? [];
    list.push(b);
    this.s.broadcasts.set(teamId, list);
    this.s.changed();
    return b;
  }
  subscribeBroadcasts(teamId: string, cb: (b: Broadcast[]) => void): Unsubscribe {
    return this.sub(teamId, () => this.s.broadcasts.get(teamId) ?? [], cb);
  }

  async createTasks(
    teamId: string,
    items: { assigneeUid: string; assigneeName: string; text: string }[],
  ): Promise<Task[]> {
    const me = this.requireMember(teamId);
    const out: Task[] = [];
    for (const it of items) {
      if (!this.member(teamId, it.assigneeUid)) throw new BackendError('invalid', `${it.assigneeName} is not in this team.`);
      const team = this.s.teams.get(teamId)!;
      team.taskSeq += 1;
      const now = this.s.now();
      out.push({
        id: randomUUID(), num: team.taskSeq,
        assigneeUid: it.assigneeUid, assigneeName: it.assigneeName,
        assignerUid: this.uid, assignerName: me.name,
        text: bounded(it.text, LIMITS.text, 'Task'),
        status: 'todo', note: '', createdAt: now, updatedAt: now,
      });
    }
    const list = this.s.tasks.get(teamId) ?? [];
    list.push(...out);
    this.s.tasks.set(teamId, list);
    this.s.changed();
    return structuredClone(out);
  }

  async updateTask(teamId: string, taskId: string, patch: { status: TaskStatus; note?: string }): Promise<void> {
    const me = this.requireMember(teamId);
    const t = this.s.tasks.get(teamId)?.find((x) => x.id === taskId);
    if (!t) throw new BackendError('not-found', 'Task not found.');
    const allowed = t.assigneeUid === this.uid || t.assignerUid === this.uid || me.role === 'manager';
    if (!allowed) throw new BackendError('permission-denied', "You can't change this task.");
    t.status = patch.status;
    if (patch.note !== undefined) t.note = patch.note.slice(0, LIMITS.error);
    t.updatedAt = this.s.now();
    this.s.changed();
  }
  subscribeTasks(teamId: string, cb: (t: Task[]) => void): Unsubscribe {
    return this.sub(teamId, () => this.s.tasks.get(teamId) ?? [], cb);
  }

  async createRequest(
    teamId: string,
    input: { targetUid: string; targetName: string; question: string },
  ): Promise<ReadRequest> {
    const me = this.requireMember(teamId);
    if (!this.member(teamId, input.targetUid)) throw new BackendError('invalid', `${input.targetName} is not in this team.`);
    const now = this.s.now();
    const r: ReadRequest = {
      id: randomUUID(), askerUid: this.uid, askerName: me.name,
      targetUid: input.targetUid, targetName: input.targetName,
      question: bounded(input.question, LIMITS.text, 'Question'),
      status: 'pending', answer: '', agent: '', error: '', movedFrom: [], createdAt: now, updatedAt: now,
    };
    const list = this.s.requests.get(teamId) ?? [];
    list.push(r);
    this.s.requests.set(teamId, list);
    this.s.changed();
    return structuredClone(r);
  }

  private req(teamId: string, id: string): ReadRequest {
    this.requireMember(teamId);
    const r = this.s.requests.get(teamId)?.find((x) => x.id === id);
    if (!r) throw new BackendError('not-found', 'Question not found.');
    return r;
  }

  async rerouteRequest(teamId: string, id: string, target: { uid: string; name: string }): Promise<void> {
    const r = this.req(teamId, id);
    if (r.askerUid !== this.uid) throw new BackendError('permission-denied', 'Only the asker can move a question.');
    if (r.status !== 'pending') throw new BackendError('conflict', "That question is already being answered.");
    if (!this.member(teamId, target.uid)) throw new BackendError('invalid', `${target.name} is not in this team.`);
    if (target.uid === r.targetUid) return;
    r.movedFrom = [...r.movedFrom, r.targetUid];
    r.targetUid = target.uid;
    r.targetName = target.name;
    r.updatedAt = this.s.now();
    this.s.changed();
  }

  async cancelRequest(teamId: string, id: string): Promise<void> {
    const r = this.req(teamId, id);
    if (r.askerUid !== this.uid) throw new BackendError('permission-denied', 'Only the asker can cancel.');
    if (r.status !== 'pending' && r.status !== 'running') throw new BackendError('conflict', 'That question is already finished.');
    r.status = 'cancelled';
    r.updatedAt = this.s.now();
    this.s.changed();
  }

  async respondRequest(
    teamId: string,
    id: string,
    patch: { status: 'running' | 'answered' | 'declined' | 'failed'; answer?: string; agent?: string; error?: string },
  ): Promise<void> {
    const r = this.req(teamId, id);
    if (r.targetUid !== this.uid) throw new BackendError('permission-denied', "This question isn't yours to answer.");
    if (r.status !== 'pending' && r.status !== 'running') throw new BackendError('conflict', 'That question is no longer open.');
    if (patch.status === 'running' && r.status !== 'pending') throw new BackendError('conflict', 'Already running.');
    r.status = patch.status;
    if (patch.answer !== undefined) r.answer = patch.answer.slice(0, LIMITS.answer);
    if (patch.agent !== undefined) r.agent = patch.agent.slice(0, 20);
    if (patch.error !== undefined) r.error = patch.error.slice(0, LIMITS.error);
    r.updatedAt = this.s.now();
    this.s.changed();
  }

  subscribeRequests(teamId: string, cb: (r: ReadRequest[]) => void): Unsubscribe {
    return this.sub(
      teamId,
      () =>
        (this.s.requests.get(teamId) ?? []).filter(
          (r) => r.askerUid === this.uid || r.targetUid === this.uid || r.movedFrom.includes(this.uid),
        ),
      cb,
    );
  }

  async close(): Promise<void> {}
}
