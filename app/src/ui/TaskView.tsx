import { Box, Text, useInput } from 'ink';
import { useEffect, useState } from 'react';
import type { Engine } from '../engine.js';
import { hhmm } from '../format.js';
import type { Task, TaskStatus } from '../types.js';
import { FilterChips, GLYPH, Panel, TaskRows } from './Panels.js';
import { FILTERS, buildRows, indexOfTask, taskRows, type TaskFilter } from './tasklist.js';
import { wrap } from './textlayout.js';
import { shortPath } from './useEngine.js';
import { useHeight, type Size } from './useTerminal.js';

type Pane = 'list' | 'detail';

function timeline(t: Task): { at: number; text: string }[] {
  const out = [{ at: t.createdAt, text: `${t.assignerName} assigned it` }];
  if (t.updatedAt - t.createdAt > 1000) {
    out.push({ at: t.updatedAt, text: `${GLYPH[t.status].word}${t.note ? ` — ${t.note}` : ''}` });
  }
  return out;
}

export function TaskView({
  engine, tasks, meUid, size, narrow, team, folder, initial, onClose,
}: {
  engine: Engine; tasks: Task[]; meUid: string; size: Size; narrow: boolean; team: string; folder: string;
  /** Where the list was when the view opened, so the same task stays selected. */
  initial?: { selId: string; filter: TaskFilter; showDone: boolean };
  onClose: () => void;
}) {
  const [filter, setFilter] = useState<TaskFilter>(initial?.filter ?? 'mine');
  const [showDone, setShowDone] = useState(initial?.showDone ?? false);
  const [selId, setSelId] = useState(initial?.selId ?? '');
  const [pane, setPane] = useState<Pane>('list');
  const [scroll, setScroll] = useState(0);

  const rows = buildRows(tasks, meUid, filter, showDone);
  const list = taskRows(rows);
  const selIdx = indexOfTask(rows, selId);
  const sel = (rows[selIdx] as { task?: Task } | undefined)?.task;
  useEffect(() => setScroll(0), [sel?.id]);

  const act = (status: TaskStatus) => {
    if (sel) void engine.setTaskStatus(sel.id, status);
  };

  useInput((input, key) => {
    if (key.escape || input === 'q') {
      if (pane === 'detail') setPane('list');
      else onClose();
      return;
    }
    if (input === 's') return act('in_progress');
    if (input === 'd') return act('done');
    if (input === 'h') return act('needs_help');
    if (input === 'x') return act('cancelled');
    if (pane === 'list') {
      const at = Math.max(0, list.findIndex((t) => t.id === sel?.id));
      if (key.upArrow && list.length) setSelId(list[(at + list.length - 1) % list.length]!.id);
      else if (key.downArrow && list.length) setSelId(list[(at + 1) % list.length]!.id);
      else if ((key.rightArrow || key.return) && sel) setPane('detail');
      else if (input === 'm') setFilter(FILTERS[(FILTERS.indexOf(filter) + 1) % FILTERS.length]!);
      else if (input === 'v') setShowDone(!showDone);
    } else if (key.upArrow) setScroll((s) => Math.max(0, s - 1));
    else if (key.downArrow) setScroll((s) => s + 1);
    else if (key.pageUp) setScroll((s) => Math.max(0, s - 5));
    else if (key.pageDown) setScroll((s) => s + 5);
    else if (key.leftArrow) setPane('list');
  });

  const listW = narrow ? size.columns : 38;
  const detailW = narrow ? size.columns : size.columns - listW - 1;
  const showList = !narrow || pane === 'list';
  const showDetail = !narrow || pane === 'detail';
  const [listRef, listH] = useHeight(8);
  // The detail panel's body is measured once; the description gets whatever the fixed rows below it leave.
  const [bodyRef, bodyH] = useHeight(12);

  const lines = sel ? wrap(sel.text, detailW - 4) : [];
  const fixedRows = 1 + (sel ? timeline(sel).length : 0) + 1 + 2; // meta, timeline, actions, spacer and scroll note
  const descH = Math.max(2, bodyH - fixedRows);
  const maxScroll = Math.max(0, lines.length - descH);
  const from = Math.min(scroll, maxScroll);
  const shown = lines.slice(from, from + descH);

  const actions: string[] = [];
  if (sel) {
    if (sel.status !== 'in_progress' && sel.status !== 'done') actions.push('[s] Start');
    if (sel.status !== 'done') actions.push('[d] Done');
    if (sel.status !== 'needs_help' && sel.status !== 'done') actions.push('[h] Need help');
    if (sel.status !== 'cancelled' && sel.status !== 'done') actions.push('[x] Cancel');
  }

  return (
    <Box flexDirection="column" height={size.rows - 1} width={size.columns}>
      <Box justifyContent="space-between">
        <Text>
          <Text bold color="cyan">◆ crewai</Text>
          <Text dimColor>  {team} · {shortPath(folder)} · Tasks</Text>
        </Text>
        <Text dimColor>esc back to chat</Text>
      </Box>
      <Box flexGrow={1} flexBasis={0}>
        {showList ? (
          <Panel
            title={`${tasks.length} TASKS`}
            right={<FilterChips filter={filter} />}
            color={pane === 'list' ? 'cyan' : 'gray'}
            width={listW}
            flexShrink={0}
          >
            <Box ref={listRef} flexDirection="column" flexGrow={1} flexBasis={0} overflow="hidden">
              {rows.length === 0 ? (
                <Text dimColor>{tasks.length === 0 ? 'No tasks yet.' : 'Nothing here. Press m.'}</Text>
              ) : (
                <TaskRows rows={rows} selId={selId} focused={pane === 'list'} cap={listH} meUid={meUid} filter={filter} />
              )}
            </Box>
          </Panel>
        ) : null}
        {showDetail ? (
          <Panel
            title={sel ? `T-${sel.num}` : 'TASK'}
            right={sel ? <Text color={GLYPH[sel.status].c}>{GLYPH[sel.status].g} {GLYPH[sel.status].word}</Text> : null}
            color={pane === 'detail' ? 'cyan' : 'blue'}
            flexGrow={1}
            flexBasis={0}
            marginLeft={narrow ? 0 : 1}
          >
            {sel ? (
              <>
                <Box ref={bodyRef} flexDirection="column" flexGrow={1} flexBasis={0} overflow="hidden">
                <Box flexDirection="column" height={shown.length}>
                  {shown.map((l, i) => (
                    <Text key={from + i}>{l === '' ? ' ' : l}</Text>
                  ))}
                </Box>
                <Box flexDirection="column" flexShrink={0} marginTop={1}>
                  {lines.length > descH ? (
                    <Text dimColor>lines {from + 1}–{from + shown.length} of {lines.length}{from < maxScroll ? ' · ↓ to read more' : ''}</Text>
                  ) : null}
                  <Text>
                    <Text dimColor>From </Text>{sel.assignerName}   <Text dimColor>To </Text>{sel.assigneeUid === meUid ? 'you' : sel.assigneeName}
                  </Text>
                  <Box flexDirection="column" borderStyle="single" borderColor="gray" borderTop={false} borderRight={false} borderBottom={false} paddingLeft={1}>
                    {timeline(sel).map((e) => (
                      <Text key={e.at + e.text} wrap="truncate-end"><Text dimColor>{hhmm(e.at)}</Text>  {e.text}</Text>
                    ))}
                  </Box>
                  <Text color="cyan">{actions.join('   ') || ' '}</Text>
                </Box>
                </Box>
              </>
            ) : (
              <Text dimColor>Pick a task on the left.</Text>
            )}
          </Panel>
        ) : null}
      </Box>
      <Text dimColor wrap="truncate-end">
        {pane === 'list'
          ? `↑↓ choose · ${narrow ? 'enter open' : '→ detail'} · m view · v done · s d h x act · esc back`
          : '↑↓ read · ← list · s d h x act · esc back'}
      </Text>
    </Box>
  );
}
