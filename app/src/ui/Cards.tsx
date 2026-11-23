import { Box, Text } from 'ink';
import type { Attention } from '../engine.js';
import type { Entry } from '../config.js';
import { ago, clip, hhmm } from '../format.js';
import { isOnline, type AgentName, type Member, type Task } from '../types.js';

export function EntryView({ e }: { e: Entry }) {
  const time = <Text dimColor>{hhmm(e.at)}</Text>;
  switch (e.kind) {
    case 'you':
      return (
        <Box flexDirection="column" marginTop={1}>
          <Text>
            <Text bold color="cyan">you</Text> {time}
          </Text>
          <Text>{e.text}</Text>
        </Box>
      );
    case 'agent':
      return (
        <Box flexDirection="column" marginTop={1}>
          <Text>
            <Text bold color="magenta">✦ {e.who}</Text> {time}
          </Text>
          <Text>{e.text}</Text>
          {e.meta ? <Text dimColor>{e.meta}</Text> : null}
        </Box>
      );
    case 'broadcast':
      return (
        <Box flexDirection="column" marginTop={1}>
          <Text>
            <Text bold color="yellow">{e.who}</Text> {time} <Text color="yellow">broadcast</Text>
          </Text>
          <Text>{e.text}</Text>
        </Box>
      );
    case 'task':
      return (
        <Text>
          <Text color="blue">▸ </Text>
          {e.text} {time}
        </Text>
      );
    default:
      return (
        <Text dimColor>
          {e.text}
        </Text>
      );
  }
}

export function StatusBar({
  team, folder, agent, me, members, now, error,
}: {
  team: string; folder: string; agent: AgentName | null; me: Member | null; members: Member[]; now: number; error: string;
}) {
  const sorted = [...members].sort((a, b) => (a.uid === me?.uid ? -1 : b.uid === me?.uid ? 1 : a.name.localeCompare(b.name)));
  return (
    <Box flexDirection="column" marginTop={1}>
      <Text>
        <Text bold>crewai</Text>
        <Text dimColor> · </Text>
        {team}
        <Text dimColor> · </Text>
        {folder}
        <Text dimColor> · agent: </Text>
        <Text color={agent ? 'magenta' : 'red'}>{agent ?? 'none'}</Text>
      </Text>
      <Text>
        {sorted.map((m, i) => {
          const on = isOnline(m, now);
          const label = m.uid === me?.uid ? 'you' : m.name;
          return (
            <Text key={m.uid}>
              {i > 0 ? '  ' : ''}
              <Text color={on ? 'green' : 'gray'}>{on ? '●' : '○'}</Text>
              <Text dimColor={!on}> {label}{on ? '' : ` (away ${ago(m.lastSeen, now).replace(' ago', '')})`}</Text>
            </Text>
          );
        })}
      </Text>
      {error ? <Text color="red">{error}</Text> : null}
    </Box>
  );
}

export interface CardOption {
  label: string;
  run: () => void;
}

function OptionRow({ options, selected, focused }: { options: CardOption[]; selected: number; focused: boolean }) {
  return (
    <Text>
      {options.map((o, i) => (
        <Text key={o.label}>
          {i > 0 ? '   ' : ''}
          {focused && i === selected ? <Text inverse color="cyan"> {o.label} </Text> : <Text> [{o.label}] </Text>}
        </Text>
      ))}
    </Text>
  );
}

export function AttentionCard({
  item, options, selected, focused, agent, folder,
}: {
  item: Attention; options: CardOption[]; selected: number; focused: boolean; agent: AgentName | null; folder: string;
}) {
  const r = item.request;
  if (item.type === 'consent') {
    return (
      <Box flexDirection="column" borderStyle="round" borderColor="yellow" paddingX={1} marginTop={1}>
        <Text bold color="yellow">{r.askerName} asks about your project</Text>
        <Text>"{clip(r.question, 300)}"</Text>
        <Text dimColor>
          Your {agent ?? 'agent'} will answer from {folder}. Limited protection: it can read any file in this folder. Only the
          answer is shared, and values that look like secrets are removed first, but that check is not perfect.
        </Text>
        <OptionRow options={options} selected={selected} focused={focused} />
        {focused ? <Text dimColor>← → pick an option, then enter (when the message box is empty)</Text> : null}
      </Box>
    );
  }
  if (item.compact) {
    return (
      <Box flexDirection="column">
        <Text>
          <Text color="yellow">◌ </Text>Waiting for {r.targetName} <Text dimColor>(away) · "{clip(r.question, 50)}"</Text>
        </Text>
        <OptionRow options={options} selected={selected} focused={focused} />
      </Box>
    );
  }
  return (
    <Box flexDirection="column" borderStyle="round" borderColor="yellow" paddingX={1} marginTop={1}>
      <Text bold color="yellow">
        {item.targetOnline ? `Waiting for ${r.targetName} to allow…` : `${r.targetName} is offline`}
      </Text>
      <Text dimColor>"{clip(r.question, 120)}" is saved{item.targetOnline ? '.' : ' and will reach them when they return.'}</Text>
      <OptionRow options={options} selected={selected} focused={focused} />
      {focused ? <Text dimColor>← → pick an option, then enter (when the message box is empty)</Text> : null}
    </Box>
  );
}

const GLYPH: Record<Task['status'], { g: string; c: string }> = {
  todo: { g: '○', c: 'white' },
  in_progress: { g: '▸', c: 'blue' },
  needs_help: { g: '!', c: 'red' },
  done: { g: '✓', c: 'green' },
  cancelled: { g: '✗', c: 'gray' },
};

export function sortTasks(tasks: Task[]): Task[] {
  const open = (t: Task) => t.status === 'todo' || t.status === 'in_progress' || t.status === 'needs_help';
  return [...tasks].sort((a, b) => Number(open(b)) - Number(open(a)) || b.updatedAt - a.updatedAt);
}

const PAGE = 12;

/** `rows` is already sorted; only a window of PAGE rows around the selection is drawn. */
export function TaskPanel({ rows, selected, meUid }: { rows: Task[]; selected: number; meUid: string }) {
  const start = Math.max(0, Math.min(selected - Math.floor(PAGE / 2), rows.length - PAGE));
  const visible = rows.slice(start, start + PAGE);
  return (
    <Box flexDirection="column" borderStyle="round" borderColor="blue" paddingX={1} marginTop={1}>
      <Text bold color="blue">
        Tasks{rows.length > PAGE ? <Text dimColor> {start + 1}–{start + visible.length} of {rows.length}</Text> : null}
      </Text>
      {rows.length === 0 ? <Text dimColor>No tasks yet. Try /task @name what to do.</Text> : null}
      {visible.map((t, j) => {
        const i = start + j;
        const { g, c } = GLYPH[t.status];
        return (
          <Text key={t.id} inverse={i === selected} wrap="truncate-end">
            <Text color={c}>{g}</Text> T-{t.num} {t.assigneeUid === meUid ? 'you' : t.assigneeName} · {clip(t.text, 60)}
            <Text dimColor> {t.status.replace('_', ' ')}</Text>
          </Text>
        );
      })}
      <Text dimColor>↑↓ choose · s start · d done · h need help · x cancel · esc close</Text>
    </Box>
  );
}
