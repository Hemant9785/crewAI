import { Terminal } from '@xterm/headless';
import { render } from 'ink';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { PassThrough, Writable } from 'node:stream';
import { afterEach, expect, it } from 'vitest';
import { fakeRunner } from '../agents.js';
import { LocalStore } from '../config.js';
import { MemoryStore } from '../memoryBackend.js';
import { App } from './App.js';
import { createFrameWriter } from './frameStream.js';

const tick = (ms: number) => new Promise((r) => setTimeout(r, ms));
const dirs: string[] = [];
afterEach(() => dirs.splice(0).forEach((d) => rmSync(d, { recursive: true, force: true })));

type Event = { chunk: string } | { resize: [number, number] };

const screenOf = (term: Terminal, rows: number): string[] =>
  Array.from({ length: rows }, (_, i) => term.buffer.active.getLine(i)?.translateToString(true) ?? '');
const feed = (term: Terminal, data: string) => new Promise<void>((r) => term.write(data, r));

/** Runs the real app with Ink's own renderer and records every byte it writes, and when the window resizes. */
async function record(): Promise<{ events: Event[]; cols: number; rows: number }> {
  const cloud = new MemoryStore();
  const boss = cloud.backendFor('u-boss');
  const team = await boss.createTeam('Acme', 'Boss');
  for (const n of ['Hemant', 'Ravi']) await cloud.backendFor(`u-${n}`).joinTeam((await boss.createInvite(team.id)).code, n);
  const items = Array.from({ length: 12 }, (_, i) => ({
    assigneeUid: 'u-boss', assigneeName: 'Boss', text: i === 0 ? 'Implement the submit button on the checkout page '.repeat(4) : `small task ${i + 1}`,
  }));
  for (let i = 0; i < items.length; i += 8) await boss.createTasks(team.id, items.slice(i, i + 8));
  const home = mkdtempSync(join(tmpdir(), 'crewai-replay-'));
  dirs.push(home);
  const store = new LocalStore(home);
  store.saveSettings({ displayName: 'Boss', teamId: team.id, teamName: 'Acme' });

  const events: Event[] = [];
  const stdout = Object.assign(new Writable({ write(c, _e, cb) { events.push({ chunk: String(c) }); cb(); } }), { columns: 120, rows: 36, isTTY: true });
  const stdin = Object.assign(new PassThrough(), { isTTY: true, setRawMode() { return stdin; }, ref() {}, unref() {} });
  const app = render(
    <App backend={boss} store={store} runner={fakeRunner(['codex'], (q) => `answer: ${q}`)} folder={join(home, 'web-app')} engineOptions={{ tickMs: 0, heartbeatMs: 0 }} />,
    { stdout: stdout as unknown as NodeJS.WriteStream, stdin: stdin as unknown as NodeJS.ReadStream, exitOnCtrlC: false, patchConsole: false },
  );
  const key = async (s: string, wait = 140) => {
    stdin.write(s);
    await tick(wait);
  };
  await tick(800);
  for (const ch of 'hello team') await key(ch, 40); // typing
  await key('\u007f'); // backspace
  await key('\u0015'); // clear
  await key('/'); // command menu
  await key('\u001b'); // close it
  await key('\t'); // focus the task panel
  await key('\u001b[B');
  await key('\u001b[B');
  await key('\r'); // open the task view
  await key('\u001b[C'); // detail
  await key('\u001b'); // back to the list
  await key('\u001b'); // back to chat
  await boss.sendBroadcast(team.id, 'Release freeze starts Friday. '.repeat(3)); // a teammate's message arrives
  await tick(300);
  await key('\u001b[5~'); // page up
  await key('what is this?');
  await key('\r'); // ask the local agent
  await tick(300);
  // The window is made smaller, then narrower than the split view.
  stdout.columns = 100;
  stdout.rows = 28;
  events.push({ resize: [100, 28] });
  stdout.emit('resize');
  await tick(300);
  await key('x');
  await tick(200);
  app.unmount();
  return { events, cols: 120, rows: 36 };
}

it('draws exactly the same screen as Ink does, after every frame, with far fewer bytes', async () => {
  const { events, cols, rows } = await record();
  let curCols = cols;
  let curRows = rows;
  const outB = { columns: curCols, rows: curRows, bytes: 0, chunks: [] as string[], write(c: string) { this.bytes += c.length; this.chunks.push(c); return true; } };
  const fw = createFrameWriter(outB);
  const termA = new Terminal({ cols, rows, allowProposedApi: true, convertEol: true });
  const termB = new Terminal({ cols, rows, allowProposedApi: true, convertEol: true });
  let bytesA = 0;
  let compared = 0;

  for (const ev of events) {
    if ('resize' in ev) {
      [curCols, curRows] = ev.resize;
      outB.columns = curCols;
      outB.rows = curRows;
      termA.resize(curCols, curRows);
      termB.resize(curCols, curRows);
      continue;
    }
    bytesA += ev.chunk.length;
    await feed(termA, ev.chunk);
    outB.chunks.length = 0;
    fw.write(ev.chunk);
    for (const c of outB.chunks) await feed(termB, c);
    // Compare once a whole frame has been drawn (Ink's own synchronized-output markers frame it).
    if (/\n/.test(ev.chunk)) {
      const a = screenOf(termA, curRows);
      const b = screenOf(termB, curRows);
      const bad = a.findIndex((l, i) => l !== b[i]);
      if (bad >= 0) {
        throw new Error(`frame ${compared}: row ${bad} differs\nInk:    ${JSON.stringify(a[bad])}\nwriter: ${JSON.stringify(b[bad])}\nsize ${curCols}x${curRows}`);
      }
      compared++;
    }
  }
  expect(compared).toBeGreaterThan(15); // many frames were checked
  expect(outB.bytes).toBeLessThan(bytesA * 0.4); // about 4x less overall, 25x less while typing
  const finalScreen = screenOf(termB, curRows).join('\n');
  expect(finalScreen).toContain('CONVERSATION');
}, 60000);
