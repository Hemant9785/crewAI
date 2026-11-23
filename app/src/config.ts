import { createHash, randomBytes } from 'node:crypto';
import {
  chmodSync, existsSync, mkdirSync, readFileSync, readdirSync, renameSync, writeFileSync,
} from 'node:fs';
import { homedir } from 'node:os';
import { join, resolve } from 'node:path';
import type { AgentPref } from './types.js';

export interface Settings {
  displayName: string;
  teamId: string;
  teamName: string;
  agent: AgentPref;
  /** uids the owner chose "Always allow" for, per team. */
  alwaysAllow: Record<string, string[]>;
}

export interface Identity {
  email: string;
  password: string;
}

export type EntryKind = 'you' | 'agent' | 'broadcast' | 'task' | 'system' | 'question';

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
}

export interface SessionSummary {
  id: string;
  updatedAt: number;
  title: string;
  count: number;
}

const DEFAULTS: Settings = { displayName: '', teamId: '', teamName: '', agent: 'auto', alwaysAllow: {} };

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
    return { ...DEFAULTS, ...(readJson<Partial<Settings>>(join(this.dir, 'settings.json')) ?? {}) };
  }
  saveSettings(patch: Partial<Settings>): Settings {
    const next = { ...this.settings(), ...patch };
    writeJson(join(this.dir, 'settings.json'), next);
    return next;
  }

  isAlwaysAllowed(teamId: string, uid: string): boolean {
    return (this.settings().alwaysAllow[teamId] ?? []).includes(uid);
  }
  allowAlways(teamId: string, uid: string): void {
    const s = this.settings();
    const list = new Set(s.alwaysAllow[teamId] ?? []);
    list.add(uid);
    this.saveSettings({ alwaysAllow: { ...s.alwaysAllow, [teamId]: [...list] } });
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

const MAX_ENTRIES = 500;

/** Append without duplicates (by key) and keep the log bounded. */
export function appendEntry(s: Session, e: Omit<Entry, 'id'>): Entry | null {
  if (s.entries.some((x) => x.key === e.key)) return null;
  const entry: Entry = { ...e, id: randomBytes(5).toString('hex') };
  s.entries.push(entry);
  if (s.entries.length > MAX_ENTRIES) s.entries.splice(0, s.entries.length - MAX_ENTRIES);
  s.updatedAt = e.at;
  if (s.title === 'New chat' && (e.kind === 'you' || e.kind === 'question')) s.title = e.text.slice(0, 60);
  return entry;
}
