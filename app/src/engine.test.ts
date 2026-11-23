import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { fakeRunner, type AgentRunner } from './agents.js';
import { LocalStore } from './config.js';
import { Engine } from './engine.js';
import { MemoryStore } from './memoryBackend.js';
import type { AgentName, Team } from './types.js';

const dirs: string[] = [];
const tmp = () => {
  const d = mkdtempSync(join(tmpdir(), 'crewai-eng-'));
  dirs.push(d);
  return d;
};
const engines: Engine[] = [];
afterEach(async () => {
  await Promise.all(engines.splice(0).map((e) => e.stop()));
  dirs.splice(0).forEach((d) => rmSync(d, { recursive: true, force: true }));
});

function world() {
  const clock = { t: 10_000_000 };
  const store = new MemoryStore(() => clock.t);
  let n = 0;
  let team: Team | null = null;
  let boss: ReturnType<MemoryStore['backendFor']> | null = null;

  async function person(name: string, opts: { agents?: AgentName[]; runner?: AgentRunner } = {}) {
    const backend = store.backendFor(`u-${name}`);
    if (!team) {
      boss = backend;
      team = await backend.createTeam('Acme', name);
    } else {
      const inv = await boss!.createInvite(team.id);
      await backend.joinTeam(inv.code, name);
    }
    const local = new LocalStore(tmp());
    const runner = opts.runner ?? fakeRunner(opts.agents ?? ['codex'], (q, a) => `${name}/${a} says: ${q}`);
    const engine = new Engine({
      backend, store: local, runner, folder: join(tmp(), `${name}-project`),
      now: () => clock.t, tickMs: 0, heartbeatMs: 0,
    });
    engines.push(engine);
    await engine.start(team);
    const session = local.newSession(join(tmp(), 'x'), clock.t);
    engine.attach(session, clock.t);
    n++;
    return { name, engine, local, backend, runner: runner as ReturnType<typeof fakeRunner> };
  }
  return { clock, store, person, get n() { return n; } };
}
const texts = (e: Engine) => e.state.entries.map((x) => `${x.kind}:${x.text}`);

describe('local questions', () => {
  it('answers plain text and /read from the local folder', async () => {
    const w = world();
    const a = await w.person('Ann');
    await a.engine.submit('what does this do?');
    await a.engine.submit('/read how is auth done');
    expect(a.runner.calls).toEqual(['what does this do?', 'how is auth done']);
    expect(texts(a.engine).filter((t) => t.startsWith('agent:'))).toHaveLength(2);
    expect(a.engine.state.entries.find((x) => x.kind === 'agent')?.who).toBe('your codex');
  });
  it('says so when no coding agent is installed', async () => {
    const w = await world();
    const a = await w.person('Ann', { agents: [] });
    const out = await a.engine.submit('hello');
    expect(texts(a.engine).some((t) => t.includes('No coding agent found'))).toBe(true);
    expect(out.keepDraft).toBe(true);
  });
  it('hands the text back when nothing was sent', async () => {
    const w = world();
    const a = await w.person('Ann');
    expect((await a.engine.submit('/task @zed do it')).keepDraft).toBe(true);
    expect((await a.engine.submit('/read')).keepDraft).toBe(true);
    expect((await a.engine.submit('/broadcast hi all')).keepDraft).toBeFalsy();
    expect((await a.engine.submit('fine question')).keepDraft).toBe(false);
  });
});

