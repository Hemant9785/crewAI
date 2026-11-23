import { describe, expect, it } from 'vitest';
import type { Entry } from '../config.js';
import type { Task } from '../types.js';
import { buildFeed } from './feed.js';
import { buildRows, counts, indexOfTask, peek, taskRows, windowRows, type Row } from './tasklist.js';
import { wrap, wrapClamp } from './textlayout.js';
import { layoutMode } from './useTerminal.js';

const task = (n: number, status: Task['status'], over: Partial<Task> = {}): Task => ({
  id: `id${n}`, num: n, assigneeUid: n % 3 === 0 ? 'me' : 'other', assigneeName: n % 3 === 0 ? 'Me' : 'Ravi',
  assignerUid: 'boss', assignerName: 'Boss', text: `task number ${n}`, status, note: '', createdAt: 1000 + n, updatedAt: 1000 + n, ...over,
});

describe('wrap', () => {
  it('wraps on spaces and never exceeds the width', () => {
    const lines = wrap('the quick brown fox jumps over the lazy dog', 12);
    expect(lines.every((l) => l.length <= 12)).toBe(true);
    expect(lines.join(' ')).toBe('the quick brown fox jumps over the lazy dog');
  });
  it('breaks a long word with no spaces', () => {
    const lines = wrap('x'.repeat(50), 20);
    expect(lines).toEqual(['x'.repeat(20), 'x'.repeat(20), 'x'.repeat(10)]);
  });
  it('keeps blank lines and indentation (code in an answer)', () => {
    expect(wrap('a\n\n  indented line', 40)).toEqual(['a', '', '  indented line']);
  });
  it('clamps to a number of lines with an ellipsis', () => {
    const out = wrapClamp('one two three four five six seven eight nine ten', 10, 2);
    expect(out).toHaveLength(2);
    expect(out[1]!.endsWith('…')).toBe(true);
    expect(wrapClamp('short', 10, 2)).toEqual(['short']);
  });
  it('survives tiny and empty input', () => {
    expect(wrap('', 10)).toEqual(['']);
    expect(wrap('abc def', 1).every((l) => l.length <= 4)).toBe(true);
  });
});

describe('layout modes', () => {
  it('picks the layout from the terminal width', () => {
    expect(layoutMode(140)).toBe('wide');
    expect(layoutMode(110)).toBe('wide');
    expect(layoutMode(109)).toBe('medium');
    expect(layoutMode(80)).toBe('medium');
    expect(layoutMode(79)).toBe('narrow');
  });
});

describe('task rows', () => {
  const tasks = [
    task(1, 'in_progress'), task(2, 'todo'), task(3, 'needs_help'), task(4, 'done'), task(5, 'todo'),
    task(6, 'in_progress'), task(7, 'cancelled'),
  ];
  it('groups by what needs attention first and collapses finished tasks', () => {
    const rows = buildRows(tasks, 'me', 'all', false);
    expect(rows.map((r) => (r.kind === 'group' ? r.label : r.kind === 'done' ? 'DONE' : 'task'))).toEqual([
      'NEEDS HELP', 'task', 'IN PROGRESS', 'task', 'task', 'TO DO', 'task', 'task', 'DONE',
    ]);
    expect(taskRows(rows)).toHaveLength(5);
  });
  it('shows finished tasks when expanded, and hides them in the open view', () => {
    expect(taskRows(buildRows(tasks, 'me', 'all', true))).toHaveLength(7);
    const open = buildRows(tasks, 'me', 'open', true);
    expect(open.some((r) => r.kind === 'done')).toBe(false);
    expect(taskRows(open)).toHaveLength(5);
  });
  it('limits the mine view to tasks assigned to you', () => {
    const mine = taskRows(buildRows(tasks, 'me', 'mine', true));
    expect(mine.every((t) => t.assigneeUid === 'me')).toBe(true);
    expect(mine.map((t) => t.num).sort()).toEqual([3, 6]);
  });
  it('counts tasks for the headers', () => {
    expect(counts(tasks, 'me')).toMatchObject({ total: 7, open: 5, mine: 2, helping: 1, doing: 2, todo: 2, done: 2 });
  });
  it('shows the first lines of a long task for the peek area', () => {
    const long = task(9, 'todo', { text: 'word '.repeat(80) });
    const lines = peek(long, 26);
    expect(lines).toHaveLength(2);
    expect(lines[1]!.endsWith('…')).toBe(true);
  });
});

