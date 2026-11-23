import { Box, Text, useInput } from 'ink';
import { useState } from 'react';
import { applyEdit } from './editing.js';

export interface LineInputProps {
  label: string;
  placeholder?: string;
  initial?: string;
  validate?: (v: string) => string | null;
  onSubmit: (v: string) => void;
  onCancel?: () => void;
  busy?: boolean;
}

export function LineInput({ label, placeholder, initial = '', validate, onSubmit, onCancel, busy }: LineInputProps) {
  const [value, setValue] = useState(initial);
  const [cursor, setCursor] = useState(initial.length);
  const [error, setError] = useState('');

  useInput((input, key) => {
    if (busy) return;
    if (key.return) {
      const v = value.trim();
      const problem = validate?.(v) ?? null;
      if (problem) setError(problem);
      else onSubmit(v);
      return;
    }
    if (key.escape) {
      onCancel?.();
      return;
    }
    const r = applyEdit(value, cursor, input, key);
    if (r) {
      setValue(r.value);
      setCursor(r.cursor);
      setError('');
    }
  });

  const before = value.slice(0, cursor);
  const at = value.slice(cursor, cursor + 1) || ' ';
  const after = value.slice(cursor + 1);
  return (
    <Box flexDirection="column">
      <Text>{label}</Text>
      <Box borderStyle="round" borderColor="cyan" paddingX={1}>
        <Text color="cyan">› </Text>
        {value === '' && placeholder ? <Text dimColor>{placeholder}</Text> : (
          <Text>
            {before}
            <Text inverse>{at}</Text>
            {after}
          </Text>
        )}
      </Box>
      {error ? <Text color="red">{error}</Text> : null}
    </Box>
  );
}
