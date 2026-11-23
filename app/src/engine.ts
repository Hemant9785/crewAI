import { basename } from 'node:path';
import { pickAgent, type AgentRunner } from './agents.js';
import { BackendError, type Backend, type Unsubscribe } from './backend.js';
import { COMMANDS, parseInput, resolveMention } from './commands.js';
import { appendEntry, type Entry, type EntryKind, type LocalStore, type Session } from './config.js';
import { ago, clip, formatCode } from './format.js';
import {
  HEARTBEAT_MS,
  isOnline,
  type AgentName,
  type AgentPref,
  type Broadcast,
  type Member,
  type ReadRequest,
  type Task,
  type TaskStatus,
  type Team,
} from './types.js';

export type Attention =
  | { type: 'consent'; request: ReadRequest }
  | {
      type: 'pending';
      request: ReadRequest;
      targetOnline: boolean;
      /** Teammates who are online and have an agent, other than the current target. */
      alternatives: Member[];
      /** True once the asker chose to wait; shown as a quiet one-line status. */
      compact: boolean;
    };

export interface EngineState {
  ready: boolean;
  team: Team | null;
  me: Member | null;
  members: Member[];
  tasks: Task[];
  broadcasts: Broadcast[];
  requests: ReadRequest[];
  entries: Entry[];
  agent: AgentName | null;
  available: AgentName[];
  pref: AgentPref;
  attention: Attention[];
  busy: string;
  now: number;
  error: string;
}

/** `keepDraft` tells the UI to put the text back in the message box because nothing was sent. */
export type Outcome = { action?: 'quit' | 'new' | 'tasks'; keepDraft?: boolean };

export interface EngineDeps {
  backend: Backend;
  store: LocalStore;
  runner: AgentRunner;
  folder: string;
  now?: () => number;
  /** 0 disables the timers (tests call tick() themselves). */
  tickMs?: number;
  heartbeatMs?: number;
}

const STATUS_WORD: Record<TaskStatus, string> = {
  todo: 'not started', in_progress: 'started', done: 'done', cancelled: 'cancelled', needs_help: 'needs help',
};

export class Engine {
  private s: EngineState;
  private listeners = new Set<() => void>();
  private unsubs: Unsubscribe[] = [];
  private timers: NodeJS.Timeout[] = [];
  private session: Session | null = null;
  private since = 0;
  private handled = new Set<string>();
  private dismissed = new Set<string>();
  private queue: Promise<void> = Promise.resolve();
  private lastDetect = 0;
  private counter = 0;
  private stopped = false;

  constructor(private d: EngineDeps) {
    this.s = {
      ready: false, team: null, me: null, members: [], tasks: [], broadcasts: [], requests: [], entries: [],
      agent: null, available: [], pref: d.store.settings().agent, attention: [], busy: '', now: this.now(), error: '',
    };
  }

  private now(): number {
    return this.d.now?.() ?? Date.now();
  }
  get state(): EngineState {
    return this.s;
  }
  subscribe(fn: () => void): Unsubscribe {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }
  private emit(patch: Partial<EngineState> = {}): void {
    this.s = { ...this.s, ...patch, now: this.now() };
    for (const l of [...this.listeners]) l();
  }

  // ───────────────────────── lifecycle

  async start(team: Team): Promise<void> {
    const b = this.d.backend;
    b.onError = (e) => this.emit({ error: e.message });
    this.s = { ...this.s, team };
    await this.refreshAgents(true);
    await this.heartbeat();
    const raw: { members: Member[]; tasks: Task[]; broadcasts: Broadcast[]; requests: ReadRequest[] } = {
      members: [], tasks: [], broadcasts: [], requests: [],
    };
    const upd = <K extends keyof typeof raw>(k: K) => (v: (typeof raw)[K]) => {
      raw[k] = v;
      this.ingest(raw);
    };
    this.unsubs.push(
      b.subscribeMembers(team.id, upd('members')),
      b.subscribeBroadcasts(team.id, upd('broadcasts')),
      b.subscribeTasks(team.id, upd('tasks')),
      b.subscribeRequests(team.id, upd('requests')),
    );
    const tickMs = this.d.tickMs ?? 5000;
    const beatMs = this.d.heartbeatMs ?? HEARTBEAT_MS;
    if (tickMs > 0) this.timers.push(setInterval(() => void this.tick(), tickMs));
    if (beatMs > 0) this.timers.push(setInterval(() => void this.heartbeat(), beatMs));
    for (const t of this.timers) t.unref?.();
    this.emit({ ready: true });
  }

