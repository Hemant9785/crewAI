import { createHash, randomBytes } from 'node:crypto';
import {
  chmodSync, existsSync, mkdirSync, readFileSync, readdirSync, renameSync, rmSync, writeFileSync,
} from 'node:fs';
import { homedir } from 'node:os';
import { join, resolve } from 'node:path';
import type { AgentPref } from './types.js';

export interface Settings {
  displayName: string;
  teamId: string;
  teamName: string;
  agent: AgentPref;
  /** Per team: uid -> time (local ms) until which that person's questions run without asking. */
  allowUntil: Record<string, Record<string, number>>;
  /** Per team: server time of the newest answer to my questions already shown in a chat. */
  answersSeenAt: Record<string, number>;
  /** Thread id -> time of the latest activity I have looked at. Kept to the newest few hundred. */
  threadsSeen: Record<string, number>;
}

/** How long "Always allow" lasts before the owner is asked again. */
export const ALLOW_DAYS = 7;
const ALLOW_MS = ALLOW_DAYS * 86_400_000;

export interface Identity {
  email: string;
  password: string;
}

export type EntryKind = 'you' | 'agent' | 'broadcast' | 'task' | 'thread' | 'system' | 'question';

export interface Entry {
  id: string;
  /** Dedupe key so reconnecting or resuming never repeats an event. */
  key: string;
  at: number;
  kind: EntryKind;
  who: string;
  text: string;
  /** Small dim footer, for example where an answer came from. */
  meta?: string;
}

export interface Session {
  id: string;
  folder: string;
  startedAt: number;
  updatedAt: number;
  title: string;
  entries: Entry[];
  /**
   * Newest time among entries trimmed off the front of the log. Events at or before it are never
   * added again, so a long chat does not replay old events after trimming.
   */
  floor?: number;
}

export interface SessionSummary {
  id: string;
  updatedAt: number;
  title: string;
  count: number;
}

const DEFAULTS: Settings = { displayName: '', teamId: '', teamName: '', agent: 'auto', allowUntil: {}, answersSeenAt: {}, threadsSeen: {} };

function writeJson(path: string, data: unknown, mode?: number): void {
  const tmp = `${path}.${process.pid}.tmp`;
  writeFileSync(tmp, JSON.stringify(data, null, 2), { mode });
  renameSync(tmp, path);
  if (mode !== undefined) {
    try {
      chmodSync(path, mode);
    } catch {
      /* not supported on this filesystem */
    }
  }
}

function readJson<T>(path: string): T | null {
  try {
    return JSON.parse(readFileSync(path, 'utf8')) as T;
  } catch {
    return null;
  }
}

export function folderKey(folder: string): string {
  const norm = resolve(folder);
  const key = process.platform === 'win32' ? norm.toLowerCase() : norm;
  return createHash('sha1').update(key).digest('hex').slice(0, 12);
}

export class LocalStore {
  readonly dir: string;
  constructor(dir: string = process.env.CREWAI_HOME ?? join(homedir(), '.crewai')) {
    this.dir = dir;
    mkdirSync(dir, { recursive: true });
  }

  settings(): Settings {
    const raw = readJson<Partial<Settings> & { alwaysAllow?: Record<string, string[]> }>(join(this.dir, 'settings.json')) ?? {};
    const { alwaysAllow, ...rest } = raw;
    const s: Settings = { ...DEFAULTS, ...rest, allowUntil: { ...(rest.allowUntil ?? {}) }, answersSeenAt: { ...(rest.answersSeenAt ?? {}) }, threadsSeen: { ...(rest.threadsSeen ?? {}) } };
    // Older versions kept "always allow" forever. Carry those over with the normal time limit.
    if (alwaysAllow) {
      const until = Date.now() + ALLOW_MS;
      for (const [team, uids] of Object.entries(alwaysAllow)) {
        s.allowUntil[team] = { ...Object.fromEntries(uids.map((u) => [u, until])), ...(s.allowUntil[team] ?? {}) };
      }
      writeJson(join(this.dir, 'settings.json'), s);
    }
    return s;
  }
  saveSettings(patch: Partial<Settings>): Settings {
    const next = { ...this.settings(), ...patch };
    writeJson(join(this.dir, 'settings.json'), next);
    return next;
  }

