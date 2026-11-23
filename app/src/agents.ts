import { spawn } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import type { AgentName, AgentPref } from './types.js';

export const AGENT_ORDER: AgentName[] = ['codex', 'claude'];
const MAX_OUTPUT = 8000;
const KILL_GRACE_MS = 3000;

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
  `Never reveal secrets: do not quote passwords, API keys, tokens, private keys, connection strings ` +
  `or the contents of .env and credential files, even if asked; say that the value is a secret instead. ` +
  `Answer in under 300 words, in plain language.\n\nQuestion: ${q}\n`;

/** Tools Claude must never use when answering, whatever the user's own Claude settings allow. */
const CLAUDE_DENY = 'Bash,Edit,Write,MultiEdit,NotebookEdit,WebFetch,WebSearch,Task';

/** Fixed arguments only. The question goes through stdin so it can never be parsed as a shell argument. */
export function buildCommand(agent: AgentName, outFile: string): { cmd: string; args: string[] } {
  if (agent === 'codex') {
    return {
      cmd: 'codex',
      args: ['exec', '--sandbox', 'read-only', '--skip-git-repo-check', '--ephemeral', '--color', 'never', '-o', outFile, '-'],
    };
  }
  // An explicit permission mode overrides a user default such as bypassPermissions; in -p mode
  // anything not allowed is refused rather than prompted.
  return {
    cmd: 'claude',
    args: ['-p', '--permission-mode', 'default', '--allowedTools', 'Read,Grep,Glob', '--disallowedTools', CLAUDE_DENY, '--output-format', 'text'],
  };
}

export function truncate(text: string, max = MAX_OUTPUT): string {
  const t = text.trim();
  return t.length <= max ? t : `${t.slice(0, max - 1)}…`;
}

const SECRET_PATTERNS: RegExp[] = [
  /-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----[\s\S]*?(?:-----END [A-Z0-9 ]*PRIVATE KEY-----|$)/g,
  /\b(?:AKIA|ASIA)[0-9A-Z]{16}\b/g, // AWS access key id
  /\bgh[pousr]_[A-Za-z0-9]{30,}\b/g, // GitHub tokens
  /\bgithub_pat_[A-Za-z0-9_]{30,}\b/g,
  /\bxox[abprs]-[A-Za-z0-9-]{10,}\b/g, // Slack
  /\bAIza[0-9A-Za-z_-]{35}\b/g, // Google API key
  /\bsk-(?:ant-)?[A-Za-z0-9_-]{20,}\b/g, // OpenAI / Anthropic
  /\b(?:sk|rk|pk)_(?:live|test)_[A-Za-z0-9]{16,}\b/g, // Stripe
  /\beyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\b/g, // JWT
  /(?<=\b[a-z][a-z0-9+.-]*:\/\/[^\s:/@]+:)[^\s@/]+(?=@)/gi, // password inside a URL
];
/** NAME=value or "name": "value" where the name looks like a secret. */
const SECRET_ASSIGN =
  /\b([A-Za-z0-9_.-]*(?:secret|token|passw(?:or)?d|pwd|api[_-]?key|private[_-]?key|access[_-]?key|credential)[A-Za-z0-9_.-]*)(["']?\s*[:=]\s*["']?)([^\s"',;]{4,})/gi;
