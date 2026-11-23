import { Box, Text } from 'ink';
import type { SessionSummary } from '../config.js';
import type { Engine } from '../engine.js';
import { clip, dayLabel } from '../format.js';
import { Select } from './Select.js';
import { shortPath, useEngine } from './useEngine.js';

export function Resume({
  engine, sessions, folder, onPick, onNew,
}: {
  engine: Engine;
  sessions: SessionSummary[];
  folder: string;
  onPick: (id: string) => void;
  onNew: () => void;
}) {
  const s = useEngine(engine);
  const approvals = s.attention.filter((a) => a.type === 'consent').length;
  const myTasks = s.tasks.filter((t) => t.assigneeUid === s.me?.uid && t.status === 'todo').length;
  const needs: string[] = [];
  if (approvals) needs.push(`${approvals} approval${approvals > 1 ? 's' : ''}`);
  if (myTasks) needs.push(`${myTasks} new task${myTasks > 1 ? 's' : ''}`);
  const total = approvals + myTasks;

  return (
    <Box flexDirection="column" marginTop={1}>
      <Text>
        <Text bold>crewai</Text>
        <Text dimColor> · {s.team?.name} · {shortPath(folder)}</Text>
      </Text>
      {total > 0 ? (
        <Text>
          <Text color="yellow">{total} thing{total > 1 ? 's' : ''} need{total > 1 ? '' : 's'} you</Text>   {needs.join(' · ')}
        </Text>
      ) : null}
      <Text> </Text>
      <Text>Resume a chat</Text>
      <Select
        options={[
          ...sessions.map((x) => ({ label: `${dayLabel(x.updatedAt).padEnd(12)} ${clip(x.title, 44)}`, hint: `${x.count} msgs` })),
          { label: '+ Start a new chat' },
        ]}
        onSelect={(i) => (i < sessions.length ? onPick(sessions[i]!.id) : onNew())}
        hotkeys={{ n: onNew }}
        footer="↑↓ choose · enter resume · n new chat"
      />
    </Box>
  );
}
