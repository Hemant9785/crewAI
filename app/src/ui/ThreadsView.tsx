import { Box, Text, useInput } from 'ink';
import { useEffect, useRef, useState } from 'react';
import type { Engine } from '../engine.js';
import { clip, hhmm } from '../format.js';
import { openUrl } from '../openUrl.js';
import { threadLabel } from '../threads.js';
import type { Thread, ThreadMessage, Verdict } from '../types.js';
import { applyEdit } from './editing.js';
import { Panel } from './Panels.js';
import { wrap } from './textlayout.js';
import { shortPath } from './useEngine.js';
import { useHeight, type Size } from './useTerminal.js';

type Pane = 'list' | 'detail';
const VERDICTS: { v: Verdict; label: string }[] = [
  { v: 'comment', label: 'Comment' },
  { v: 'approved', label: 'Approve' },
  { v: 'changes', label: 'Request changes' },
];
const BADGE: Record<string, { text: string; color: string }> = {
  approved: { text: 'approved', color: 'green' },
  changes: { text: 'asked for changes', color: 'red' },
};

function bubbleLines(m: ThreadMessage, width: number, meUid: string): { t: string; color?: string; bold?: boolean; dim?: boolean }[][] {
  const out: { t: string; color?: string; bold?: boolean; dim?: boolean }[][] = [];
  const badge = BADGE[m.verdict];
  out.push([
    { t: hhmm(m.createdAt) + '  ', dim: true },
    { t: m.fromUid === meUid ? 'you' : m.fromName, bold: true, color: m.fromUid === meUid ? 'cyan' : 'magenta' },
    ...(badge ? [{ t: '  ' + badge.text, color: badge.color, bold: true }] : []),
  ]);
  for (const l of wrap(m.text, Math.max(8, width - 7))) out.push([{ t: '       ' }, { t: l }]);
  out.push([{ t: '' }]);
  return out;
}