  async stop(): Promise<void> {
    this.stopped = true;
    this.unsubs.forEach((u) => u());
    this.timers.forEach(clearInterval);
    this.unsubs = [];
    this.timers = [];
    await this.queue.catch(() => {});
  }

  /** Begin writing to a session. `since` limits which team events are replayed into it. */
  attach(session: Session, since: number): void {
    this.session = session;
    this.since = since;
    this.emit({ entries: [...session.entries] });
    this.recompute();
  }

  async tick(): Promise<void> {
    this.recompute();
  }

  async heartbeat(): Promise<void> {
    const team = this.s.team;
    if (!team || this.stopped) return;
    try {
      await this.d.backend.heartbeat(team.id, { agent: this.s.agent, project: basename(this.d.folder) });
    } catch (e) {
      this.emit({ error: e instanceof BackendError ? e.message : 'Lost connection to the team.' });
    }
  }

  // ───────────────────────── agents

  async refreshAgents(force = false): Promise<void> {
    if (!force && this.now() - this.lastDetect < 30_000) return;
    this.lastDetect = this.now();
    const available = await this.d.runner.detect();
    const pref = this.d.store.settings().agent;
    const { agent, fellBack } = pickAgent(pref, available);
    const changed = agent !== this.s.agent;
    this.emit({ available, agent, pref });
    if (fellBack && changed && this.session) this.log('system', '', `${pref} isn't available here, so crewai is using ${agent}.`, `fb:${pref}:${agent}`);
    if (changed) void this.heartbeat();
  }

  // ───────────────────────── log

  private log(kind: EntryKind, who: string, text: string, key: string, at = this.now(), meta?: string): void {
    const sess = this.session;
    if (!sess) return;
    const e = appendEntry(sess, { key, at, kind, who, text, meta });
    if (!e) return;
    this.d.store.saveSession(sess);
    this.emit({ entries: [...sess.entries] });
  }
  private sys(text: string): void {
    this.log('system', '', text, `s:${this.now()}:${++this.counter}`);
  }

  // ───────────────────────── incoming data

  private ingest(raw: { members: Member[]; tasks: Task[]; broadcasts: Broadcast[]; requests: ReadRequest[] }): void {
    const me = raw.members.find((m) => m.uid === this.d.backend.uid) ?? null;
    this.s = { ...this.s, me, members: raw.members, tasks: raw.tasks, broadcasts: raw.broadcasts, requests: raw.requests };
    this.recompute();
  }

  private recompute(): void {
    if (this.stopped) return;
    const s = this.s;
    const me = s.me;
    if (me && this.session) this.deriveEntries(me);
    if (me) this.handleIncoming(me);
    this.emit({ attention: this.computeAttention() });
  }

