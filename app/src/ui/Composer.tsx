import { Box, Text, useInput } from 'ink';
import { useEffect, useRef, useState } from 'react';
import { COMMANDS, activeMention, mentionName } from '../commands.js';
import { isOnline, type Member } from '../types.js';
import { applyEdit } from './editing.js';

export interface ComposerProps {
  members: Member[];
  meUid: string;
  history: string[];
  now: number;
  onSubmit: (text: string) => void;
  onEmptyChange?: (empty: boolean) => void;
  placeholder?: string;
}

const STRICT_NO_ARGS = ['tasks', 'new', 'help', 'quit'];
const OPTIONAL_ARGS = ['team', 'agent'];

export function Composer({ members, meUid, history, now, onSubmit, onEmptyChange, placeholder }: ComposerProps) {
  const [value, setValue] = useState('');
  const [cursor, setCursor] = useState(0);
  const [sel, setSel] = useState(0);
  const [hist, setHist] = useState(-1);
  const draft = useRef('');

  const slash = /^\/(\S*)$/.exec(value);
  const cmds = slash ? COMMANDS.filter((c) => c.name.startsWith(slash[1]!.toLowerCase())) : [];
  const mention = slash ? null : activeMention(value.slice(0, cursor));
  const people = mention
    ? members.filter((m) => mentionName(m.name).toLowerCase().startsWith(mention.query.toLowerCase()))
    : [];
  const open: 'cmd' | 'mention' | null = cmds.length ? 'cmd' : people.length ? 'mention' : null;
  const count = open === 'cmd' ? cmds.length : people.length;
  const shown = Math.min(sel, Math.max(0, count - 1));

  useEffect(() => {
    onEmptyChange?.(value === '');
  }, [value, onEmptyChange]);

  const set = (v: string, c = v.length) => {
    setValue(v);
    setCursor(c);
    setSel(0);
  };

  useInput((input, key) => {
    if (key.return) {
      if (open === 'cmd') {
        const c = cmds[shown]!;
        const exact = value.trim().toLowerCase() === `/${c.name}`;
        if (STRICT_NO_ARGS.includes(c.name) || (exact && OPTIONAL_ARGS.includes(c.name))) {
          onSubmit(`/${c.name}`);
          set('');
        } else set(`/${c.name} `);
        return;
      }
      if (open === 'mention') {
        insertMention(people[shown]!);
        return;
      }
      const text = value.trim();
      if (!text) return;
      onSubmit(text);
      setHist(-1);
      set('');
      return;
    }
    if (key.tab && open) {
      if (open === 'cmd') set(`/${cmds[shown]!.name} `);
      else insertMention(people[shown]!);
      return;
    }
    if (key.escape) {
      if (open) setSel(0);
      set('');
      return;
    }
    if (key.upArrow || key.downArrow) {
      if (open) {
        setSel((shown + (key.downArrow ? 1 : count - 1)) % count);
        return;
      }
      if (history.length === 0) return;
      if (key.upArrow) {
        if (hist === -1) draft.current = value;
        const next = Math.min(hist + 1, history.length - 1);
        setHist(next);
        set(history[history.length - 1 - next]!);
      } else if (hist >= 0) {
        const next = hist - 1;
        setHist(next);
        set(next === -1 ? draft.current : history[history.length - 1 - next]!);
      }
      return;
    }
    const r = applyEdit(value, cursor, input, key);
    if (r) {
      setValue(r.value);
      setCursor(r.cursor);
      setSel(0);
      setHist(-1);
    }
  });

  function insertMention(m: Member) {
    const start = mention!.start;
    const head = `${value.slice(0, start)}@${mentionName(m.name)} `;
    set(head + value.slice(cursor), head.length);
  }

  const before = value.slice(0, cursor);
  const at = value.slice(cursor, cursor + 1) || ' ';
  const after = value.slice(cursor + 1);

  return (
    <Box flexDirection="column">
      {open === 'cmd' &&
        cmds.map((c, i) => (
          <Text key={c.name} color={i === shown ? 'cyan' : undefined} bold={i === shown}>
            {i === shown ? '› ' : '  '}
            {`/${c.name}`.padEnd(12)}
            <Text dimColor>{c.desc}</Text>
          </Text>
        ))}
      {open === 'mention' &&
        people.map((m, i) => (
          <Text key={m.uid} color={i === shown ? 'cyan' : undefined} bold={i === shown}>
            {i === shown ? '› ' : '  '}
            <Text color={isOnline(m, now) ? 'green' : 'gray'}>{isOnline(m, now) ? '●' : '○'}</Text>
            {` ${mentionName(m.name)}${m.uid === meUid ? ' (you)' : ''}`}
            <Text dimColor>{isOnline(m, now) ? `  ${m.agent ?? 'no agent'}` : '  offline'}</Text>
          </Text>
        ))}
      <Box borderStyle="round" borderColor="cyan" paddingX={1}>
        <Text color="cyan">› </Text>
        {value === '' && placeholder ? (
          <Text>
            <Text inverse> </Text>
            <Text dimColor>{placeholder}</Text>
          </Text>
        ) : (
          <Text>
            {before}
            <Text inverse>{at}</Text>
            {after}
          </Text>
        )}
      </Box>
      <Text dimColor>  / commands   @ teammates   enter send   ctrl+c quit</Text>
    </Box>
  );
}
