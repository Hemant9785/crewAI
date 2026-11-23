import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { fakeRunner } from './agents.js';
import { parseInput } from './commands.js';
import { LocalStore } from './config.js';
import { Engine } from './engine.js';
import { MemoryStore } from './memoryBackend.js';
import type { Team } from './types.js';

const dirs: string[] = [];
const tmp = () => {
  const d = mkdtempSync(join(tmpdir(), 'crewai-thr-'));
  dirs.push(d);
  return d;
};
const engines: Engine[] = [];
afterEach(async () => {
  await Promise.all(engines.splice(0).map((e) => e.stop()));
  dirs.splice(0).forEach((d) => rmSync(d, { recursive: true, force: true }));
});

const PR = 'https://github.com/acme/web/pull/88';

function world() {
  const clock = { t: 10_000_000 };
  const cloud = new MemoryStore(() => clock.t);
  let team: Team | null = null;
  let boss: ReturnType<MemoryStore['backendFor']> | null = null;
  async function person(name: string) {
    const backend = cloud.backendFor(`u-${name}`);
    if (!team) {
      boss = backend;
      team = await backend.createTeam('Acme', name);
    } else {
      await backend.joinTeam((await boss!.createInvite(team.id)).code, name);
    }
    const local = new LocalStore(tmp());
    const engine = new Engine({ backend, store: local, runner: fakeRunner(['codex'], () => ''), folder: tmp(), now: () => clock.t, tickMs: 0, heartbeatMs: 0 });
    engines.push(engine);
    await engine.start(team);
    engine.attach(local.newSession(tmp(), clock.t), clock.t);
    return { name, engine, backend };
  }
  return { clock, person, get team() { return team!; } };
}
const feed = (e: Engine) => e.state.entries.map((x) => x.text);

describe('parsing', () => {
  it('parses a follow-up with and without a recipient', () => {
    expect(parseInput('/followup T-4 @ann which design?')).toEqual({ kind: 'followup', taskNum: 4, recipients: ['ann'], text: 'which design?' });
    expect(parseInput('/followup 4 which design?')).toEqual({ kind: 'followup', taskNum: 4, recipients: [], text: 'which design?' });
    expect(parseInput('/followup #12 @a @b ready?')).toMatchObject({ taskNum: 12, recipients: ['a', 'b'] });
  });
  it('parses a review with a link, reviewers and comments', () => {
    expect(parseInput(`/review ${PR} @ann @bob please check the retry logic`)).toEqual({
      kind: 'review', url: PR, reviewers: ['ann', 'bob'], text: 'please check the retry logic',
    });
    expect(parseInput(`/review ${PR} @team`)).toMatchObject({ reviewers: ['team'], text: '' });
  });
  it('rejects incomplete commands without sending anything', () => {
    for (const bad of ['/followup', '/followup T-4', '/followup banana @ann hi', '/review', `/review ${PR}`, `/review ${PR} no mention`]) {
      expect(parseInput(bad)).toMatchObject({ kind: 'error' });
    }
    expect(parseInput('/threads')).toEqual({ kind: 'threads' });
    expect(parseInput('/inbox')).toEqual({ kind: 'threads' });
  });
});

