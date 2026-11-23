import { Box, Text, type BoxProps } from 'ink';
import { useEffect, type ReactNode } from 'react';
import { clip, ago } from '../format.js';
import { isOnline, type Member, type Task } from '../types.js';
import type { Line } from './feed.js';
import { peek, buildRows, counts, indexOfTask, windowRows, FILTERS, type Row, type TaskFilter } from './tasklist.js';
import { wrap } from './textlayout.js';
import { useHeight } from './useTerminal.js';

export function Panel({
  title, right, color = 'gray', children, ...rest
}: { title: ReactNode; right?: ReactNode; color?: string; children?: ReactNode } & BoxProps) {
  return (
    <Box flexDirection="column" borderStyle="round" borderColor={color} paddingX={1} {...rest}>
      <Box justifyContent="space-between">
        <Text bold color={color === 'gray' ? undefined : color} dimColor={color === 'gray'}>{title}</Text>
        {right}
      </Box>
      {children}
    </Box>
  );
}

export function LineText({ line }: { line: Line }) {
  if (line.length === 0 || (line.length === 1 && line[0]!.t === '')) return <Text> </Text>;
  return (
    <Text wrap="truncate-end">
      {line.map((s, i) => (
        <Text key={i} color={s.color} bold={s.bold} dimColor={s.dim}>{s.t}</Text>
      ))}
    </Text>
  );
}

// ───────────────────────── conversation

export function ConversationPanel({
  lines, scroll, onHeight, focusedColor = 'gray', ...rest
}: { lines: Line[]; scroll: number; onHeight: (h: number) => void; focusedColor?: string } & BoxProps) {
  const [ref, h] = useHeight(10);
  useEffect(() => onHeight(h), [h]);
  const end = Math.max(0, lines.length - scroll);
  const start = Math.max(0, end - h);
  const visible = lines.slice(start, end);
  const newer = lines.length - end;
  const right = scroll > 0
    ? <Text color="yellow">↓ {newer} newer · pgdn</Text>
    : start > 0 ? <Text dimColor>↑ {start} earlier · pgup</Text> : null;
  return (
    <Panel title="CONVERSATION" right={right} color={focusedColor} flexGrow={1} flexBasis={0} {...rest}>
      <Box ref={ref} flexDirection="column" flexGrow={1} flexBasis={0} overflow="hidden" justifyContent="flex-end">
        {visible.length === 0 ? (
          <Text dimColor>Nothing yet. Type / for commands, @ to pick a teammate. Plain text asks your own agent about this folder.</Text>
        ) : (
          visible.map((l, i) => <LineText key={start + i} line={l} />)
        )}
      </Box>
    </Panel>
  );
}

// ───────────────────────── team

export function TeamPanel({ members, me, now, max = 6 }: { members: Member[]; me: Member | null; now: number; max?: number }) {
  const sorted = [...members].sort((a, b) => {
    if (a.uid === me?.uid) return -1;
    if (b.uid === me?.uid) return 1;
    const ao = isOnline(a, now);
    const bo = isOnline(b, now);
    return ao === bo ? (ao ? a.name.localeCompare(b.name) : b.lastSeen - a.lastSeen) : ao ? -1 : 1;
  });
  const shown = sorted.slice(0, max);
  return (
    <Panel title="TEAM" right={<Text dimColor>{members.length}</Text>} flexShrink={0}>
      {shown.map((m) => {
        const on = isOnline(m, now);
        return (
          <Text key={m.uid} wrap="truncate-end">
            <Text color={on ? 'green' : 'gray'}>{on ? '●' : '○'}</Text>
            <Text dimColor={!on}> {m.uid === me?.uid ? 'you' : m.name}</Text>
            <Text dimColor>{on ? (m.uid === me?.uid ? `  ${m.role}` : m.agent ? `  ${m.agent}` : '') : ` · ${ago(m.lastSeen, now).replace(' ago', '')}`}</Text>
          </Text>
        );
      })}
      {members.length > max ? <Text dimColor>+{members.length - max} more</Text> : null}
    </Panel>
  );
}

// ───────────────────────── tasks

export const GLYPH: Record<Task['status'], { g: string; c: string; word: string }> = {
  todo: { g: '○', c: 'white', word: 'to do' },
  in_progress: { g: '▸', c: 'blue', word: 'in progress' },
  needs_help: { g: '!', c: 'red', word: 'needs help' },
  done: { g: '✓', c: 'green', word: 'done' },
  cancelled: { g: '✗', c: 'gray', word: 'cancelled' },
};

const countTasks = (rows: Row[]) => rows.reduce((n, r) => n + (r.kind === 'task' ? 1 : 0), 0);