  private deriveEntries(me: Member): void {
    const s = this.s;
    const since = this.since;
    const fresh: { at: number; kind: EntryKind; who: string; text: string; key: string; meta?: string }[] = [];

    for (const m of s.members) {
      if (m.uid !== me.uid && m.joinedAt >= since) fresh.push({ at: m.joinedAt, kind: 'system', who: '', text: `${m.name} joined the team.`, key: `m:${m.uid}` });
    }
    for (const b of s.broadcasts) {
      if (b.createdAt >= since) {
        fresh.push({ at: b.createdAt, kind: 'broadcast', who: b.fromUid === me.uid ? 'you' : b.fromName, text: b.text, key: `b:${b.id}` });
      }
    }
    for (const t of s.tasks) {
      const involved = t.assigneeUid === me.uid || t.assignerUid === me.uid;
      if (!involved) continue;
      if (t.createdAt >= since) {
        const text = t.assigneeUid === me.uid
          ? `${t.assignerUid === me.uid ? 'You' : t.assignerName} assigned you T-${t.num}: ${t.text}`
          : `You assigned T-${t.num} to ${t.assigneeName}: ${t.text}`;
        fresh.push({ at: t.createdAt, kind: 'task', who: '', text, key: `tc:${t.id}` });
      }
      if (t.updatedAt >= since && (t.status === 'in_progress' || t.status === 'needs_help' || t.status === 'cancelled')) {
        const who = t.assigneeUid === me.uid ? 'You' : t.assigneeName;
        fresh.push({
          at: t.updatedAt, kind: 'task', who: '',
          text: `${who} ${STATUS_WORD[t.status]} T-${t.num}${t.note ? ` — ${t.note}` : ''}`, key: `ts:${t.id}:${t.status}`,
        });
      }
    }
    for (const r of s.requests) {
      if (r.askerUid === me.uid) {
        if (r.status === 'answered' && r.updatedAt >= since) {
          fresh.push({
            at: r.updatedAt, kind: 'agent', who: `${r.targetName}'s ${r.agent || 'agent'}`, text: r.answer, key: `ra:${r.id}`,
            meta: `answered from ${r.targetName}'s laptop`,
          });
        } else if (r.status === 'declined' && r.updatedAt >= since) {
          fresh.push({ at: r.updatedAt, kind: 'system', who: '', text: `${r.targetName} declined your question.`, key: `rd:${r.id}` });
        } else if (r.status === 'failed' && r.updatedAt >= since) {
          fresh.push({
            at: r.updatedAt, kind: 'system', who: '',
            text: `${r.targetName}'s agent couldn't answer${r.error ? `: ${r.error}` : '.'}`, key: `rf:${r.id}`,
          });
        }
      } else if (r.movedFrom.includes(me.uid) && r.targetUid !== me.uid && r.updatedAt >= since) {
        fresh.push({
          at: r.updatedAt, kind: 'system', who: '',
          text: `${r.askerName}'s question was already handled by ${r.targetName}.`, key: `rm:${r.id}`,
        });
      } else if (r.targetUid === me.uid && r.status === 'cancelled' && r.updatedAt >= since) {
        fresh.push({ at: r.updatedAt, kind: 'system', who: '', text: `${r.askerName} cancelled their question.`, key: `rc:${r.id}` });
      }
    }
    fresh.sort((a, b) => a.at - b.at);
    for (const f of fresh) this.log(f.kind, f.who, f.text, f.key, f.at, f.meta);
  }

  // ───────────────────────── incoming questions (this laptop is the target)

  private handleIncoming(me: Member): void {
    const team = this.s.team;
    if (!team) return;
    for (const r of this.s.requests) {
      if (r.targetUid !== me.uid || this.handled.has(r.id)) continue;
      if (r.status === 'running') {
        // Left over from a previous run that was interrupted.
        this.handled.add(r.id);
        void this.d.backend
          .respondRequest(team.id, r.id, { status: 'failed', error: 'Interrupted. Please ask again.' })
          .catch(() => {});
        continue;
      }
      if (r.status !== 'pending') continue;
      if (!this.s.agent) {
        this.handled.add(r.id);
        void this.d.backend
          .respondRequest(team.id, r.id, { status: 'failed', error: `${me.name} has no coding agent installed (Codex or Claude).` })
          .catch(() => {});
        this.sys(`${r.askerName} asked about your project, but no coding agent is installed.`);
        continue;
      }
      if (this.d.store.isAlwaysAllowed(team.id, r.askerUid)) {
        this.handled.add(r.id);
        this.sys(`${r.askerName} asked about your project (always allowed).`);
        this.enqueue(r);
      }
    }
  }

  private enqueue(r: ReadRequest): void {
    this.queue = this.queue.then(() => this.runRequest(r)).catch(() => {});
  }

  private async runRequest(r: ReadRequest): Promise<void> {
    const team = this.s.team;
    if (!team || this.stopped) return;
    try {
      await this.refreshAgents();
      const agent = this.s.agent;
      if (!agent) {
        await this.d.backend.respondRequest(team.id, r.id, { status: 'failed', error: 'No coding agent is installed.' });
        return;
      }
      await this.d.backend.respondRequest(team.id, r.id, { status: 'running', agent });
      this.emit({ busy: `Answering ${r.askerName}'s question with ${agent}…` });
      const result = await this.d.runner.run(agent, r.question, this.d.folder);
      this.emit({ busy: '' });
      await this.d.backend.respondRequest(
        team.id, r.id,
        result.ok ? { status: 'answered', answer: result.text, agent } : { status: 'failed', error: result.text, agent },
      );
      this.sys(result.ok ? `Answered ${r.askerName}'s question.` : `Couldn't answer ${r.askerName}'s question.`);
    } catch (e) {
      // Cancelled or moved while running: nothing to report back.
      if (!(e instanceof BackendError && (e.code === 'conflict' || e.code === 'permission-denied'))) {
        this.sys("Couldn't send an answer back.");
      }
    } finally {
      this.emit({ busy: '' });
    }
  }

