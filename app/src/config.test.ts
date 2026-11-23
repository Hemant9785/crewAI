import { mkdtempSync, rmSync, statSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { buildCommand, fakeRunner, pickAgent, truncate } from './agents.js';
import { appendEntry, folderKey, LocalStore } from './config.js';

const dirs: string[] = [];
const tmp = () => {
  const d = mkdtempSync(join(tmpdir(), 'crewai-test-'));
  dirs.push(d);
  return d;
};
afterEach(() => dirs.splice(0).forEach((d) => rmSync(d, { recursive: true, force: true })));

describe('LocalStore', () => {
  it('keeps one stable device identity with owner-only permissions', () => {
    const s = new LocalStore(tmp());
    const a = s.identity();
    expect(s.identity()).toEqual(a);
    if (process.platform !== 'win32') expect(statSync(join(s.dir, 'identity.json')).mode & 0o077).toBe(0);
  });
  it('saves settings and always-allow per team', () => {
    const s = new LocalStore(tmp());
    s.saveSettings({ displayName: 'Ann' });
    s.allowAlways('t1', 'u1');
    expect(s.isAlwaysAllowed('t1', 'u1')).toBe(true);
    expect(s.isAlwaysAllowed('t2', 'u1')).toBe(false);
    expect(s.settings().displayName).toBe('Ann');
  });
  it('lists only non-empty sessions for this folder, newest first, and resumes them', () => {
    const s = new LocalStore(tmp());
    const folder = tmp();
    const a = s.newSession(folder, 1000);
    expect(s.listSessions(folder)).toHaveLength(0);
    appendEntry(a, { key: 'k1', at: 1100, kind: 'you', who: 'you', text: 'hello world' });
    s.saveSession(a);
    const b = s.newSession(folder, 2000);
    appendEntry(b, { key: 'k2', at: 2100, kind: 'you', who: 'you', text: 'later' });
    s.saveSession(b);
    expect(s.listSessions(folder).map((x) => x.id)).toEqual([b.id, a.id]);
    expect(s.listSessions(tmp())).toHaveLength(0);
    expect(s.loadSession(folder, a.id)?.entries[0]?.text).toBe('hello world');
    expect(s.loadSession(folder, '../evil')).toBeNull();
  });
  it('dedupes entries by key', () => {
    const s = new LocalStore(tmp());
    const sess = s.newSession(tmp());
    expect(appendEntry(sess, { key: 'x', at: 1, kind: 'system', who: '', text: 'a' })).not.toBeNull();
    expect(appendEntry(sess, { key: 'x', at: 2, kind: 'system', who: '', text: 'a' })).toBeNull();
  });
  it('maps the same folder to the same key', () => {
    expect(folderKey('.')).toBe(folderKey(process.cwd()));
  });
});

describe('agents', () => {
  it('never puts user text on the command line', () => {
    for (const a of ['codex', 'claude'] as const) {
      const { args } = buildCommand(a, 'out.txt');
      expect(args.join(' ')).not.toMatch(/[;&|`$"]/);
    }
    expect(buildCommand('codex', 'o').args).toContain('read-only');
  });
  it('falls back when the preferred agent disappears', () => {
    expect(pickAgent('codex', ['claude'])).toEqual({ agent: 'claude', fellBack: true });
    expect(pickAgent('auto', ['claude', 'codex'])).toEqual({ agent: 'codex', fellBack: false });
    expect(pickAgent('claude', ['claude'])).toEqual({ agent: 'claude', fellBack: false });
    expect(pickAgent('auto', [])).toEqual({ agent: null, fellBack: false });
  });
  it('truncates long output', () => {
    expect(truncate('a'.repeat(9000)).length).toBeLessThanOrEqual(8000);
  });
  it('fake runner records questions', async () => {
    const r = fakeRunner(['codex'], (q) => `A:${q}`);
    expect((await r.run('codex', 'hi', '.')).text).toBe('A:hi');
    expect(r.calls).toEqual(['hi']);
  });
});
