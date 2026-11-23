import { mkdtempSync, rmSync, statSync, writeFileSync } from 'node:fs';
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

describe('fixes from the 2026-11-23 review', () => {
  it('stops agents that ignore the time limit, including a process tree on Windows', async () => {
    const { execForTest } = await import('./agents.js');
    const started = Date.now();
    const r = await execForTest(process.execPath, ['-e', 'setInterval(() => {}, 1000)'], null, process.cwd(), 300);
    expect(r.timedOut).toBe(true);
    expect(Date.now() - started).toBeLessThan(6000);
  }, 10000);

  it('treats a non-zero exit as a failure even when something was printed', async () => {
    const { interpretRun } = await import('./agents.js');
    expect(interpretRun('claude', 1, 'Error: not logged in', '')).toMatchObject({ ok: false });
    expect(interpretRun('claude', 1, 'usage: claude', '').text).toContain('usage: claude');
    expect(interpretRun('claude', 0, 'the answer', '')).toMatchObject({ ok: true, text: 'the answer' });
    expect(interpretRun('codex', 1, '', 'warn', 'answer from file')).toMatchObject({ ok: true, text: 'answer from file' });
    expect(interpretRun('codex', 0, '   ', '')).toMatchObject({ ok: false });
  });

  it('removes values that look like secrets from answers', async () => {
    const { redactSecrets } = await import('./agents.js');
    const raw = [
      'DATABASE_PASSWORD=hunter2hunter2',
      'aws key AKIAABCDEFGHIJKLMNOP here',
      'token ghp_abcdefghijklmnopqrstuvwxyz0123456789',
      '"apiKey": "AIzaSyA-1234567890abcdefghijklmnopqrstu"',
      'postgres://admin:s3cretpw@db.example.com/app',
      '-----BEGIN RSA PRIVATE KEY-----\nMIIabc\n-----END RSA PRIVATE KEY-----',
      'API_KEY=${API_KEY} and password: ****',
      'The login flow lives in src/auth.ts.',
    ].join('\n');
    const { text, count } = redactSecrets(raw);
    for (const leaked of ['hunter2hunter2', 'AKIAABCDEFGHIJKLMNOP', 'ghp_abc', 'AIzaSyA', 's3cretpw', 'MIIabc']) {
      expect(text).not.toContain(leaked);
    }
    expect(text).toContain('API_KEY=${API_KEY}');
    expect(text).toContain('src/auth.ts');
    expect(count).toBeGreaterThanOrEqual(6);
  });

  it('runs Claude with write, shell and web tools explicitly denied', () => {
    const args = buildCommand('claude', 'o').args.join(' ');
    expect(args).toContain('--disallowedTools');
    expect(args).toMatch(/Bash/);
    expect(args).toContain('--permission-mode default');
  });

  it('lets only one window per laptop answer questions, and takes over a lock left by a dead process', () => {
    const s = new LocalStore(tmp());
    expect(s.claimResponder('a')).toBe(true);
    expect(s.claimResponder('a')).toBe(true);
    expect(s.claimResponder('b')).toBe(false);
    s.releaseResponder('b'); // not the holder: no effect
    expect(s.claimResponder('b')).toBe(false);
    s.releaseResponder('a');
    expect(s.claimResponder('b')).toBe(true);
    writeFileSync(join(s.dir, 'responder.lock'), JSON.stringify({ pid: 2_147_000_000, token: 'dead' }));
    expect(s.claimResponder('c')).toBe(true);
  });

  it('expires "always allow" and migrates the old forever list', () => {
    const s = new LocalStore(tmp());
    s.allowAlways('t1', 'u1', 1000);
    expect(s.isAlwaysAllowed('t1', 'u1', 2000)).toBe(true);
    expect(s.isAlwaysAllowed('t1', 'u1', 1000 + 8 * 86_400_000)).toBe(false);
    const old = new LocalStore(tmp());
    writeFileSync(join(old.dir, 'settings.json'), JSON.stringify({ displayName: 'Ann', alwaysAllow: { t1: ['u9'] } }));
    expect(old.isAlwaysAllowed('t1', 'u9')).toBe(true);
    expect(old.isAlwaysAllowed('t1', 'u9', Date.now() + 8 * 86_400_000)).toBe(false);
    expect(old.settings().displayName).toBe('Ann');
    old.forgetTeam('t1');
    expect(old.isAlwaysAllowed('t1', 'u9')).toBe(false);
  });

  it('never re-adds an event after it was trimmed from a long chat', () => {
    const s = new LocalStore(tmp());
    const sess = s.newSession(tmp(), 0);
    for (let i = 1; i <= 5; i++) appendEntry(sess, { key: `k${i}`, at: i, kind: 'system', who: '', text: `${i}` }, 3);
    expect(sess.entries.map((e) => e.key)).toEqual(['k3', 'k4', 'k5']);
    expect(appendEntry(sess, { key: 'k1', at: 1, kind: 'system', who: '', text: '1' }, 3)).toBeNull();
    expect(appendEntry(sess, { key: 'k6', at: 6, kind: 'system', who: '', text: '6' }, 3)).not.toBeNull();
  });
});
