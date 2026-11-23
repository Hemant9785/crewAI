import { Box, Text, useInput } from 'ink';
import { useState } from 'react';

export interface SelectOption {
  label: string;
  hint?: string;
}

export function Select({
  options, onSelect, hotkeys, footer,
}: {
  options: SelectOption[];
  onSelect: (index: number) => void;
  hotkeys?: Record<string, () => void>;
  footer?: string;
}) {
  const [i, setI] = useState(0);
  useInput((input, key) => {
    if (key.upArrow) setI((i + options.length - 1) % options.length);
    else if (key.downArrow) setI((i + 1) % options.length);
    else if (key.return) onSelect(i);
    else if (input && hotkeys?.[input]) hotkeys[input]!();
  });
  return (
    <Box flexDirection="column">
      {options.map((o, n) => (
        <Text key={`${n}-${o.label}`} color={n === i ? 'cyan' : undefined} bold={n === i}>
          {n === i ? '› ' : '  '}
          {o.label}
          {o.hint ? <Text dimColor>{`   ${o.hint}`}</Text> : null}
        </Text>
      ))}
      {footer ? <Text dimColor>{footer}</Text> : null}
    </Box>
  );
}
