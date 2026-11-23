#!/usr/bin/env node
import { render } from 'ink';
import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { realAgentRunner } from './agents.js';
import type { Backend } from './backend.js';
import { LocalStore } from './config.js';
import { createDemo } from './demo.js';
import type { Engine } from './engine.js';
import { FirebaseBackend } from './firebaseBackend.js';
import { SETUP_HINT, firebaseSource, loadFirebaseSettings } from './firebaseConfig.js';
import { App } from './ui/App.js';
import { withFrameWriter } from './ui/frameStream.js';
import { formatCode } from './format.js';

/** Replaced at build time by scripts/build-binary.mjs for the standalone binaries. */
declare const __CREWAI_VERSION__: string | undefined;

function version(): string {
  if (typeof __CREWAI_VERSION__ !== 'undefined') return __CREWAI_VERSION__;
  try {
    return (JSON.parse(readFileSync(new URL('../package.json', import.meta.url), 'utf8')) as { version: string }).version;
  } catch {
    return 'dev';
  }
}

const HELP = `crewai ${version()}

Team chat for coding agents. Run it in a project folder.

  crewai            open the chat
  crewai --demo     try it with simulated teammates (no account needed)
  crewai --check    check the connection to your team service
  crewai --version

Inside the chat: /read  /task  /broadcast  /tasks  /team  /agent  /new  /help
`;

async function main(): Promise<number> {
  const args = process.argv.slice(2);
  if (args.includes('--help') || args.includes('-h')) {
    process.stdout.write(HELP);
    return 0;
  }
  if (args.includes('--version') || args.includes('-v')) {
    process.stdout.write(`${version()}\n`);
    return 0;
  }
  if (args.includes('--config-status')) {
    // Offline check of where the team service settings come from (used by the release smoke test).
    process.stdout.write(`firebase config: ${firebaseSource(new LocalStore().dir)}\n`);
    return 0;
  }

  const demo = args.includes('--demo');
  let store = new LocalStore();
  let backend: Backend;
  let runner = realAgentRunner;
  let stopDemo: (() => Promise<void>) | null = null;

  if (demo) {
    const d = await createDemo();
    store = d.store;
    backend = d.backend;
    runner = d.runner;
    stopDemo = d.stop;
    process.stdout.write(`Demo team ready. To join it, choose "Join a team" and enter ${formatCode(d.code)}.\n`);
  } else {
    const envFile = join(store.dir, '.env');
    if (existsSync(envFile)) {
      try {
        process.loadEnvFile(envFile);
      } catch {
        /* ignore a malformed file */
      }
    }
    const cfg = loadFirebaseSettings(store.dir);
    if (!cfg) {
      process.stderr.write(`${SETUP_HINT}\n`);
      return 1;
    }
    try {
      backend = await FirebaseBackend.connect(cfg, store.identity());
    } catch (e) {
      process.stderr.write(`Couldn't connect: ${e instanceof Error ? e.message : 'unknown error'}\n`);
      return 1;
    }
    if (args.includes('--check')) {
      process.stdout.write(`Connected to Firebase project ${cfg.projectId}${cfg.emulator ? ' (emulator)' : ''}. This device is signed in.\n`);
      await backend.close();
      return 0;
    }
  }

  if (!process.stdin.isTTY) {
    process.stderr.write('crewai needs an interactive terminal.\n');
    await backend.close();
    return 1;
  }

  const holder: { engine?: Engine } = {};
  // Full-screen layout: use the terminal's alternate screen so the panels do not scroll into the
  // user's shell history, and always restore it, even on a crash or Ctrl+C.
  const fullScreen = Boolean(process.stdout.isTTY);
  const ESC = String.fromCharCode(27);
  const leave = () => {
    if (fullScreen) process.stdout.write(`${ESC}[?1049l${ESC}[?25h`);
  };
  if (fullScreen) process.stdout.write(`${ESC}[?1049h${ESC}[2J${ESC}[H`);
  process.once('exit', leave);
  let app;
  try {
    // The frame writer rewrites only the lines that changed. Ink's default erases and redraws the whole
    // screen on every keystroke (about 25 times more output with the split view), which visibly flickers.
    app = render(<App backend={backend} store={store} runner={runner} folder={process.cwd()} holder={holder} />, {
      stdout: fullScreen ? withFrameWriter(process.stdout) : process.stdout,
    });
    await app.waitUntilExit();
  } finally {
    leave();
  }
  await holder.engine?.stop();
  await stopDemo?.();
  await backend.close();
  return 0;
}

// A short pause lets network handles finish closing; exiting at once can trip a libuv assertion on Windows.
const exitSoon = (code: number) => setTimeout(() => process.exit(code), 150);
main().then(exitSoon, (e: unknown) => {
  process.stderr.write(`crewai stopped: ${e instanceof Error ? e.message : String(e)}\n`);
  exitSoon(1);
});
