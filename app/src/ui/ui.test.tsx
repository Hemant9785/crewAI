import { render } from 'ink-testing-library';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { fakeRunner } from '../agents.js';
import { LocalStore } from '../config.js';
import { Engine } from '../engine.js';
import { MemoryStore } from '../memoryBackend.js';
import type { AgentName, Team } from '../types.js';
import { App } from './App.js';

const dirs: string[] = [];
const tmp = () => {
  const d = mkdtempSync(join(tmpdir(), 'crewai-ui-'));
  dirs.push(d);
  return d;
};
const cleanups: (() => unknown)[] = [];
afterEach(async () => {
  for (const c of cleanups.splice(0)) await c();
  dirs.splice(0).forEach((d) => rmSync(d, { recursive: true, force: true }));
});

const KEY = { enter: '\r', up: '\u001b[A', down: '\u001b[B', right: '\u001b[C', left: '\u001b[D', esc: '\u001b' };
const tick = (ms = 30) => new Promise((r) => setTimeout(r, ms));

async function mount(cloud: MemoryStore, uid: string, opts: { agents?: AgentName[]; settings?: { displayName: string; team: Team } } = {}) {
  const store = new LocalStore(tmp());
  if (opts.settings) store.saveSettings({ displayName: opts.settings.displayName, teamId: opts.settings.team.id, teamName: opts.settings.team.name });
  const runner = fakeRunner(opts.agents ?? ['codex'], (q, a) => `${a} answer to: ${q}`);
  const holder: { engine?: Engine } = {};
  const backend = cloud.backendFor(uid);
  const ui = render(<App backend={backend} store={store} runner={runner} folder={tmp()} holder={holder} engineOptions={{ tickMs: 0, heartbeatMs: 0 }} />);
  cleanups.push(() => ui.unmount(), () => holder.engine?.stop());
  const out = () => ui.frames.join('\n');
  const waitFor = (text: string) => vi.waitFor(() => expect(out()).toContain(text), { timeout: 4000 });
  const type = async (s: string) => {
    ui.stdin.write(s);
    await tick();
  };
  return { ui, store, holder, backend, runner, out, waitFor, type };
}

/** A teammate who is part of the team and runs headless. */
async function teammate(cloud: MemoryStore, team: Team, name: string, boss: ReturnType<MemoryStore['backendFor']>, agents: AgentName[] = ['claude']) {
  const backend = cloud.backendFor(`u-${name}`);
  await backend.joinTeam((await boss.createInvite(team.id)).code, name);
  const engine = new Engine({
    backend, store: new LocalStore(tmp()), runner: fakeRunner(agents, (q, a) => `${name}/${a}: ${q}`),
    folder: join(tmp(), `${name}-proj`), tickMs: 0, heartbeatMs: 0,
  });
  await engine.start(team);
  cleanups.push(() => engine.stop());
  return { backend, engine };
}

describe('onboarding and chat', () => {
  it('creates a team from first run, then answers a plain-text question from the local agent', async () => {
    const cloud = new MemoryStore();
    const m = await mount(cloud, 'u1');
    await m.waitFor('What should teammates call you?');
    await m.type('Sonu');
    await m.type(KEY.enter);
    await m.waitFor('Join a team');
    await m.type(KEY.down);
    await m.type(KEY.enter);
    await m.waitFor('Name your team');
    await m.type('Acme');
    await m.type(KEY.enter);
    await m.waitFor('Ask your agent, or type / for commands');
    expect(m.store.settings()).toMatchObject({ displayName: 'Sonu', teamName: 'Acme' });
    await m.type('what is this project?');
    await m.type(KEY.enter);
    await m.waitFor('codex answer to: what is this project?');
    await m.waitFor('your codex');
  });

  it('rejects a bad name and a bad code inline', async () => {
    const cloud = new MemoryStore();
    const m = await mount(cloud, 'u1');
    await m.waitFor('What should teammates call you?');
    await m.type('has space');
    await m.type(KEY.enter);
    await m.waitFor('no spaces');
    await m.type('\u0015'); // ctrl+u clears
    await m.type('Ann');
    await m.type(KEY.enter);
    await m.type(KEY.enter); // Join a team
    await m.waitFor('6-digit code');
    await m.type('12');
    await m.type(KEY.enter);
    await m.waitFor('The code is six digits.');
    await m.type('3456');
    await m.type(KEY.enter);
    await m.waitFor("That code isn't valid.");
  });

  it('shows the command menu and the teammate picker, and keeps the draft', async () => {
    const cloud = new MemoryStore();
    const boss = cloud.backendFor('u-boss');
    const team = await boss.createTeam('Acme', 'Boss');
    await teammate(cloud, team, 'Hemant', boss);
    const m = await mount(cloud, 'u-boss', { settings: { displayName: 'Boss', team } });
    await m.waitFor('Ask your agent, or type / for commands');
    await m.type('/');
    await m.waitFor('ask a project');
    expect(m.out()).toContain('/broadcast');
    await m.type(KEY.esc);
    await m.type('/task @he');
    await m.waitFor('Hemant');
    await m.type(KEY.enter); // picks the teammate, does not send
    await m.type('fix nav');
    await m.type(KEY.enter);
    await vi.waitFor(() => expect(m.holder.engine!.state.tasks.map((t) => t.text)).toEqual(['fix nav']));
    expect(m.holder.engine!.state.tasks[0]!.assigneeName).toBe('Hemant');
  });
});