/** Rows of the task list, cut to `cap` lines around the selection, with "more" markers. */
export function TaskRows({
  rows, selId, focused, cap, meUid, filter,
}: { rows: Row[]; selId: string; focused: boolean; cap: number; meUid: string; filter: TaskFilter }) {
  const selIdx = indexOfTask(rows, selId);
  const w = windowRows(rows, selIdx, cap);
  const aboveTasks = countTasks(rows.slice(0, w.above));
  const belowTasks = countTasks(rows.slice(rows.length - w.below));
  const selected = (rows[selIdx] as { task?: Task } | undefined)?.task?.id;
  return (
    <>
      {w.above > 0 ? <Text dimColor>▲ {aboveTasks > 0 ? aboveTasks : w.above} more</Text> : null}
      {w.rows.map((r, i) => {
        if (r.kind === 'group') {
          return (
            <Text key={`g${r.id}`} bold color={r.color === 'white' ? undefined : r.color} dimColor={r.color === 'white'} wrap="truncate-end">
              {r.label} · {r.count}
            </Text>
          );
        }
        if (r.kind === 'done') {
          return (
            <Text key="done" color="green" wrap="truncate-end">
              ✓ {r.count} done <Text dimColor>· {r.expanded ? 'hide' : 'show'} (v)</Text>
            </Text>
          );
        }
        const t = r.task;
        const sel = t.id === selected;
        const who = filter !== 'mine' && t.assigneeUid !== meUid ? `${t.assigneeName} · ` : '';
        const { g, c } = GLYPH[t.status];
        return (
          <Text key={t.id} wrap="truncate-end" inverse={focused && sel} bold={sel && !focused}>
            {sel ? '›' : ' '}
            <Text color={c}>{g}</Text> T-{t.num} {who}{clip(t.text, 300)}
          </Text>
        );
      })}
      {w.below > 0 ? <Text dimColor>▼ {belowTasks > 0 ? belowTasks : w.below} more</Text> : null}
    </>
  );
}

export function FilterChips({ filter }: { filter: TaskFilter }) {
  return (
    <Text>
      {FILTERS.map((f) => (
        <Text key={f}>
          {filter === f ? <Text inverse color="cyan"> {f[0]!.toUpperCase() + f.slice(1)} </Text> : <Text dimColor> {f[0]!.toUpperCase() + f.slice(1)} </Text>}
        </Text>
      ))}
    </Text>
  );
}

export function TasksPanel({
  tasks, meUid, filter, showDone, selId, focused, innerWidth, ...rest
}: {
  tasks: Task[]; meUid: string; filter: TaskFilter; showDone: boolean; selId: string; focused: boolean; innerWidth: number;
} & BoxProps) {
  const rows = buildRows(tasks, meUid, filter, showDone);
  const [ref, h] = useHeight(6);
  const c = counts(tasks, meUid);
  const sel = (rows[indexOfTask(rows, selId)] as { task?: Task } | undefined)?.task;
  const lines = sel ? peek(sel, innerWidth) : [];
  return (
    <Panel
      title={`TASKS · ${tasks.length}`}
      right={<Text dimColor>{c.mine} mine</Text>}
      color={focused ? 'cyan' : 'blue'}
      flexGrow={1}
      flexBasis={0}
      {...rest}
    >
      <FilterChips filter={filter} />
      <Box ref={ref} flexDirection="column" flexGrow={1} flexBasis={0} overflow="hidden">
        {rows.length === 0 ? (
          <Text dimColor>{tasks.length === 0 ? 'No tasks yet. Try /task @name what to do.' : 'Nothing here. Press m for another view.'}</Text>
        ) : (
          <TaskRows rows={rows} selId={selId} focused={focused} cap={h} meUid={meUid} filter={filter} />
        )}
      </Box>
      <Box flexDirection="column" flexShrink={0} height={5} borderStyle="single" borderColor="gray" borderLeft={false} borderRight={false} borderBottom={false}>
        <Text dimColor wrap="truncate-end">
          {sel ? `T-${sel.num} · ${sel.assigneeUid === meUid ? 'you' : sel.assigneeName} · from ${sel.assignerName}` : ' '}
        </Text>
        <Text wrap="truncate-end">{lines[0] ?? ' '}</Text>
        <Text wrap="truncate-end">{lines[1] ?? ' '}</Text>
        <Text color={focused ? 'blue' : undefined} dimColor={!focused}>{sel ? (focused ? 'enter to open' : 'tab to focus') : ' '}</Text>
      </Box>
    </Panel>
  );
}

