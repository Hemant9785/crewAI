import { render } from 'ink-testing-library';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { expect, vi } from 'vitest';
import { fakeRunner } from '../agents.js';
import { LocalStore } from '../config.js';
import type { Engine } from '../engine.js';
import { MemoryStore } from '../memoryBackend.js';
import type { AgentName } from '../types.js';
import { App } from '../ui/App.js';

export const KEY = {
  enter: '\r', up: '\u001b[A', down: '\u001b[B', right: '\u001b[C', left: '\u001b[D',
  esc: '\u001b', tab: '\t', pgup: '\u001b[5~', pgdn: '\u001b[6~', ctrlU: '\u0015',
};
export const tick = (ms = 40) => new Promise((r) => setTimeout(r, ms));

export const LONG =
  'Implement the submit button on the checkout page: disabled until the form is valid, shows a spinner while the request runs, ' +
  'handles a 409 duplicate order by showing the existing order, and sends a confirmation event to analytics. Match the Figma spec.';

const dirs: string[] = [];
const cleanups: (() => unknown)[] = [];
const tmp = () => {
  const d = mkdtempSync(join(tmpdir(), 'crewai-world-'));
  dirs.push(d);
  return d;
};
/** Call from afterEach. */
export async function cleanupWorlds(): Promise<void> {
  for (const c of cleanups.splice(0)) await c();
  dirs.splice(0).forEach((d) => rmSync(d, { recursive: true, force: true }));
}

/** A team of Boss (the user), Hemant and Ravi with `n` tasks of mixed status assigned to Boss. */
export async function world(n: number, opts: { agents?: AgentName[] } = {}) {
  const cloud = new MemoryStore();
  const boss = cloud.backendFor('u-boss');
  const team = await boss.createTeam('Acme', 'Boss');
  for (const name of ['Hemant', 'Ravi']) {
    await cloud.backendFor(`u-${name}`).joinTeam((await boss.createInvite(team.id)).code, name);
  }
  const items = Array.from({ length: n }, (_, i) => ({
    assigneeUid: 'u-boss', assigneeName: 'Boss', text: i === 0 ? LONG : `small task ${i + 1}`,
  }));
  const made = [];
  for (let i = 0; i < items.length; i += 8) made.push(...(await boss.createTasks(team.id, items.slice(i, i + 8))));
  // 1 needs help, 3 in progress, the rest to do, and the last 5 done (when there are many).
  if (made[1]) await boss.updateTask(team.id, made[1].id, { status: 'needs_help' });
  for (const i of [2, 3, 4]) if (made[i]) await boss.updateTask(team.id, made[i]!.id, { status: 'in_progress' });
  if (n >= 12) for (const t of made.slice(-5)) await boss.updateTask(team.id, t.id, { status: 'done' });

  const store = new LocalStore(tmp());
  store.saveSettings({ displayName: 'Boss', teamId: team.id, teamName: 'Acme' });
  const holder: { engine?: Engine } = {};

  const mount = async (columns: number, rows = 36) => {
    const ui = render(
      <App backend={boss} store={store} runner={fakeRunner(opts.agents ?? ['codex'], (q) => `answer: ${q}`)} folder={join(tmp(), 'web-app')}
        holder={holder} engineOptions={{ tickMs: 0, heartbeatMs: 0 }} />,
    );
    cleanups.push(() => ui.unmount(), () => holder.engine?.stop());
    // Resize the fake terminal the way a real one resizes: set its size, then announce it.
    const out = ui.stdout as unknown as NodeJS.WriteStream;
    Object.defineProperty(out, 'columns', { value: columns, configurable: true });
    Object.defineProperty(out, 'rows', { value: rows, configurable: true });
    out.emit('resize');
    const type = async (s: string) => {
      ui.stdin.write(s);
      await tick();
    };
    const frame = () => ui.lastFrame() ?? '';
    const waitFor = (t: string) => vi.waitFor(() => expect(frame()).toContain(t), { timeout: 4000 });
    await waitFor('Ask your agent, or type / for commands');
    await tick(80);
    return { ui, type, frame, waitFor };
  };
  return { boss, team, cloud, mount, holder };
}