  async decideConsent(requestId: string, choice: 'once' | 'always' | 'decline'): Promise<void> {
    const team = this.s.team;
    const r = this.s.requests.find((x) => x.id === requestId);
    if (!team || !r || this.handled.has(requestId)) return;
    this.handled.add(requestId);
    this.recompute();
    if (choice === 'decline') {
      try {
        await this.d.backend.respondRequest(team.id, requestId, { status: 'declined' });
      } catch {
        /* already cancelled or moved */
      }
      this.sys(`You declined ${r.askerName}'s question.`);
      return;
    }
    if (choice === 'always') {
      this.d.store.allowAlways(team.id, r.askerUid);
      this.sys(`You allowed ${r.askerName}'s question, and will allow future ones from them.`);
    } else {
      this.sys(`You allowed ${r.askerName}'s question once.`);
    }
    this.enqueue(r);
  }

  // ───────────────────────── asker-side cards

  private computeAttention(): Attention[] {
    const s = this.s;
    const me = s.me;
    if (!me) return [];
    const now = this.now();
    const out: Attention[] = [];
    for (const r of s.requests) {
      if (r.targetUid === me.uid && r.status === 'pending' && !this.handled.has(r.id)) {
        out.push({ type: 'consent', request: r });
      }
    }
    for (const r of s.requests) {
      if (r.askerUid !== me.uid || r.status !== 'pending') continue;
      const target = s.members.find((m) => m.uid === r.targetUid);
      const targetOnline = !!target && isOnline(target, now);
      const alternatives = s.members.filter((m) => m.uid !== me.uid && m.uid !== r.targetUid && m.agent !== null && isOnline(m, now));
      out.push({ type: 'pending', request: r, targetOnline, alternatives, compact: this.dismissed.has(r.id) && !targetOnline });
    }
    return out;
  }

  waitFor(requestId: string): void {
    this.dismissed.add(requestId);
    const r = this.s.requests.find((x) => x.id === requestId);
    if (r) this.sys(`Waiting for ${r.targetName}. Your question stays saved.`);
    this.recompute();
  }

  async moveRequest(requestId: string, uid: string): Promise<void> {
    const team = this.s.team;
    const target = this.s.members.find((m) => m.uid === uid);
    const r = this.s.requests.find((x) => x.id === requestId);
    if (!team || !target || !r) return;
    try {
      await this.d.backend.rerouteRequest(team.id, requestId, { uid, name: target.name });
      this.dismissed.delete(requestId);
      this.sys(`Moved your question to ${target.name}. ${r.targetName}'s copy was cancelled.`);
    } catch (e) {
      this.sys(e instanceof BackendError ? e.message : "Couldn't move the question.");
    }
    this.recompute();
  }

  async cancelRequest(requestId: string): Promise<void> {
    const team = this.s.team;
    const r = this.s.requests.find((x) => x.id === requestId);
    if (!team || !r) return;
    try {
      await this.d.backend.cancelRequest(team.id, requestId);
      this.sys(`Cancelled your question to ${r.targetName}.`);
    } catch (e) {
      this.sys(e instanceof BackendError ? e.message : "Couldn't cancel.");
    }
    this.recompute();
  }

  // ───────────────────────── tasks

  async setTaskStatus(taskId: string, status: TaskStatus, note?: string): Promise<void> {
    const team = this.s.team;
    const t = this.s.tasks.find((x) => x.id === taskId);
    if (!team || !t) return;
    try {
      await this.d.backend.updateTask(team.id, taskId, { status, note });
      if (status === 'done') {
        // Marking a task done tells the whole team.
        await this.d.backend.sendBroadcast(team.id, `✓ T-${t.num} done by ${this.s.me?.name ?? 'someone'}: ${clip(t.text, 120)}`);
      }
    } catch (e) {
      this.sys(e instanceof BackendError ? e.message : "Couldn't update that task.");
    }
  }

  // ───────────────────────── commands