  isAlwaysAllowed(teamId: string, uid: string, now = Date.now()): boolean {
    return (this.settings().allowUntil[teamId]?.[uid] ?? 0) > now;
  }
  allowAlways(teamId: string, uid: string, now = Date.now()): void {
    const s = this.settings();
    this.saveSettings({ allowUntil: { ...s.allowUntil, [teamId]: { ...(s.allowUntil[teamId] ?? {}), [uid]: now + ALLOW_MS } } });
  }
  /** Forget everything tied to a team (after leaving it). */
  forgetTeam(teamId: string): void {
    const s = this.settings();
    const allowUntil = { ...s.allowUntil };
    const answersSeenAt = { ...s.answersSeenAt };
    delete allowUntil[teamId];
    delete answersSeenAt[teamId];
    this.saveSettings({ allowUntil, answersSeenAt, ...(s.teamId === teamId ? { teamId: '', teamName: '' } : {}) });
  }

  /**
   * Only one crewai window per laptop answers teammates' questions (they all share one account).
   * Returns true when this window holds the lock. A lock left by a process that has exited is taken over.
   */
  claimResponder(token: string): boolean {
    const path = join(this.dir, 'responder.lock');
    const mine = JSON.stringify({ pid: process.pid, token });
    for (let attempt = 0; attempt < 2; attempt++) {
      try {
        writeFileSync(path, mine, { flag: 'wx' });
        return true;
      } catch {
        const cur = readJson<{ pid: number; token: string }>(path);
        if (cur?.token === token) return true;
        if (cur && processAlive(cur.pid)) return false;
        try {
          rmSync(path, { force: true });
        } catch {
          return false;
        }
      }
    }
    return false;
  }
  releaseResponder(token: string): void {
    const path = join(this.dir, 'responder.lock');
    if (readJson<{ token: string }>(path)?.token === token) {
      try {
        rmSync(path, { force: true });
      } catch {
        /* already gone */
      }
    }
  }

  /** Stable device account for the backend. The password never leaves this machine except to sign in. */
  identity(): Identity {
    const path = join(this.dir, 'identity.json');
    const existing = readJson<Identity>(path);
    if (existing?.email && existing.password) return existing;
    const id: Identity = {
      email: `device-${randomBytes(8).toString('hex')}@crewai.invalid`,
      password: randomBytes(24).toString('base64url'),
    };
    writeJson(path, id, 0o600);
    return id;
  }

  private sessionsDir(folder: string): string {
    const d = join(this.dir, 'sessions', folderKey(folder));
    mkdirSync(d, { recursive: true });
    return d;
  }

  newSession(folder: string, now = Date.now()): Session {
    const s: Session = {
      id: `${now.toString(36)}-${randomBytes(3).toString('hex')}`,
      folder: resolve(folder), startedAt: now, updatedAt: now, title: 'New chat', entries: [],
    };
    this.saveSession(s);
    return s;
  }

  saveSession(s: Session): void {
    writeJson(join(this.sessionsDir(s.folder), `${s.id}.json`), s);
  }

  loadSession(folder: string, id: string): Session | null {
    if (!/^[\w-]+$/.test(id)) return null;
    return readJson<Session>(join(this.sessionsDir(folder), `${id}.json`));
  }

  listSessions(folder: string): SessionSummary[] {
    const d = this.sessionsDir(folder);
    const out: SessionSummary[] = [];
    for (const f of readdirSync(d)) {
      if (!f.endsWith('.json')) continue;
      const s = readJson<Session>(join(d, f));
      if (s && s.entries.length > 0) {
        out.push({ id: s.id, updatedAt: s.updatedAt, title: s.title, count: s.entries.length });
      }
    }
    return out.sort((a, b) => b.updatedAt - a.updatedAt);
  }

  hasFile(name: string): boolean {
    return existsSync(join(this.dir, name));
  }
}

function processAlive(pid: number): boolean {
  if (!Number.isInteger(pid) || pid <= 0) return false;
  try {
    process.kill(pid, 0);
    return true;
  } catch (e) {
    return (e as NodeJS.ErrnoException).code === 'EPERM';
  }
}

const MAX_ENTRIES = 500;

/** Append without duplicates (by key) and keep the log bounded. */
export function appendEntry(s: Session, e: Omit<Entry, 'id'>, max = MAX_ENTRIES): Entry | null {
  if (s.entries.some((x) => x.key === e.key)) return null;
  // Anything at or before the newest trimmed entry was already shown once; never add it again.
  if (s.floor !== undefined && e.at <= s.floor) return null;
  const entry: Entry = { ...e, id: randomBytes(5).toString('hex') };
  s.entries.push(entry);
  if (s.entries.length > max) {
    const gone = s.entries.splice(0, s.entries.length - max);
    s.floor = Math.max(s.floor ?? -Infinity, ...gone.map((g) => g.at));
  }
  s.updatedAt = Math.max(s.updatedAt, e.at);
  if (s.title === 'New chat' && (e.kind === 'you' || e.kind === 'question')) s.title = e.text.slice(0, 60);
  return entry;
}
