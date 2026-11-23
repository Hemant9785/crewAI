import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import type { AgentRunner } from './agents.js';
import type { Backend } from './backend.js';
import { LocalStore } from './config.js';
import { Engine } from './engine.js';
import { MemoryStore } from './memoryBackend.js';
import type { AgentName } from './types.js';

export const DEMO_UID = 'demo-you';

function scripted(who: string, delayMs: number, available: AgentName[]): AgentRunner {
  return {
    detect: async () => available,
    run: async (agent, question) => {
      await new Promise((r) => setTimeout(r, delayMs));
      return { ok: true, agent, isolation: 'best-effort', text: `(demo) ${who}'s ${agent} would read the project here and answer: "${question}"` };
    },
  };
}

/**
 * A self-contained team with simulated teammates, for trying the interface without Firebase.
 * Nothing leaves this process.
 */
export async function createDemo() {
  const cloud = new MemoryStore();
  const dir = mkdtempSync(join(tmpdir(), 'crewai-demo-'));
  const store = new LocalStore(dir);
  const temps: string[] = [];

  const mk = async (name: string, uid: string, agents: AgentName[], boss?: Backend, code?: string) => {
    const backend = cloud.backendFor(uid);
    const team = boss ? await backend.joinTeam(code!, name) : await backend.createTeam('Acme', name);
    const localDir = mkdtempSync(join(tmpdir(), 'crewai-demo-p-'));
    temps.push(localDir);
    const local = new LocalStore(localDir);
    local.allowAlways(team.id, DEMO_UID);
    const engine = new Engine({ backend, store: local, runner: scripted(name, 1200, agents), folder: join(dir, `${name}-project`) });
    await engine.start(team);
    return { backend, team, engine };
  };

  const priya = await mk('Priya', 'demo-priya', ['codex']);
  const invite = async () => (await priya.backend.createInvite(priya.team.id)).code;
  const hemant = await mk('Hemant', 'demo-hemant', ['claude'], priya.backend, await invite());
  const shwetank = await mk('Shwetank', 'demo-shwetank', ['codex'], priya.backend, await invite());
  const ravi = cloud.backendFor('demo-ravi');
  await ravi.joinTeam(await invite(), 'Ravi');
  cloud.members.get(priya.team.id)!.get('demo-ravi')!.lastSeen = Date.now() - 2 * 3_600_000;

  const code = await invite();
  const seed = setInterval(() => {
    if (!cloud.members.get(priya.team.id)?.has(DEMO_UID)) return;
    clearInterval(seed);
    setTimeout(() => {
      void shwetank.backend.sendBroadcast(priya.team.id, 'Release freeze starts Friday.');
      void shwetank.backend.createTasks(priya.team.id, [{ assigneeUid: DEMO_UID, assigneeName: store.settings().displayName || 'You', text: 'fix the flaky login test' }]).catch(() => {});
    }, 4000);
  }, 500);
  seed.unref?.();

  return {
    backend: cloud.backendFor(DEMO_UID),
    store,
    runner: scripted('Your', 900, ['codex', 'claude']),
    code,
    async stop() {
      clearInterval(seed);
      await Promise.all([priya, hemant, shwetank].map((p) => p.engine.stop()));
      // Remove the demo's temporary folders.
      for (const d of [dir, ...temps]) rmSync(d, { recursive: true, force: true });
    },
  };
}