  async submit(raw: string): Promise<Outcome> {
    const p = parseInput(raw);
    const team = this.s.team;
    if (!team || !this.s.me) return {};
    try {
      switch (p.kind) {
        case 'empty':
          return {};
        case 'error':
          this.sys(p.message);
          return { keepDraft: true };
        case 'quit':
          return { action: 'quit' };
        case 'new':
          return { action: 'new' };
        case 'tasks':
          return { action: 'tasks' };
        case 'help':
          this.sys(COMMANDS.map((c) => `${c.usage.padEnd(30)} ${c.desc}`).join('\n') + '\nPlain text asks your own agent about this folder.');
          return {};
        case 'ask':
          return { keepDraft: !(await this.askLocal(p.text)) };
        case 'read': {
          if (!p.target) return { keepDraft: !(await this.askLocal(p.question, `/read ${p.question}`)) };
          const r = resolveMention(p.target, this.s.members);
          if ('error' in r) {
            this.sys(r.error);
            return { keepDraft: true };
          }
          if (r.member.uid === this.s.me.uid) return { keepDraft: !(await this.askLocal(p.question, `/read ${p.question}`)) };
          this.log('you', 'you', `/read @${r.member.name} ${p.question}`, `u:${this.now()}:${++this.counter}`);
          await this.d.backend.createRequest(team.id, { targetUid: r.member.uid, targetName: r.member.name, question: p.question });
          return {};
        }
        case 'task': {
          const people: Member[] = [];
          for (const name of p.assignees) {
            const r = resolveMention(name, this.s.members);
            if ('error' in r) {
              this.sys(r.error);
              return { keepDraft: true };
            }
            people.push(r.member);
          }
          await this.d.backend.createTasks(team.id, people.map((m) => ({ assigneeUid: m.uid, assigneeName: m.name, text: p.text })));
          return {};
        }
        case 'broadcast':
          await this.d.backend.sendBroadcast(team.id, p.text);
          return {};
        case 'team':
          await this.team(p.sub);
          return {};
        case 'agent':
          await this.agentCommand(p.name);
          return {};
      }
    } catch (e) {
      this.sys(e instanceof BackendError ? e.message : 'Something went wrong. Try again.');
      return { keepDraft: true };
    }
    return {};
  }

  /** Returns true when an answer was shown. */
  private async askLocal(question: string, echo = question): Promise<boolean> {
    this.log('you', 'you', echo, `u:${this.now()}:${++this.counter}`);
    await this.refreshAgents();
    const agent = this.s.agent;
    if (!agent) {
      this.sys('No coding agent found on this laptop. Install Codex or Claude Code, then try again.');
      return false;
    }
    this.emit({ busy: `Asking ${agent} about ${basename(this.d.folder)}…` });
    try {
      const res = await this.d.runner.run(agent, question, this.d.folder);
      if (res.ok) this.log('agent', `your ${agent}`, res.text, `la:${this.now()}:${++this.counter}`, this.now(), `answered from ${basename(this.d.folder)} on this laptop`);
      else this.sys(res.text);
      return res.ok;
    } finally {
      this.emit({ busy: '' });
    }
  }

  private async team(sub?: string): Promise<void> {
    const team = this.s.team!;
    const me = this.s.me!;
    if (sub === 'invite') {
      const inv = await this.d.backend.createInvite(team.id);
      this.sys(`Invite code ${formatCode(inv.code)}. It works once and expires in 10 minutes.`);
      return;
    }
    const now = this.now();
    const lines = this.s.members.map((m) => {
      const dot = isOnline(m, now) ? '●' : '○';
      const who = m.uid === me.uid ? `${m.name} (you)` : m.name;
      const detail = isOnline(m, now)
        ? `${m.agent ?? 'no agent'}${m.project ? ` · ${m.project}` : ''}`
        : `away ${ago(m.lastSeen, now).replace(' ago', '')}`;
      return ` ${dot} ${who.padEnd(18)} ${m.role.padEnd(8)} ${detail}`;
    });
    const hint = me.role === 'manager' ? '\nType /team invite to add someone.' : '';
    this.sys(`${team.name} · ${this.s.members.length} people\n${lines.join('\n')}\nEveryone can assign tasks and ask questions. Only managers invite.${hint}`);
  }

  private async agentCommand(name?: string): Promise<void> {
    if (!name) {
      const s = this.s;
      this.sys(`Using ${s.agent ?? 'no agent'} (setting: ${s.pref}). Installed: ${s.available.join(', ') || 'none'}. Change with /agent codex, /agent claude or /agent auto.`);
      return;
    }
    if (name !== 'codex' && name !== 'claude' && name !== 'auto') {
      this.sys('Choose codex, claude or auto.');
      return;
    }
    await this.refreshAgents(true);
    if (name !== 'auto' && !this.s.available.includes(name)) {
      this.sys(`${name} isn't installed on this laptop.`);
      return;
    }
    this.d.store.saveSettings({ agent: name });
    await this.refreshAgents(true);
    this.sys(`Now using ${this.s.agent ?? 'no agent'}.`);
  }
}