describe('follow-up questions about a task', () => {
  it('goes only to the other person on the task and appears in the feed of that person alone', async () => {
    const w = world();
    const boss = await w.person('Boss');
    const ann = await w.person('Ann');
    const bob = await w.person('Bob');
    await boss.engine.submit('/task @ann build the form');
    const out = await boss.engine.submit('/followup T-1 Which design should it follow?');
    expect(out.keepDraft).toBeFalsy();
    expect(feed(boss.engine).some((t) => t.includes('Asked Ann about T-1'))).toBe(true);
    expect(feed(ann.engine).some((t) => t.includes('Boss asked you about T-1: Which design should it follow?'))).toBe(true);
    // Bob is on the team but never sees any trace of it.
    expect(bob.engine.state.threads).toHaveLength(0);
    expect(feed(bob.engine).some((t) => t.includes('design'))).toBe(false);
    expect(ann.engine.unreadThreads()).toHaveLength(1);
    expect(boss.engine.unreadThreads()).toHaveLength(0); // I started it
  });

  it('defaults to the assigner when the assignee asks', async () => {
    const w = world();
    const boss = await w.person('Boss');
    const ann = await w.person('Ann');
    await boss.engine.submit('/task @ann build the form');
    await ann.engine.submit('/followup T-1 Is the API ready?');
    expect(boss.engine.state.threads).toHaveLength(1);
    expect(feed(boss.engine).some((t) => t.includes('Ann asked you about T-1'))).toBe(true);
  });

  it('shows replies, and an unread reply stops counting once it is seen', async () => {
    const w = world();
    const boss = await w.person('Boss');
    const ann = await w.person('Ann');
    await boss.engine.submit('/task @ann build the form');
    await boss.engine.submit('/followup T-1 @ann Are you blocked?');
    const id = ann.engine.state.threads[0]!.id;
    w.clock.t += 1000;
    expect(await ann.engine.reply(id, 'Yes, waiting on the API.')).toBeNull();
    expect(feed(boss.engine).some((t) => t.includes('Ann replied on T-1: Yes, waiting on the API.'))).toBe(true);
    expect(boss.engine.unreadThreads()).toHaveLength(1);
    boss.engine.markThreadSeen(id);
    expect(boss.engine.unreadThreads()).toHaveLength(0);
    w.clock.t += 1000;
    await ann.engine.reply(id, 'Unblocked now.');
    expect(boss.engine.unreadThreads()).toHaveLength(1); // a newer reply counts again
  });

  it('explains problems and keeps the text', async () => {
    const w = world();
    const boss = await w.person('Boss');
    await w.person('Ann');
    const none = await boss.engine.submit('/followup T-9 hello?');
    expect(none.keepDraft).toBe(true);
    expect(feed(boss.engine).some((t) => t.includes('There is no task T-9'))).toBe(true);
    await boss.engine.submit('/task @boss do it');
    const self = await boss.engine.submit('/followup T-1 hi');
    expect(self.keepDraft).toBe(true); // both sides of the task are me: say who to ask
    const unknown = await boss.engine.submit('/followup T-1 @zed hi');
    expect(unknown.keepDraft).toBe(true);
  });
});

describe('pull request reviews', () => {
  it('asks the chosen reviewers, shows their verdicts, and keeps everyone else out', async () => {
    const w = world();
    const boss = await w.person('Boss');
    const ann = await w.person('Ann');
    const bob = await w.person('Bob');
    const cat = await w.person('Cat');
    await boss.engine.submit(`/review ${PR} @ann @bob Please check the retry logic. I am unsure about the backoff.`);
    expect(feed(boss.engine).some((t) => t.includes('to review acme/web#88'))).toBe(true);
    for (const r of [ann, bob]) {
      expect(feed(r.engine).some((t) => t.includes('Boss asked you to review acme/web#88: Please check the retry logic'))).toBe(true);
    }
    expect(cat.engine.state.threads).toHaveLength(0);
    expect(feed(cat.engine).some((t) => t.includes('acme/web'))).toBe(false);

    const id = ann.engine.state.threads[0]!.id;
    w.clock.t += 1000;
    await ann.engine.reply(id, 'Backoff looks right.', 'approved');
    w.clock.t += 1000;
    await bob.engine.reply(id, 'Needs a cap on retries.', 'changes');
    expect(feed(boss.engine).some((t) => t.includes('Ann approved acme/web#88'))).toBe(true);
    expect(feed(boss.engine).some((t) => t.includes('Bob asked for changes on acme/web#88: Needs a cap on retries.'))).toBe(true);
    // The author cannot approve their own pull request.
    expect(await boss.engine.reply(id, 'LGTM', 'approved')).toContain('Only a reviewer');
    // Closing is the author's job.
    expect(await ann.engine.setThreadStatus(id, 'closed')).not.toBeNull();
    expect(await boss.engine.setThreadStatus(id, 'closed')).toBeNull();
    expect(await ann.engine.reply(id, 'late')).toContain('closed');
  });

  it('can ask the whole team with @team', async () => {
    const w = world();
    const boss = await w.person('Boss');
    const ann = await w.person('Ann');
    const bob = await w.person('Bob');
    await boss.engine.submit(`/review ${PR} @team everyone please look`);
    expect(ann.engine.state.threads).toHaveLength(1);
    expect(bob.engine.state.threads).toHaveLength(1);
  });

  it('rejects a bad link, a missing reviewer or yourself, and keeps the text', async () => {
    const w = world();
    const boss = await w.person('Boss');
    await w.person('Ann');
    for (const bad of ['/review http://github.com/a/b/pull/1 @ann hi', '/review javascript:alert(1) @ann hi', `/review ${PR} @zed hi`, `/review ${PR} @boss hi`]) {
      const out = await boss.engine.submit(bad);
      expect(out.keepDraft).toBe(true);
    }
    expect(boss.engine.state.threads).toHaveLength(0);
  });
});