describe('asking a teammate', () => {
  it('needs the owner to allow it, then shows the real answer with attribution', async () => {
    const w = world();
    const boss = await w.person('Boss');
    const hem = await w.person('Hemant', { agents: ['claude'] });
    await boss.engine.submit('/read @hemant what is this project about');
    expect(hem.engine.state.attention[0]).toMatchObject({ type: 'consent' });
    expect(hem.runner.calls).toHaveLength(0);
    await hem.engine.decideConsent(hem.engine.state.attention[0]!.request.id, 'once');
    await vi.waitFor(() => expect(boss.engine.state.entries.some((e) => e.kind === 'agent')).toBe(true));
    const ans = boss.engine.state.entries.find((e) => e.kind === 'agent')!;
    expect(ans.who).toBe("Hemant's claude");
    expect(ans.text).toContain('what is this project about');
    expect(hem.runner.calls).toHaveLength(1);
  });

  it('skips the prompt after "always allow" for that person only', async () => {
    const w = world();
    const boss = await w.person('Boss');
    const hem = await w.person('Hemant');
    const ravi = await w.person('Ravi');
    await boss.engine.submit('/read @hemant first');
    await hem.engine.decideConsent(hem.engine.state.attention[0]!.request.id, 'always');
    await vi.waitFor(() => expect(hem.runner.calls).toHaveLength(1));
    await boss.engine.submit('/read @hemant second');
    await vi.waitFor(() => expect(hem.runner.calls).toHaveLength(2));
    expect(hem.engine.state.attention.filter((a) => a.type === 'consent')).toHaveLength(0);
    await ravi.engine.submit('/read @hemant from ravi');
    expect(hem.engine.state.attention[0]).toMatchObject({ type: 'consent' });
    expect(hem.runner.calls).toHaveLength(2);
  });

  it('reports a decline and never runs the agent', async () => {
    const w = world();
    const boss = await w.person('Boss');
    const hem = await w.person('Hemant');
    await boss.engine.submit('/read @hemant secrets?');
    await hem.engine.decideConsent(hem.engine.state.attention[0]!.request.id, 'decline');
    await vi.waitFor(() => expect(texts(boss.engine).some((t) => t.includes('Hemant declined'))).toBe(true));
    expect(hem.runner.calls).toHaveLength(0);
  });

  it('fails fast when the teammate has no agent', async () => {
    const w = world();
    const boss = await w.person('Boss');
    await w.person('Hemant', { agents: [] });
    await boss.engine.submit('/read @hemant hi');
    await vi.waitFor(() => expect(texts(boss.engine).some((t) => t.includes('no coding agent'))).toBe(true));
  });

  it('refuses unknown or ambiguous names without creating a request', async () => {
    const w = world();
    const boss = await w.person('Boss');
    await w.person('Shwetank');
    await w.person('Shweta');
    await boss.engine.submit('/read @zed hi');
    await boss.engine.submit('/read @shw hi');
    expect(boss.engine.state.requests).toHaveLength(0);
    expect(texts(boss.engine).filter((t) => t.startsWith('system:')).length).toBeGreaterThanOrEqual(2);
  });
});

describe('offline teammate', () => {
  it('offers online alternatives, moves the question once, and tells the returning owner', async () => {
    const w = world();
    const boss = await w.person('Boss');
    const hem = await w.person('Hemant');
    const shw = await w.person('Shwetank');
    w.clock.t += 5 * 60_000; // everyone goes quiet...
    await boss.engine.heartbeat();
    await shw.engine.heartbeat(); // ...except Boss and Shwetank
    await boss.engine.tick();

    await boss.engine.submit('/read @hemant what is this project about');
    const card = boss.engine.state.attention[0]!;
    expect(card).toMatchObject({ type: 'pending', targetOnline: false });
    if (card.type !== 'pending') throw new Error('unreachable');
    expect(card.alternatives.map((m) => m.name)).toEqual(['Shwetank']);

    await boss.engine.moveRequest(card.request.id, shw.backend.uid);
    expect(shw.engine.state.attention[0]).toMatchObject({ type: 'consent' });
    await shw.engine.decideConsent(shw.engine.state.attention[0]!.request.id, 'once');
    await vi.waitFor(() => expect(boss.engine.state.entries.some((e) => e.kind === 'agent')).toBe(true));
    expect(boss.engine.state.entries.find((e) => e.kind === 'agent')!.who).toBe("Shwetank's codex");

    // Hemant returns: no stale prompt, just a note.
    await hem.engine.heartbeat();
    expect(hem.engine.state.attention).toHaveLength(0);
    expect(texts(hem.engine).some((t) => t.includes('already handled by Shwetank'))).toBe(true);
    expect(hem.runner.calls).toHaveLength(0);
  });

  it('keeps a waiting question and delivers it when the owner returns', async () => {
    const w = world();
    const boss = await w.person('Boss');
    const hem = await w.person('Hemant');
    w.clock.t += 5 * 60_000;
    await boss.engine.heartbeat();
    await boss.engine.submit('/read @hemant status?');
    const id = boss.engine.state.attention[0]!.request.id;
    boss.engine.waitFor(id);
    expect(boss.engine.state.attention[0]).toMatchObject({ type: 'pending', compact: true });
    await hem.engine.heartbeat();
    expect(hem.engine.state.attention[0]).toMatchObject({ type: 'consent' });
    expect(boss.engine.state.attention[0]).toMatchObject({ type: 'pending', targetOnline: true, compact: false });
  });

  it('lets the asker cancel, which clears the owner prompt', async () => {
    const w = world();
    const boss = await w.person('Boss');
    const hem = await w.person('Hemant');
    await boss.engine.submit('/read @hemant hi');
    await boss.engine.cancelRequest(boss.engine.state.attention[0]!.request.id);
    expect(hem.engine.state.attention).toHaveLength(0);
    expect(texts(hem.engine).some((t) => t.includes('cancelled their question'))).toBe(true);
  });
});

