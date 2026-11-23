import { Box, Text } from 'ink';
import type { Attention } from '../engine.js';
import { clip } from '../format.js';
import type { AgentName } from '../types.js';

export interface CardOption {
  label: string;
  run: () => void;
}

function OptionRow({ options, selected, focused }: { options: CardOption[]; selected: number; focused: boolean }) {
  return (
    <Text>
      {options.map((o, i) => (
        <Text key={o.label}>
          {i > 0 ? '  ' : ''}
          {focused && i === selected ? <Text inverse color="cyan"> {o.label} </Text> : <Text> [{o.label}] </Text>}
        </Text>
      ))}
    </Text>
  );
}

/**
 * A question that needs the user. `bare` leaves off the border (for use inside a panel);
 * `narrow` uses the short copy that fits a side column.
 */
export function AttentionCard({
  item, options, selected, focused, agent, folder, bare = false, narrow = false,
}: {
  item: Attention; options: CardOption[]; selected: number; focused: boolean; agent: AgentName | null; folder: string;
  bare?: boolean; narrow?: boolean;
}) {
  const r = item.request;
  const frame = bare ? {} : ({ borderStyle: 'round', borderColor: 'yellow', paddingX: 1, marginTop: 1 } as const);
  const hint = focused ? (
    <Text dimColor>{narrow ? '← → pick · enter' : '← → pick an option, then enter (when the message box is empty)'}</Text>
  ) : null;

  if (item.type === 'consent') {
    return (
      <Box flexDirection="column" {...frame}>
        <Text bold color="yellow">{r.askerName} asks about your project</Text>
        <Text>"{clip(r.question, narrow ? 120 : 300)}"</Text>
        <Text dimColor>
          {narrow
            ? `Your ${agent ?? 'agent'} answers from ${folder}. Limited protection: it can read this folder. Secrets are filtered, not perfectly.`
            : `Your ${agent ?? 'agent'} will answer from ${folder}. Limited protection: it can read any file in this folder. Only the answer is shared, and values that look like secrets are removed first, but that check is not perfect.`}
        </Text>
        <OptionRow options={options} selected={selected} focused={focused} />
        {hint}
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
    <Box flexDirection="column" {...frame}>
      <Text bold color="yellow">
        {item.targetOnline ? `Waiting for ${r.targetName} to allow…` : `${r.targetName} is offline`}
      </Text>
      <Text dimColor>"{clip(r.question, narrow ? 60 : 120)}" is saved{item.targetOnline ? '.' : ' and will reach them when they return.'}</Text>
      <OptionRow options={options} selected={selected} focused={focused} />
      {hint}
    </Box>
  );
}
