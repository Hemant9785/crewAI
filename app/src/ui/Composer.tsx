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
  /** Tab pressed while the box is empty. */
  onTab?: () => void;
  placeholder?: string;
  /** False while another part of the screen has keyboard focus. The draft stays. */
  active?: boolean;
  hint?: string;
  /** Put text back in the box (a message that was not sent). Ignored if the user already started typing. */
  restore?: { text: string; n: number };
}

const STRICT_NO_ARGS = ['tasks', 'threads', 'new', 'help', 'quit'];
const OPTIONAL_ARGS = ['team', 'agent'];

export function Composer({ members, meUid, history, now, onSubmit, onEmptyChange, onTab, placeholder, restore, active = true, hint }: ComposerProps) {
  const [value, setValue] = useState('');
  const [cursor, setCursor] = useState(0);
  const [sel, setSel] = useState(0);
  const [hist, setHist] = useState(-1);
  /** The text for which the user closed the menu with Esc; the menu stays closed until the text changes. */
  const [closedFor, setClosedFor] = useState<string | null>(null);
  const draft = useRef('');

  const slash = /^\/(\S*)$/.exec(value);
  const cmds = slash ? COMMANDS.filter((c) => c.name.startsWith(slash[1]!.toLowerCase())) : [];
  const mention = slash ? null : activeMention(value.slice(0, cursor));
  const people = mention
    ? members.filter((m) => mentionName(m.name).toLowerCase().startsWith(mention.query.toLowerCase()))
    : [];
  const menuClosed = closedFor === value;
  const open: 'cmd' | 'mention' | null = menuClosed ? null : cmds.length ? 'cmd' : people.length ? 'mention' : null;
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

  const restored = useRef(0);
  useEffect(() => {
    if (!restore || restore.n === restored.current) return;
    restored.current = restore.n;
    if (value === '') set(restore.text);
  }, [restore]);

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
    if (key.tab) {
      if (open === 'cmd') set(`/${cmds[shown]!.name} `);
      else if (open === 'mention') insertMention(people[shown]!);
      // With nothing typed, Tab belongs to the screen (move to the task panel). Decided here, from the text
      // itself, so a Tab right after typing can never be mistaken for "the box is empty".
      else if (value === '') onTab?.();
      return;
    }
    if (key.escape) {
      // Esc closes an open menu and keeps the text (a lone "/" is just the menu, so it goes too).
      // With no menu open, it clears the box.
      if (open && value !== '/') {
        setSel(0);
        setClosedFor(value);
      } else set('');
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
  }, { isActive: active });

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
      <Box borderStyle="round" borderColor={active ? 'cyan' : 'gray'} paddingX={1}>
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
      <Text dimColor wrap="truncate-end">  {hint ?? '/ commands   @ teammates   enter send   ctrl+c quit'}</Text>
    </Box>
  );
}
