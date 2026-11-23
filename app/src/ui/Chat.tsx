import { Box, Static, Text, useInput } from 'ink';
import { basename } from 'node:path';
import { useCallback, useEffect, useRef, useState } from 'react';
import type { Entry } from '../config.js';
import type { Attention, Engine } from '../engine.js';
import { AttentionCard, EntryView, StatusBar, TaskPanel, sortTasks, type CardOption } from './Cards.js';
import { Composer } from './Composer.js';
import { shortPath, useEngine } from './useEngine.js';

type Item = { id: string; hdr: true } | { id: string; hdr?: false; e: Entry };

const rank = (a: Attention) => (a.type === 'consent' ? 0 : a.compact ? 2 : 1);

export function Chat({ engine, folder, onQuit, onNew }: { engine: Engine; folder: string; onQuit: () => void; onNew: () => void }) {
  const s = useEngine(engine);
  const [items, setItems] = useState<Item[]>([{ id: '__hdr', hdr: true }]);
  const seen = useRef(new Set<string>());
  const [history, setHistory] = useState<string[]>([]);
  const [panel, setPanel] = useState(false);
  const [taskSel, setTaskSel] = useState(0);
  const [composerEmpty, setComposerEmpty] = useState(true);
  const [sel, setSel] = useState(0);
  const [restore, setRestore] = useState<{ text: string; n: number } | undefined>();

  useEffect(() => {
    const fresh = s.entries.filter((e) => !seen.current.has(e.id));
    if (fresh.length === 0) return;
    fresh.forEach((e) => seen.current.add(e.id));
    setItems((prev) => [...prev, ...fresh.map((e) => ({ id: e.id, e }))]);
  }, [s.entries]);

  const cards = [...s.attention].sort((a, b) => rank(a) - rank(b));
  const optionsFor = (a: Attention): CardOption[] => {
    const id = a.request.id;
    if (a.type === 'consent') {
      return [
        { label: 'Allow once', run: () => void engine.decideConsent(id, 'once') },
        { label: `Always allow ${a.request.askerName}`, run: () => void engine.decideConsent(id, 'always') },
        { label: 'Decline', run: () => void engine.decideConsent(id, 'decline') },
      ];
    }
    const o: CardOption[] = [];
    if (!a.targetOnline && !a.compact) o.push({ label: `Wait for ${a.request.targetName}`, run: () => engine.waitFor(id) });
    for (const m of a.alternatives) o.push({ label: `Ask ${m.name}`, run: () => void engine.moveRequest(id, m.uid) });
    o.push({ label: 'Cancel', run: () => void engine.cancelRequest(id) });
    return o;
  };
  const focus = cards[0];
  const focusOptions = focus ? optionsFor(focus) : [];
  const focusKey = focus ? `${focus.request.id}:${focus.type}:${focusOptions.length}` : '';
  useEffect(() => setSel(0), [focusKey]);

  useInput(
    (_input, key) => {
      const n = focusOptions.length;
      if (n === 0) return;
      if (key.leftArrow) setSel((sel + n - 1) % n);
      else if (key.rightArrow) setSel((sel + 1) % n);
      else if (key.return) focusOptions[Math.min(sel, n - 1)]!.run();
    },
    { isActive: !panel && composerEmpty && focusOptions.length > 0 },
  );

  const rows = sortTasks(s.tasks).slice(0, 12);
  useInput(
    (input, key) => {
      if (key.escape || input === 'q') return setPanel(false);
      if (key.upArrow) return setTaskSel((taskSel + rows.length - 1) % Math.max(1, rows.length));
      if (key.downArrow) return setTaskSel((taskSel + 1) % Math.max(1, rows.length));
      const t = rows[taskSel];
      if (!t) return;
      if (input === 's') void engine.setTaskStatus(t.id, 'in_progress');
      else if (input === 'd') void engine.setTaskStatus(t.id, 'done');
      else if (input === 'h') void engine.setTaskStatus(t.id, 'needs_help');
      else if (input === 'x') void engine.setTaskStatus(t.id, 'cancelled');
    },
    { isActive: panel },
  );

  const submit = useCallback(
    async (text: string) => {
      setHistory((h) => [...h, text]);
      const out = await engine.submit(text);
      if (out.keepDraft) setRestore((r) => ({ text, n: (r?.n ?? 0) + 1 }));
      if (out.action === 'quit') onQuit();
      else if (out.action === 'new') onNew();
      else if (out.action === 'tasks') {
        setTaskSel(0);
        setPanel(true);
      }
    },
    [engine, onQuit, onNew],
  );

  const folderName = basename(folder);
  return (
    <Box flexDirection="column">
      <Static items={items}>
        {(it) =>
          'hdr' in it && it.hdr ? (
            <Box key={it.id} flexDirection="column" marginBottom={1}>
              <Text>
                <Text bold>crewai</Text>
                <Text dimColor> · {s.team?.name} · {shortPath(folder)}</Text>
              </Text>
              <Text dimColor>Type / for commands, @ to pick a teammate. Plain text asks your own agent about this folder.</Text>
            </Box>
          ) : (
            <EntryView key={it.id} e={(it as { e: Entry }).e} />
          )
        }
      </Static>

      <StatusBar team={s.team?.name ?? ''} folder={shortPath(folder)} agent={s.agent} me={s.me} members={s.members} now={s.now} error={s.error} />
      {s.busy ? <Text color="yellow">◌ {s.busy}</Text> : null}
      {cards.map((a, i) => (
        <AttentionCard
          key={`${a.type}:${a.request.id}`}
          item={a}
          options={optionsFor(a)}
          selected={Math.min(sel, Math.max(0, focusOptions.length - 1))}
          focused={i === 0 && composerEmpty && !panel}
          agent={s.agent}
          folder={folderName}
        />
      ))}
      {panel ? (
        <TaskPanel tasks={s.tasks} selected={taskSel} meUid={s.me?.uid ?? ''} />
      ) : (
        <Composer
          members={s.members}
          meUid={s.me?.uid ?? ''}
          history={history}
          now={s.now}
          onSubmit={(t) => void submit(t)}
          onEmptyChange={setComposerEmpty}
          restore={restore}
          placeholder="Ask your agent, or type / for commands"
        />
      )}
    </Box>
  );
}
