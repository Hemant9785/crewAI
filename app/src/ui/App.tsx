import { Box, Text, useApp, useInput } from 'ink';
import { useEffect, useRef, useState } from 'react';
import type { AgentRunner } from '../agents.js';
import { BackendError, type Backend } from '../backend.js';
import type { LocalStore, Session, SessionSummary } from '../config.js';
import { Engine } from '../engine.js';
import type { AgentName, Team } from '../types.js';
import { Chat } from './Chat.js';
import { Onboarding } from './Onboarding.js';
import { Resume } from './Resume.js';

export interface AppProps {
  backend: Backend;
  store: LocalStore;
  runner: AgentRunner;
  folder: string;
  /** Lets the caller stop the engine after the UI exits. */
  holder?: { engine?: Engine };
  /** Force the resume screen even for a brand-new folder (tests). */
  engineOptions?: { tickMs?: number; heartbeatMs?: number };
}

type Phase = 'loading' | 'offline' | 'onboarding' | 'resume' | 'chat';

const RETRY_MS = 5000;

export function App({ backend, store, runner, folder, holder, engineOptions }: AppProps) {
  const { exit } = useApp();
  const [phase, setPhase] = useState<Phase>('loading');
  const [engine, setEngine] = useState<Engine | null>(null);
  const [session, setSession] = useState<Session | null>(null);
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [agents, setAgents] = useState<AgentName[]>([]);
  const started = useRef(false);
  const [offlineMsg, setOfflineMsg] = useState('');

  async function begin(team: Team) {
    const e = new Engine({ backend, store, runner, folder, ...engineOptions });
    if (holder) holder.engine = e;
    await e.start(team);
    setEngine(e);
    const list = store.listSessions(folder);
    setSessions(list);
    if (list.length) setPhase('resume');
    else startNew(e);
  }

  function startNew(e: Engine | null = engine) {
    if (!e) return;
    // Start from the service's clock: event times are server times.
    const s = store.newSession(folder, e.time());
    e.attach(s, e.time());
    setSession(s);
    setPhase('chat');
  }

  function resume(id: string) {
    const s = store.loadSession(folder, id);
    if (!s || !engine) return startNew();
    engine.attach(s, s.updatedAt);
    setSession(s);
    setPhase('chat');
  }

  useEffect(() => {
    if (started.current) return;
    started.current = true;
    let timer: NodeJS.Timeout | undefined;
    let cancelled = false;
    const open = async (): Promise<void> => {
      const saved = store.settings();
      if (saved.teamId) {
        try {
          const team = await backend.getTeam(saved.teamId);
          if (team) return await begin(team);
        } catch (e) {
          // Offline or the service is down: keep the saved team and try again, never re-onboard.
          if (!(e instanceof BackendError) || e.code === 'unavailable') {
            if (cancelled) return;
            setOfflineMsg(e instanceof BackendError ? e.message : "Can't reach the team service.");
            setPhase('offline');
            timer = setTimeout(() => void open(), RETRY_MS);
            return;
          }
          /* no longer a member (removed or left): fall through to onboarding */
        }
      }
      setPhase('onboarding');
    };
    void (async () => {
      setAgents(await runner.detect());
      await open();
    })();
    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
    };
  }, []);

  useInput(
    (input, key) => {
      if (input === 'q' || key.escape) exit();
    },
    { isActive: phase === 'offline' },
  );

  if (phase === 'loading') return <Text color="yellow">◌ Connecting…</Text>;
  if (phase === 'offline') {
    return (
      <Box flexDirection="column">
        <Text color="yellow">◌ {offlineMsg} Retrying every few seconds…</Text>
        <Text dimColor>Your team ({store.settings().teamName || 'saved team'}) is kept. Press q to quit.</Text>
      </Box>
    );
  }
  if (phase === 'onboarding') {
    return (
      <Onboarding
        backend={backend}
        folder={folder}
        agents={agents}
        onDone={(team, displayName) => {
          store.saveSettings({ displayName, teamId: team.id, teamName: team.name });
          void begin(team);
        }}
      />
    );
  }
  if (!engine) return <Box />;
  if (phase === 'resume') {
    return <Resume engine={engine} sessions={sessions} folder={folder} onPick={resume} onNew={() => startNew()} />;
  }
  return <Chat key={session?.id} engine={engine} folder={folder} onQuit={exit} onNew={() => startNew()} />;
}