describe('tasks and broadcasts', () => {
  it('creates one task per assignee and broadcasts when one is marked done', async () => {
    const w = world();
    const boss = await w.person('Boss');
    const hem = await w.person('Hemant');
    const shw = await w.person('Shwetank');
    await boss.engine.submit('/task @hemant @shwetank implement submit button');
    expect(boss.engine.state.tasks.map((t) => t.assigneeName).sort()).toEqual(['Hemant', 'Shwetank']);
    expect(texts(hem.engine).some((t) => t.includes('assigned you T-1'))).toBe(true);
    const mine = hem.engine.state.tasks.find((t) => t.assigneeName === 'Hemant')!;
    await hem.engine.setTaskStatus(mine.id, 'in_progress');
    expect(texts(boss.engine).some((t) => t.includes('Hemant started T-1'))).toBe(true);
    await hem.engine.setTaskStatus(mine.id, 'done');
    expect(shw.engine.state.entries.some((e) => e.kind === 'broadcast' && e.text.includes('T-1 done by Hemant'))).toBe(true);
    // Shwetank cannot change Hemant's task.
    await shw.engine.setTaskStatus(mine.id, 'cancelled');
    expect(boss.engine.state.tasks.find((t) => t.id === mine.id)!.status).toBe('done');
  });

  it('shows broadcasts to everyone, labelled by author, and does not repeat on re-delivery', async () => {
    const w = world();
    const boss = await w.person('Boss');
    const ann = await w.person('Ann');
    await boss.engine.submit('/broadcast release freeze Friday');
    const rows = ann.engine.state.entries.filter((e) => e.kind === 'broadcast');
    expect(rows).toHaveLength(1);
    expect(rows[0]).toMatchObject({ who: 'Boss', text: 'release freeze Friday' });
    await ann.engine.tick();
    await ann.engine.tick();
    expect(ann.engine.state.entries.filter((e) => e.kind === 'broadcast')).toHaveLength(1);
  });

  it('keeps going when the preferred agent disappears', async () => {
    const w = world();
    let installed: AgentName[] = ['codex', 'claude'];
    const runner: AgentRunner = {
      detect: async () => installed,
      run: async (agent, q) => ({ ok: true, text: `${agent}:${q}`, agent, isolation: 'best-effort' }),
    };
    const ann = await w.person('Ann', { runner });
    await ann.engine.submit('/agent codex');
    installed = ['claude'];
    w.clock.t += 60_000;
    await ann.engine.submit('still works?');
    expect(ann.engine.state.entries.find((e) => e.kind === 'agent')?.text).toBe('claude:still works?');
    expect(texts(ann.engine).some((t) => t.includes("isn't available here"))).toBe(true);
  });
});

describe('team commands', () => {
  it('only managers can invite, and invites are shown as grouped digits', async () => {
    const w = world();
    const boss = await w.person('Boss');
    const ann = await w.person('Ann');
    await ann.engine.submit('/team invite');
    expect(texts(ann.engine).some((t) => t.includes('Only a manager'))).toBe(true);
    await boss.engine.submit('/team invite');
    expect(texts(boss.engine).some((t) => /Invite code \d{3} \d{3}/.test(t))).toBe(true);
  });
  it('lists people with presence', async () => {
    const w = world();
    const boss = await w.person('Boss');
    await w.person('Ann');
    await boss.engine.submit('/team');
    const t = boss.engine.state.entries.map((e) => e.text).join('\n');
    expect(t).toContain('Boss (you)');
    expect(t).toContain('Ann');
  });
});

