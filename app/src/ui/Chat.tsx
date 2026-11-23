import { Box, Text, useInput } from 'ink';
import { basename } from 'node:path';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { ALLOW_DAYS } from '../config.js';
import type { Attention, Engine } from '../engine.js';
import { ago } from '../format.js';
import { isOnline } from '../types.js';
import { AttentionCard, type CardOption } from './Cards.js';
import { Composer } from './Composer.js';
import { buildFeed } from './feed.js';
import { ConversationPanel, Panel, TasksPanel, TeamPanel } from './Panels.js';
import { TaskView } from './TaskView.js';
import { FILTERS, buildRows, counts, indexOfTask, taskRows, type TaskFilter } from './tasklist.js';
import { layoutMode, useTerminalSize } from './useTerminal.js';
import { shortPath, useEngine } from './useEngine.js';

const rank = (a: Attention) => (a.type === 'consent' ? 0 : a.compact ? 2 : 1);
const LEFT_W = 32;

export function Chat({
  engine, folder, onQuit, onNew,
}: { engine: Engine; folder: string; onQuit: () => void; onNew: () => void }) {
  const s = useEngine(engine);
  const size = useTerminalSize();
  const mode = layoutMode(size.columns);
  const wide = mode === 'wide';

  const [history, setHistory] = useState<string[]>([]);
  const [view, setView] = useState<'chat' | 'tasks'>('chat');
  const [focus, setFocus] = useState<'composer' | 'tasks'>('composer');
  const [filter, setFilter] = useState<TaskFilter>('mine');
  const [showDone, setShowDone] = useState(false);
  const [taskSelId, setTaskSelId] = useState('');
  const [scroll, setScroll] = useState(0);
  const [convH, setConvH] = useState(10);
  const [composerEmpty, setComposerEmpty] = useState(true);
  // -1 means no option is chosen yet: Enter does nothing until the user picks one with an arrow key,
  // so an Enter meant for the message box can never approve a teammate's question.
  const [sel, setSel] = useState(-1);
  const [restore, setRestore] = useState<{ text: string; n: number } | undefined>();

  // ── conversation lines
  const convInner = (wide ? size.columns - LEFT_W - 1 : size.columns) - 4;
  const hour = Math.floor(s.now / 3_600_000);
  const lines = useMemo(() => buildFeed(s.entries, convInner, s.now), [s.entries, convInner, hour]);
  const prevLen = useRef(lines.length);
  useEffect(() => {
    // New lines must not move what the reader is looking at while scrolled up.
    const grew = lines.length - prevLen.current;
    prevLen.current = lines.length;
    if (grew > 0 && scroll > 0) setScroll((v) => v + grew);
  }, [lines.length]);
  const maxScroll = Math.max(0, lines.length - convH);
  const shownScroll = Math.min(scroll, maxScroll);

  // ── cards that need the user
  const cards = [...s.attention].sort((a, b) => rank(a) - rank(b));
  const optionsFor = (a: Attention): CardOption[] => {
    const id = a.request.id;
    if (a.type === 'consent') {
      return [
        { label: 'Allow once', run: () => void engine.decideConsent(id, 'once') },
        { label: `Allow ${a.request.askerName} for ${ALLOW_DAYS} days`, run: () => void engine.decideConsent(id, 'always') },
        { label: 'Decline', run: () => void engine.decideConsent(id, 'decline') },
      ];
    }
    const o: CardOption[] = [];
    if (!a.targetOnline && !a.compact) o.push({ label: `Wait for ${a.request.targetName}`, run: () => engine.waitFor(id) });
    for (const m of a.alternatives) o.push({ label: `Ask ${m.name}`, run: () => void engine.moveRequest(id, m.uid) });
    o.push({ label: 'Cancel', run: () => void engine.cancelRequest(id) });
    return o;
  };
  const top = cards[0];
  const topOptions = top ? optionsFor(top) : [];
  const topKey = top ? `${top.request.id}:${top.type}:${topOptions.length}` : '';
  useEffect(() => setSel(-1), [topKey]);

  const inChat = view === 'chat';
  useInput(
    (_input, key) => {
      const n = topOptions.length;
      if (key.leftArrow) setSel(sel < 0 ? n - 1 : (sel + n - 1) % n);
      else if (key.rightArrow) setSel(sel < 0 ? 0 : (sel + 1) % n);
      else if (key.return && sel >= 0) topOptions[Math.min(sel, n - 1)]!.run();
    },
    { isActive: inChat && focus === 'composer' && composerEmpty && topOptions.length > 0 },
  );

  // ── scrolling and focus
  useInput(
    (_input, key) => {
      const page = Math.max(1, convH - 2);
      if (key.pageUp) setScroll(Math.min(maxScroll, shownScroll + page));
      else if (key.pageDown) setScroll(Math.max(0, shownScroll - page));
      else if (key.tab && composerEmpty && focus === 'composer') {
        if (wide) setFocus('tasks');
        else setView('tasks');
      }
    },
    { isActive: inChat && focus === 'composer' },
  );

  // ── tasks panel (wide layout)
  const rows = buildRows(s.tasks, s.me?.uid ?? '', filter, showDone);
  const list = taskRows(rows);
  const selTask = (rows[indexOfTask(rows, taskSelId)] as { task?: { id: string } } | undefined)?.task;
  useInput(
    (input, key) => {
      if (key.escape || key.tab) return setFocus('composer');
      const at = Math.max(0, list.findIndex((t) => t.id === selTask?.id));
      if (key.upArrow && list.length) setTaskSelId(list[(at + list.length - 1) % list.length]!.id);
      else if (key.downArrow && list.length) setTaskSelId(list[(at + 1) % list.length]!.id);
      else if (key.return && selTask) setView('tasks');
      else if (input === 'm') setFilter(FILTERS[(FILTERS.indexOf(filter) + 1) % FILTERS.length]!);
      else if (input === 'v') setShowDone(!showDone);
      else if (selTask && input === 's') void engine.setTaskStatus(selTask.id, 'in_progress');
      else if (selTask && input === 'd') void engine.setTaskStatus(selTask.id, 'done');
      else if (selTask && input === 'h') void engine.setTaskStatus(selTask.id, 'needs_help');
      else if (selTask && input === 'x') void engine.setTaskStatus(selTask.id, 'cancelled');
    },
    { isActive: inChat && wide && focus === 'tasks' },
  );

  const submit = useCallback(
    async (text: string) => {
      setHistory((h) => [...h, text]);
      const out = await engine.submit(text);
      if (out.keepDraft) setRestore((r) => ({ text, n: (r?.n ?? 0) + 1 }));
      if (out.action === 'quit') onQuit();
      else if (out.action === 'new') onNew();
      else if (out.action === 'tasks') {
        if (wide) setFocus('tasks');
        else setView('tasks');
      }
    },
    [engine, onQuit, onNew, wide],
  );

  if (view === 'tasks') {
    return (
      <TaskView
        engine={engine}
        tasks={s.tasks}
        meUid={s.me?.uid ?? ''}
        size={size}
        narrow={mode === 'narrow'}
        team={s.team?.name ?? ''}
        folder={folder}
        initial={{ selId: selTask?.id ?? '', filter, showDone }}
        onClose={() => {
          setView('chat');
          setFocus('composer');
        }}
      />
    );
  }

  const folderName = basename(folder);
  const c = counts(s.tasks, s.me?.uid ?? '');
  const cardEls = cards.map((a, i) => (
    <AttentionCard
      key={`${a.type}:${a.request.id}`}
      item={a}
      options={optionsFor(a)}
      selected={Math.min(sel, topOptions.length - 1)}
      focused={i === 0 && focus === 'composer' && composerEmpty}
      agent={s.agent}
      folder={folderName}
      bare={wide}
      narrow={wide}
    />
  ));

  const online = s.members.filter((m) => isOnline(m, s.now));
  const hint = focus === 'tasks'
    ? '↑↓ select · enter open · s start · d done · h help · x cancel · m view · v done list · tab back'
    : wide
      ? '/ commands   @ people   tab tasks   pgup/pgdn scroll   ctrl+c quit'
      : '/ commands   @ people   tab tasks   pgup/pgdn scroll';

  return (
    <Box flexDirection="column" height={size.rows - 1} width={size.columns}>
      <Box justifyContent="space-between">
        <Text wrap="truncate-end">
          <Text bold color="cyan">◆ crewai</Text>
          <Text dimColor>  {s.team?.name}{mode === 'narrow' ? '' : ` · ${shortPath(folder)}`}</Text>
        </Text>
        {s.agent ? (
          <Text color="magenta">agent: {s.agent}</Text>
        ) : (
          <Text color="yellow">no agent · install Codex or Claude Code, then /agent</Text>
        )}
      </Box>
      {s.error ? <Text color="red" wrap="truncate-end">{s.error}</Text> : null}

      {mode === 'medium' ? (
        <Text wrap="truncate-end">
          {[...s.members]
            .sort((a, b) => (a.uid === s.me?.uid ? -1 : b.uid === s.me?.uid ? 1 : a.name.localeCompare(b.name)))
            .map((m, i) => {
              const on = isOnline(m, s.now);
              return (
                <Text key={m.uid}>
                  {i > 0 ? '  ' : ''}
                  <Text color={on ? 'green' : 'gray'}>{on ? '●' : '○'}</Text>
                  <Text dimColor={!on}> {m.uid === s.me?.uid ? 'you' : m.name}{on ? '' : ` ${ago(m.lastSeen, s.now).replace(' ago', '')}`}</Text>
                </Text>
              );
            })}
          <Text dimColor>   tasks </Text>
          <Text color="blue">▸ {c.doing}</Text>
          <Text dimColor> · </Text>
          <Text color="red">! {c.helping}</Text>
          <Text dimColor> · ○ {c.todo} · </Text>
          <Text color="green">✓ {c.done}</Text>
        </Text>
      ) : null}
      {mode === 'narrow' ? (
        <Text dimColor wrap="truncate-end">
          <Text color="green">●</Text> {online.length} online · {c.open} open tasks · /tasks
        </Text>
      ) : null}

      <Box flexGrow={1} flexBasis={0}>
        {wide ? (
          <Box flexDirection="column" width={LEFT_W} flexShrink={0}>
            <TeamPanel members={s.members} me={s.me} now={s.now} />
            <TasksPanel
              tasks={s.tasks}
              meUid={s.me?.uid ?? ''}
              filter={filter}
              showDone={showDone}
              selId={taskSelId}
              focused={focus === 'tasks'}
              innerWidth={LEFT_W - 4}
            />
            {cards.length > 0 ? (
              <Panel title={`NEEDS YOU · ${cards.length}`} color="yellow" flexShrink={0}>
                {cardEls}
              </Panel>
            ) : null}
          </Box>
        ) : null}
        <ConversationPanel lines={lines} scroll={shownScroll} onHeight={setConvH} marginLeft={wide ? 1 : 0} />
      </Box>

      {!wide ? cardEls : null}
      {s.busy ? <Text color="yellow" wrap="truncate-end">◌ {s.busy}</Text> : null}
      <Composer
        members={s.members}
        meUid={s.me?.uid ?? ''}
        history={history}
        now={s.now}
        onSubmit={(t) => void submit(t)}
        onEmptyChange={setComposerEmpty}
        restore={restore}
        active={focus === 'composer'}
        hint={hint}
        placeholder="Ask your agent, or type / for commands"
      />
    </Box>
  );
}
