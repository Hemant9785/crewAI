import type { Task } from '../types.js';
import { wrapClamp } from './textlayout.js';

export type TaskFilter = 'mine' | 'all' | 'open';
export const FILTERS: TaskFilter[] = ['mine', 'all', 'open'];

const isOpen = (t: Task) => t.status === 'todo' || t.status === 'in_progress' || t.status === 'needs_help';

export type Row =
  | { kind: 'group'; id: string; label: string; count: number; color: string }
  | { kind: 'task'; task: Task }
  | { kind: 'done'; count: number; expanded: boolean };

const GROUPS: { status: Task['status']; label: string; color: string }[] = [
  { status: 'needs_help', label: 'NEEDS HELP', color: 'red' },
  { status: 'in_progress', label: 'IN PROGRESS', color: 'blue' },
  { status: 'todo', label: 'TO DO', color: 'white' },
];

export function visibleTasks(tasks: Task[], meUid: string, filter: TaskFilter): Task[] {
  if (filter === 'mine') return tasks.filter((t) => t.assigneeUid === meUid);
  if (filter === 'open') return tasks.filter(isOpen);
  return tasks;
}

const newest = (a: Task, b: Task) => b.updatedAt - a.updatedAt || b.num - a.num;

/** Groups by what needs attention first. Finished tasks collapse into one line unless expanded. */
export function buildRows(tasks: Task[], meUid: string, filter: TaskFilter, showDone: boolean): Row[] {
  const list = visibleTasks(tasks, meUid, filter);
  const rows: Row[] = [];
  for (const g of GROUPS) {
    const inGroup = list.filter((t) => t.status === g.status).sort(newest);
    if (!inGroup.length) continue;
    rows.push({ kind: 'group', id: g.status, label: g.label, count: inGroup.length, color: g.color });
    for (const t of inGroup) rows.push({ kind: 'task', task: t });
  }
  const finished = list.filter((t) => !isOpen(t)).sort(newest);
  if (filter !== 'open' && finished.length) {
    rows.push({ kind: 'done', count: finished.length, expanded: showDone });
    if (showDone) for (const t of finished) rows.push({ kind: 'task', task: t });
  }
  return rows;
}

export const taskRows = (rows: Row[]): Task[] => rows.flatMap((r) => (r.kind === 'task' ? [r.task] : []));

export function indexOfTask(rows: Row[], id: string): number {
  const i = rows.findIndex((r) => r.kind === 'task' && r.task.id === id);
  if (i >= 0) return i;
  return rows.findIndex((r) => r.kind === 'task');
}

export interface Windowed {
  rows: Row[];
  above: number;
  below: number;
}

/**
 * At most `cap` lines, including one line each for "more above" and "more below" markers,
 * always keeping the selected row visible.
 */
export function windowRows(rows: Row[], selIdx: number, cap: number): Windowed {
  const total = rows.length;
  if (cap < 3 || total <= cap) return { rows: rows.slice(0, Math.max(0, cap)), above: 0, below: Math.max(0, total - cap) };
  const sel = Math.max(0, selIdx);
  let start = Math.max(0, Math.min(sel - Math.floor((cap - 2) / 2), total - cap));
  for (let i = 0; i < 4; i++) {
    let room = cap - (start > 0 ? 1 : 0);
    let end = Math.min(total, start + room);
    if (end < total) {
      room -= 1;
      end = Math.min(total, start + room);
    }
    if (sel < start) start = sel;
    else if (sel >= end) start = sel - room + 1;
    else return { rows: rows.slice(start, end), above: start, below: total - end };
  }
  return { rows: rows.slice(sel, sel + Math.max(1, cap - 2)), above: sel, below: Math.max(0, total - sel - (cap - 2)) };
}

export function counts(tasks: Task[], meUid: string) {
  return {
    total: tasks.length,
    open: tasks.filter(isOpen).length,
    mine: tasks.filter((t) => t.assigneeUid === meUid && isOpen(t)).length,
    helping: tasks.filter((t) => t.status === 'needs_help').length,
    doing: tasks.filter((t) => t.status === 'in_progress').length,
    todo: tasks.filter((t) => t.status === 'todo').length,
    done: tasks.filter((t) => !isOpen(t)).length,
  };
}

/** The first lines of a task, for the peek area under the list. */
export const peek = (task: Task, width: number): string[] => wrapClamp(task.text, width, 2);