const PLACEHOLDER = /^(?:\*+|x+|\.+|<[^>]*>|\$\{?[A-Za-z_][\w.]*\}?|process\.env\S*|true|false|null|none|undefined|\[secret)$/i;

/**
 * Best-effort removal of things that look like credentials from an answer before it leaves this
 * laptop. It cannot catch everything, so the consent card still says protection is limited.
 */
export function redactSecrets(text: string): { text: string; count: number } {
  let count = 0;
  let out = text;
  for (const re of SECRET_PATTERNS) {
    out = out.replace(re, () => {
      count++;
      return '[secret removed]';
    });
  }
  out = out.replace(SECRET_ASSIGN, (m: string, name: string, sep: string, value: string) => {
    if (PLACEHOLDER.test(value)) return m;
    count++;
    return `${name}${sep}[secret removed]`;
  });
  return { text: out, count };
}

export function pickAgent(pref: AgentPref, available: AgentName[]): { agent: AgentName | null; fellBack: boolean } {
  if (pref !== 'auto' && available.includes(pref)) return { agent: pref, fellBack: false };
  const first = AGENT_ORDER.find((a) => available.includes(a)) ?? null;
  return { agent: first, fellBack: pref !== 'auto' && first !== null };
}

function exec(cmd: string, args: string[], input: string | null, cwd: string, timeoutMs: number, signal?: AbortSignal) {
  return new Promise<{ code: number | null; stdout: string; stderr: string; timedOut: boolean }>((resolve) => {
    // shell is needed on Windows for .cmd shims. All args are fixed strings, never user text.
    // A single quoted command line avoids Node's warning about args with shell:true.
    const quote = (a: string) => (/^[\w./:\\=,-]+$/.test(a) ? a : `"${a.replace(/"/g, '')}"`);
    const win = process.platform === 'win32';
    const child = win
      ? spawn([cmd, ...args].map(quote).join(' '), [], { cwd, shell: true, windowsHide: true })
      : spawn(cmd, args, { cwd, windowsHide: true });
    let stdout = '';
    let stderr = '';
    let timedOut = false;
    let finished = false;
    let grace: NodeJS.Timeout | undefined;
    const finish = (code: number | null, fallbackErr = '') => {
      if (finished) return;
      finished = true;
      clearTimeout(timer);
      if (grace) clearTimeout(grace);
      signal?.removeEventListener('abort', kill);
      resolve({ code, stdout, stderr: stderr || fallbackErr, timedOut });
    };
    function kill(): void {
      if (finished) return;
      if (win && child.pid !== undefined) {
        // child.kill() would end only cmd.exe; the agent under it would keep running and keep the
        // output pipes open. taskkill /T ends the whole tree.
        try {
          spawn('taskkill', ['/pid', String(child.pid), '/T', '/F'], { windowsHide: true, stdio: 'ignore' }).on('error', () => {});
        } catch {
          /* fall back to child.kill below */
        }
      }
      try {
        child.kill();
      } catch {
        /* already gone */
      }
      // Never wait forever on a process that ignores the kill: settle shortly after asking it to stop.
      grace = setTimeout(() => {
        child.stdout?.destroy();
        child.stderr?.destroy();
        finish(null, 'stopped');
      }, KILL_GRACE_MS);
    }
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
    child.on('error', () => finish(null, 'could not start'));
    child.on('close', (code) => finish(code));
    child.stdin.on('error', () => {});
    if (input !== null) child.stdin.end(input);
    else child.stdin.end();
  });
}

/** Exported for tests: runs a command with the same timeout and kill handling as the agents. */
export const execForTest = exec;

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
      if (r.timedOut) return { ok: false, text: 'The agent took too long and was stopped.', agent, isolation: 'best-effort' };
      return interpretRun(agent, r.code, r.stdout, r.stderr, agent === 'codex' ? readOptional(outFile) : '');
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  },
};

function readOptional(path: string): string {
  try {
    return readFileSync(path, 'utf8');
  } catch {
    return '';
  }
}

/**
 * Decides whether a finished agent run produced an answer. A non-zero exit is a failure even if
 * something was printed (usually an error or usage text), unless Codex wrote its answer file.
 */
export function interpretRun(agent: AgentName, code: number | null, stdout: string, stderr: string, answerFile = ''): AgentResult {
  if (answerFile.trim()) return { ok: true, text: truncate(answerFile), agent, isolation: 'best-effort' };
  if (code === 0 && stdout.trim()) return { ok: true, text: truncate(stdout), agent, isolation: 'best-effort' };
  const detail = (stderr || stdout).replace(/\s+/g, ' ').trim().slice(0, 160);
  const base = code === 0 ? `The ${agent} agent gave no answer.` : `The ${agent} agent couldn't answer. Is it signed in?`;
  return { ok: false, text: detail ? `${base} (${detail})` : base, agent, isolation: 'best-effort' };
}

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