describe('drafts', () => {
  it('puts a message back in the box when the teammate cannot be found', async () => {
    const cloud = new MemoryStore();
    const boss = cloud.backendFor('u-boss');
    const team = await boss.createTeam('Acme', 'Boss');
    const m = await mount(cloud, 'u-boss', { settings: { displayName: 'Boss', team } });
    await m.waitFor('Ask your agent, or type / for commands');
    await m.type('/task @zed fix the nav bar');
    await m.type(KEY.enter);
    await m.waitFor('Nobody called zed');
    await vi.waitFor(() => expect(m.ui.lastFrame()).toContain('/task @zed fix the nav bar'));
  });
});

describe('questions between people', () => {
  it('shows the owner an approval card; Enter alone does nothing, an explicit choice allows it', async () => {
    const cloud = new MemoryStore();
    const boss = cloud.backendFor('u-boss');
    const team = await boss.createTeam('Acme', 'Boss');
    const hem = await mount(cloud, 'u-hem');
    const code = (await boss.createInvite(team.id)).code;
    await hem.waitFor('What should teammates call you?');
    await hem.type('Hemant');
    await hem.type(KEY.enter);
    await hem.type(KEY.enter);
    await hem.waitFor('6-digit code');
    await hem.type(code);
    await hem.type(KEY.enter);
    await hem.waitFor('Ask your agent, or type / for commands');
    const bossEngine = new Engine({
      backend: boss, store: new LocalStore(tmp()), runner: fakeRunner(['codex'], () => ''), folder: tmp(), tickMs: 0, heartbeatMs: 0,
    });
    await bossEngine.start(team);
    bossEngine.attach(new LocalStore(tmp()).newSession(tmp()), Date.now());
    cleanups.push(() => bossEngine.stop());

    await bossEngine.submit('/read @hemant what is this project about');
    await hem.waitFor('Boss asks about your project');
    expect(hem.out()).toContain('Allow once');
    expect(hem.out()).toContain('Limited protection');
    await hem.type(KEY.enter); // nothing chosen yet: an Enter meant for the message box must not approve
    await tick(100);
    expect(bossEngine.state.entries.some((e) => e.kind === 'agent')).toBe(false);
    expect(hem.holder.engine!.state.attention[0]).toMatchObject({ type: 'consent' });
    await hem.type(KEY.right); // Allow once
    await hem.type(KEY.enter);
    await vi.waitFor(() => expect(bossEngine.state.entries.some((e) => e.kind === 'agent')).toBe(true));
    expect(bossEngine.state.entries.find((e) => e.kind === 'agent')!.text).toContain('what is this project about');
  });

  it('offers to wait or ask someone online when the teammate is away, and moves the question', async () => {
    const cloud = new MemoryStore();
    const boss = cloud.backendFor('u-boss');
    const team = await boss.createTeam('Acme', 'Boss');
    await teammate(cloud, team, 'Hemant', boss);
    const shw = await teammate(cloud, team, 'Shwetank', boss, ['codex']);
    cloud.members.get(team.id)!.get('u-Hemant')!.lastSeen = Date.now() - 10 * 60_000;
    cloud.changed();
    const m = await mount(cloud, 'u-boss', { settings: { displayName: 'Boss', team } });
    await m.waitFor('Ask your agent, or type / for commands');
    await m.type('/read @Hemant what is this project about');
    await m.type(KEY.enter);
    await m.waitFor('Hemant is offline');
    expect(m.out()).toContain('Wait for Hemant');
    expect(m.out()).toContain('Ask Shwetank');
    await m.type(KEY.right); // Wait for Hemant
    await m.type(KEY.right); // Ask Shwetank
    await m.type(KEY.enter);
    await vi.waitFor(() => expect(shw.engine.state.attention[0]).toMatchObject({ type: 'consent' }));
    expect(shw.engine.state.attention[0]!.request.askerName).toBe('Boss');
  });
});