describe('review fixes', () => {
  it('lets only one window per laptop answer, and hands over when it closes', async () => {
    const w = world();
    const boss = await w.person('Boss');
    const hem = await w.person('Hemant');
    // A second window for Hemant: same account and same ~/.crewai.
    const second = new Engine({
      backend: hem.backend, store: hem.local, runner: fakeRunner(['codex'], () => 'second'), folder: join(tmp(), 'other'),
      now: () => w.clock.t, tickMs: 0, heartbeatMs: 0,
    });
    engines.push(second);
    await second.start(hem.engine.state.team!);
    second.attach(hem.local.newSession(join(tmp(), 'y'), w.clock.t), w.clock.t);
    expect(second.state.responder).toBe(false);
    expect(texts(second).some((t) => t.includes('Another crewai window'))).toBe(true);

    hem.local.allowAlways(hem.engine.state.team!.id, boss.backend.uid);
    await boss.engine.submit('/read @hemant what is this');
    await vi.waitFor(() => expect(boss.engine.state.entries.some((e) => e.kind === 'agent')).toBe(true));
    expect(boss.engine.state.entries.find((e) => e.kind === 'agent')!.text).not.toBe('second');
    expect(texts(boss.engine).some((t) => t.includes("couldn't answer"))).toBe(false);
    expect(second.state.attention).toHaveLength(0);

    await hem.engine.stop();
    await second.tick();
    expect(second.state.responder).toBe(true);
  });

  it('removes secrets from an answer before it is sent to the asker', async () => {
    const w = world();
    const boss = await w.person('Boss');
    // A fake key, assembled at runtime so secret scanners never see a whole key in the source.
    const fakeKey = ['sk', 'live', 'abcdefghijklmnop1234'].join('_');
    const runner = fakeRunner(['codex'], () => `The key is in .env: STRIPE_SECRET_KEY=${fakeKey}`);
    const hem = await w.person('Hemant', { runner });
    await boss.engine.submit('/read @hemant print the env file');
    await hem.engine.decideConsent(hem.engine.state.attention[0]!.request.id, 'once');
    await vi.waitFor(() => expect(boss.engine.state.entries.some((e) => e.kind === 'agent')).toBe(true));
    const ans = boss.engine.state.entries.find((e) => e.kind === 'agent')!.text;
    expect(ans).not.toContain(fakeKey);
    expect(ans).toContain('[secret removed]');
    expect(texts(hem.engine).some((t) => t.includes('looked like secrets'))).toBe(true);
  });

  it('does not re-broadcast a task that is already done', async () => {
    const w = world();
    const boss = await w.person('Boss');
    const hem = await w.person('Hemant');
    await boss.engine.submit('/task @hemant ship it');
    const t = hem.engine.state.tasks[0]!;
    await hem.engine.setTaskStatus(t.id, 'done');
    await hem.engine.setTaskStatus(t.id, 'done');
    expect(boss.engine.state.broadcasts.filter((b) => b.text.includes('T-1 done'))).toHaveLength(1);
    expect(texts(hem.engine).some((x) => x.includes('already done'))).toBe(true);
  });

  it('creates no tasks at all when one assignee is invalid', async () => {
    const w = world();
    const boss = await w.person('Boss');
    const hem = await w.person('Hemant');
    await expect(
      boss.backend.createTasks(boss.engine.state.team!.id, [
        { assigneeUid: hem.backend.uid, assigneeName: 'Hemant', text: 'one' },
        { assigneeUid: 'nobody', assigneeName: 'Zed', text: 'two' },
      ]),
    ).rejects.toThrow();
    expect(w.store.tasks.get(boss.engine.state.team!.id) ?? []).toHaveLength(0);
  });

  it('shows an answer that arrived while crewai was closed in a brand-new chat, once', async () => {
    const w = world();
    const boss = await w.person('Boss');
    const hem = await w.person('Hemant');
    await boss.engine.submit('/read @hemant status?');
    await boss.engine.stop(); // Boss closes crewai
    w.clock.t += 60_000;
    await hem.engine.decideConsent(hem.engine.state.attention[0]!.request.id, 'once');
    await vi.waitFor(() => expect(w.store.requests.get(hem.engine.state.team!.id)![0]!.status).toBe('answered'));

    const again = new Engine({ backend: boss.backend, store: boss.local, runner: boss.runner, folder: join(tmp(), 'b'), now: () => w.clock.t, tickMs: 0, heartbeatMs: 0 });
    engines.push(again);
    await again.start(boss.engine.state.team!);
    expect(again.unseenAnswers()).toBe(1);
    again.attach(boss.local.newSession(join(tmp(), 'n'), w.clock.t), w.clock.t);
    expect(again.state.entries.filter((e) => e.kind === 'agent')).toHaveLength(1);
    expect(again.unseenAnswers()).toBe(0);
    // A second new chat does not repeat it.
    w.clock.t += 1000;
    again.attach(boss.local.newSession(join(tmp(), 'm'), w.clock.t), w.clock.t);
    expect(again.state.entries.filter((e) => e.kind === 'agent')).toHaveLength(0);
  });

  it('lets a manager remove and promote people, and a member leave', async () => {
    const w = world();
    const boss = await w.person('Boss');
    const ann = await w.person('Ann');
    const bob = await w.person('Bob');
    await ann.engine.submit('/team remove @bob');
    expect(texts(ann.engine).some((t) => t.includes('Only a manager'))).toBe(true);
    await boss.engine.submit('/team promote @ann');
    expect(boss.engine.state.members.find((m) => m.name === 'Ann')!.role).toBe('manager');
    await ann.engine.submit('/team remove @bob');
    expect(boss.engine.state.members.map((m) => m.name).sort()).toEqual(['Ann', 'Boss']);
    expect(await bob.backend.getTeam(boss.engine.state.team!.id).catch(() => 'denied')).toBe('denied');
    await ann.engine.submit('/team remove @boss');
    expect(boss.engine.state.members).toHaveLength(2);
    expect((await ann.engine.submit('/team leave')).action).toBe('quit');
    expect(ann.local.settings().teamId).toBe('');
    expect(boss.engine.state.members.map((m) => m.name)).toEqual(['Boss']);
  });
});
