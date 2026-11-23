import { spawn } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import type { AgentName, AgentPref } from './types.js';

export const AGENT_ORDER: AgentName[] = ['codex', 'claude'];
const MAX_OUTPUT = 8000;

export interface AgentResult {
  ok: boolean;
  text: string;
  agent: AgentName;
  /** Never claims OS-enforced read-only. */
  isolation: 'best-effort';
}

export interface AgentRunner {
  detect(): Promise<AgentName[]>;
  run(agent: AgentName, question: string, cwd: string, opts?: { signal?: AbortSignal; timeoutMs?: number }): Promise<AgentResult>;
}

const WRAP = (q: string) =>
  `You are answering a teammate's question about the repository in the current directory. ` +
  `Read files as needed, but do not modify anything and do not run commands that change state. ` +
  `Answer in under 300 words, in plain language.\n\nQuestion: ${q}\n`;

/** Fixed arguments only. The question goes through stdin so it can never be parsed as a shell argument. */
export function buildCommand(agent: AgentName, outFile: string): { cmd: string; args: string[] } {
  if (agent === 'codex') {
    return {
      cmd: 'codex',
      args: ['exec', '--sandbox', 'read-only', '--skip-git-repo-check', '--ephemeral', '--color', 'never', '-o', outFile, '-'],
    };
  }
  return { cmd: 'claude', args: ['-p', '--allowedTools', 'Read,Grep,Glob', '--output-format', 'text'] };
}

export function truncate(text: string, max = MAX_OUTPUT): string {
  const t = text.trim();
  return t.length <= max ? t : `${t.slice(0, max - 1)}…`;
}

export function pickAgent(pref: AgentPref, available: AgentName[]): { agent: AgentName | null; fellBack: boolean } {
  if (pref !== 'auto' && available.includes(pref)) return { agent: pref, fellBack: false };
  const first = AGENT_ORDER.find((a) => available.includes(a)) ?? null;
  return { agent: first, fellBack: pref !== 'auto' && first !== null };
}

function exec(cmd: string, args: string[], input: string | null, cwd: string, timeoutMs: number, signal?: AbortSignal) {
  return new Promise<{ code: number | null; stdout: string; stderr: string; timedOut: boolean }>((resolve) => {
    // shell is needed on Windows for .cmd shims. All args are fixed strings, never user text.
    const child = spawn(cmd, args, { cwd, shell: process.platform === 'win32', windowsHide: true });
    let stdout = '';
    let stderr = '';
    let timedOut = false;
    const kill = () => {
      try {
        child.kill();
      } catch {
        /* already gone */
      }
    };
    const timer = setTimeout(() => {
      timedOut = true;
      kill();
    }, timeoutMs);
    signal?.addEventListener('abort', kill, { once: true });
    child.stdout.on('data', (d: Buffer) => {
      if (stdout.length < MAX_OUTPUT * 4) stdout += d.toString();
    });
    child.stderr.on('data', (d: Buffer) => {
      if (stderr.length < 4000) stderr += d.toString();
    });
    child.on('error', () => {
      clearTimeout(timer);
      resolve({ code: null, stdout, stderr: stderr || 'could not start', timedOut });
    });
    child.on('close', (code) => {
      clearTimeout(timer);
      resolve({ code, stdout, stderr, timedOut });
    });
    child.stdin.on('error', () => {});
    if (input !== null) child.stdin.end(input);
    else child.stdin.end();
  });
}

export const realAgentRunner: AgentRunner = {
  async detect() {
    const found: AgentName[] = [];
    await Promise.all(
      AGENT_ORDER.map(async (a) => {
        const r = await exec(a, ['--version'], null, process.cwd(), 8000);
        if (r.code === 0) found.push(a);
      }),
    );
    return AGENT_ORDER.filter((a) => found.includes(a));
  },

  async run(agent, question, cwd, opts = {}) {
    const dir = mkdtempSync(join(tmpdir(), 'crewai-'));
    const outFile = join(dir, 'answer.txt');
    try {
      const { cmd, args } = buildCommand(agent, outFile);
      const r = await exec(cmd, args, WRAP(question), cwd, opts.timeoutMs ?? 120_000, opts.signal);
      let text = '';
      if (agent === 'codex') {
        try {
          text = readFileSync(outFile, 'utf8');
        } catch {
          text = '';
        }
      }
      if (!text) text = r.stdout;
      if (r.timedOut) return { ok: false, text: 'The agent took too long and was stopped.', agent, isolation: 'best-effort' };
      if (r.code !== 0 && !text.trim()) {
        return { ok: false, text: `The ${agent} agent couldn't answer. Is it signed in?`, agent, isolation: 'best-effort' };
      }
      return { ok: true, text: truncate(text), agent, isolation: 'best-effort' };
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  },
};

/** Scripted runner for tests and demos. */
export function fakeRunner(available: AgentName[], answer: (q: string, agent: AgentName) => string): AgentRunner & { calls: string[] } {
  const calls: string[] = [];
  return {
    calls,
    async detect() {
      return available;
    },
    async run(agent, question) {
      calls.push(question);
      return { ok: true, text: answer(question, agent), agent, isolation: 'best-effort' };
    },
  };
}