export function ThreadsView({
  engine, threads, meUid, size, narrow, team, folder, taskId, initialId, onClose,
}: {
  engine: Engine; threads: Thread[]; meUid: string; size: Size; narrow: boolean; team: string; folder: string;
  /** Show only conversations about this task. */
  taskId?: string;
  initialId?: string;
  onClose: () => void;
}) {
  const shown = taskId ? threads.filter((t) => t.taskId === taskId) : threads;
  const [selId, setSelId] = useState(initialId ?? '');
  const [pane, setPane] = useState<Pane>(initialId ? 'detail' : 'list');
  const [messages, setMessages] = useState<ThreadMessage[]>([]);
  const [text, setText] = useState('');
  const [cursor, setCursor] = useState(0);
  const [verdictIx, setVerdictIx] = useState(0);
  const [scroll, setScroll] = useState(0);
  const [notice, setNotice] = useState('');
  const sel = shown.find((t) => t.id === selId) ?? shown[0];
  const isReviewer = !!sel && sel.kind === 'review' && sel.createdByUid !== meUid;
  const isAuthor = !!sel && sel.createdByUid === meUid;
  const unreadIds = new Set(engine.unreadThreads().map((t) => t.id));

  // Live messages for the selected conversation.
  useEffect(() => {
    setMessages([]);
    setScroll(0);
    if (!sel) return;
    return engine.subscribeThreadMessages(sel.id, setMessages);
  }, [sel?.id]);

  // Reading a conversation clears its "new" marker, including replies that arrive while it is open.
  useEffect(() => {
    if (sel && pane === 'detail') engine.markThreadSeen(sel.id);
  }, [sel?.id, sel?.updatedAt, pane]);

  const send = async () => {
    if (!sel || !text.trim()) return;
    const err = await engine.reply(sel.id, text.trim(), isReviewer ? VERDICTS[verdictIx]!.v : undefined);
    if (err) setNotice(err);
    else {
      setText('');
      setCursor(0);
      setNotice('');
      setScroll(0);
    }
  };

  useInput((input, key) => {
    if (pane === 'list') {
      if (key.escape || input === 'q') return onClose();
      const at = Math.max(0, shown.findIndex((t) => t.id === sel?.id));
      if (key.upArrow && shown.length) setSelId(shown[(at + shown.length - 1) % shown.length]!.id);
      else if (key.downArrow && shown.length) setSelId(shown[(at + 1) % shown.length]!.id);
      else if ((key.return || key.rightArrow) && sel) setPane('detail');
      return;
    }
    // detail: the keyboard is a text box
    if (key.escape) {
      if (text) {
        setText('');
        setCursor(0);
      } else setPane('list');
      return;
    }
    if (key.pageUp) return setScroll((s) => s + 5);
    if (key.pageDown) return setScroll((s) => Math.max(0, s - 5));
    if (key.tab) {
      if (isReviewer) setVerdictIx((verdictIx + 1) % VERDICTS.length);
      return;
    }
    if (key.return) return void send();
    if (key.ctrl && input === 'o') {
      if (sel?.prUrl && !openUrl(sel.prUrl)) setNotice("Couldn't open the link. Copy it from above.");
      return;
    }
    if (key.ctrl && input === 'x') {
      if (sel && isAuthor) void engine.setThreadStatus(sel.id, sel.status === 'open' ? 'closed' : 'open').then((e) => e && setNotice(e));
      return;
    }
    const r = applyEdit(text, cursor, input, key);
    if (r) {
      setText(r.value);
      setCursor(r.cursor);
    }
  });

  const listW = narrow ? size.columns : 40;
  const detailW = narrow ? size.columns : size.columns - listW - 1;
  const showList = !narrow || pane === 'list';
  const showDetail = !narrow || pane === 'detail';
  const [bodyRef, bodyH] = useHeight(8);

  const lines = sel ? messages.flatMap((m) => bubbleLines(m, detailW - 4, meUid)) : [];
  const maxScroll = Math.max(0, lines.length - bodyH);
  const up = Math.min(scroll, maxScroll);
  const end = lines.length - up;
  const visible = lines.slice(Math.max(0, end - bodyH), end);

  const before = text.slice(0, cursor);
  const at = text.slice(cursor, cursor + 1) || ' ';
  const after = text.slice(cursor + 1);
  const closed = sel?.status === 'closed';

  return (
    <Box flexDirection="column" height={size.rows - 1} width={size.columns}>
      <Box justifyContent="space-between">
        <Text wrap="truncate-end">
          <Text bold color="cyan">◆ crewai</Text>
          <Text dimColor>  {team} · {shortPath(folder)} · {taskId ? 'Follow-ups' : 'Conversations'}</Text>
        </Text>
        <Box flexShrink={0} marginLeft={2}>
          <Text dimColor>esc back</Text>
        </Box>
      </Box>
      <Box flexGrow={1} flexBasis={0}>
        {showList ? (
          <Panel title={`${shown.length} CONVERSATION${shown.length === 1 ? '' : 'S'}`} color={pane === 'list' ? 'cyan' : 'gray'} width={listW} flexShrink={0}>
            {shown.length === 0 ? (
              <Text dimColor wrap="wrap">
                Nothing yet. Ask about a task with /followup T-4 your question, or share a pull request with /review link @name comments.
              </Text>
            ) : (
              shown.slice(0, Math.max(3, bodyH + 2)).map((t) => {
                const on = t.id === sel?.id;
                return (
                  <Text key={t.id} wrap="truncate-end" inverse={on && pane === 'list'} bold={on && pane !== 'list'} dimColor={t.status === 'closed'}>
                    {unreadIds.has(t.id) ? <Text color="cyan">● </Text> : '  '}
                    {t.kind === 'review' ? '⎇ ' : '✉ '}
                    {threadLabel(t)} · {t.lastFromUid === meUid ? 'you' : t.lastFromName}: {clip(t.lastText, 80)}
                  </Text>
                );
              })
            )}
          </Panel>
        ) : null}
        {showDetail ? (
          <Panel
            title={sel ? threadLabel(sel) : 'CONVERSATION'}
            right={sel ? <Text color={closed ? 'gray' : 'green'}>{closed ? 'closed' : 'open'}</Text> : null}
            color={pane === 'detail' ? 'cyan' : 'blue'}
            flexGrow={1}
            flexBasis={0}
            marginLeft={narrow ? 0 : 1}
          >
            {sel ? (
              <>
                <Text dimColor wrap="truncate-end">
                  {sel.kind === 'review' ? 'Review' : 'Follow-up'} · with {sel.participantNames.filter((_, i) => sel.participants[i] !== meUid).join(', ')}
                  {sel.taskNum ? ` · task T-${sel.taskNum}` : ''}
                </Text>
                {sel.prUrl ? (
                  <Text wrap="truncate-end">
                    <Text color="blue">⎇ {sel.prUrl}</Text>
                    <Text dimColor>  ctrl+o opens it</Text>
                  </Text>
                ) : null}
                <Box ref={bodyRef} flexDirection="column" flexGrow={1} flexBasis={0} overflow="hidden" justifyContent="flex-end" marginTop={1}>
                  {visible.map((l, i) => (
                    <Text key={i} wrap="truncate-end">
                      {l.length === 1 && l[0]!.t === ''
                        ? ' '
                        : l.map((s, j) => (
                            <Text key={j} color={s.color} bold={s.bold} dimColor={s.dim}>{s.t}</Text>
                          ))}
                    </Text>
                  ))}
                </Box>
                {up > 0 ? <Text color="yellow">↓ older messages shown · pgdn for the latest</Text> : null}
                {isReviewer && !closed ? (
                  <Text>
                    {VERDICTS.map((x, i) => (
                      <Text key={x.v}>
                        {i === verdictIx ? <Text inverse color="cyan"> {x.label} </Text> : <Text dimColor> {x.label} </Text>}
                      </Text>
                    ))}
                    <Text dimColor>  tab to choose</Text>
                  </Text>
                ) : null}
                <Box borderStyle="round" borderColor={closed ? 'gray' : pane === 'detail' ? 'cyan' : 'gray'} paddingX={1} flexShrink={0}>
                  {closed ? (
                    <Text dimColor>This conversation is closed.{isAuthor ? ' ctrl+x reopens it.' : ''}</Text>
                  ) : (
                    <Text wrap="truncate-end">
                      <Text color="cyan">› </Text>
                      {text === '' && pane !== 'detail' ? <Text dimColor>enter to reply</Text> : (
                        <>
                          {before}
                          <Text inverse={pane === 'detail'}>{at}</Text>
                          {after}
                        </>
                      )}
                    </Text>
                  )}
                </Box>
                {notice ? <Text color="red" wrap="truncate-end">{notice}</Text> : null}
              </>
            ) : (
              <Text dimColor>Pick a conversation on the left.</Text>
            )}
          </Panel>
        ) : null}
      </Box>
      <Text dimColor wrap="truncate-end">
        {pane === 'list'
          ? '↑↓ choose · enter open · esc back'
          : `enter send · esc ${text ? 'clear' : 'list'} · pgup/pgdn scroll${isReviewer ? ' · tab verdict' : ''}${sel?.prUrl ? ' · ctrl+o open link' : ''}${isAuthor ? ' · ctrl+x close' : ''}`}
      </Text>
    </Box>
  );
}