describe('window of a long list', () => {
  const many = Array.from({ length: 20 }, (_, i) => task(i + 1, 'todo'));
  const rows: Row[] = buildRows(many, 'none', 'all', false);

  it('always keeps the selected row inside the window and respects the line budget', () => {
    for (const cap of [3, 5, 8, 12, 30]) {
      for (let i = 0; i < rows.length; i++) {
        const w = windowRows(rows, i, cap);
        const lines = w.rows.length + (w.above > 0 ? 1 : 0) + (w.below > 0 ? 1 : 0);
        expect(lines).toBeLessThanOrEqual(cap);
        expect(w.rows).toContain(rows[i]);
      }
    }
  });
  it('reports how many rows are hidden on each side', () => {
    const w = windowRows(rows, 0, 8);
    expect(w.above).toBe(0);
    expect(w.below).toBeGreaterThan(0);
    const end = windowRows(rows, rows.length - 1, 8);
    expect(end.below).toBe(0);
    expect(end.above).toBeGreaterThan(0);
  });
  it('shows everything when it fits', () => {
    expect(windowRows(rows, 3, 100)).toEqual({ rows, above: 0, below: 0 });
  });
  it('falls back to the first task when the selection is gone', () => {
    expect(indexOfTask(rows, 'missing')).toBe(rows.findIndex((r) => r.kind === 'task'));
  });
});

describe('conversation lines', () => {
  const now = new Date(2026, 10, 23, 12, 0, 0).getTime();
  const e = (kind: Entry['kind'], text: string, who = '', at = now, meta?: string): Entry => ({
    id: String(Math.random()), key: String(Math.random()), at, kind, who, text, meta,
  });
  const text = (lines: ReturnType<typeof buildFeed>) => lines.map((l) => l.map((s) => s.t).join(''));

  it('adds a day marker, a time column and a body under each speaker', () => {
    const lines = text(buildFeed([e('broadcast', 'Release freeze Friday.', 'Shwetank')], 60, now));
    expect(lines[0]).toBe('── Today ──');
    expect(lines.some((l) => /^\d\d:\d\d {2}Shwetank broadcast$/.test(l))).toBe(true);
    expect(lines.some((l) => l === '       Release freeze Friday.')).toBe(true);
  });
  it('wraps long answers inside the width and keeps the provenance line', () => {
    const lines = text(buildFeed([e('agent', 'alpha beta gamma delta epsilon zeta eta theta iota', "Ravi's claude", now, "answered on Ravi's laptop")], 30, now));
    expect(lines.every((l) => l.length <= 30)).toBe(true);
    expect(lines.slice(-2).join(' ').replace(/ +/g, ' ')).toContain("answered on Ravi's laptop");
  });
  it('separates days and keeps tasks and system notes compact', () => {
    const yesterday = now - 86_400_000;
    const lines = text(buildFeed([e('task', 'Shwetank assigned you T-1', '', yesterday), e('system', 'Ravi joined the team.'), e('system', 'Another note')], 60, now));
    expect(lines.filter((l) => l.startsWith('──'))).toEqual(['── Yesterday ──', '── Today ──']);
    expect(lines.some((l) => l.includes('▸ Shwetank assigned you T-1'))).toBe(true);
    const iJoined = lines.findIndex((l) => l.includes('Ravi joined'));
    expect(lines[iJoined + 1]).toContain('Another note');
  });
  it('does not fail on empty input or a tiny width', () => {
    expect(buildFeed([], 40, now)).toEqual([]);
    expect(buildFeed([e('you', 'hi')], 3, now).length).toBeGreaterThan(0);
  });
});
