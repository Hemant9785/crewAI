import { Box, Text } from 'ink';
import { useState } from 'react';
import type { Backend } from '../backend.js';
import { BackendError } from '../backend.js';
import { validName, type AgentName, type Team } from '../types.js';
import { LineInput } from './LineInput.js';
import { Select } from './Select.js';
import { shortPath } from './useEngine.js';

type Step = 'name' | 'mode' | 'code' | 'teamName' | 'working';

export function Onboarding({
  backend, folder, agents, onDone,
}: {
  backend: Backend;
  folder: string;
  agents: AgentName[];
  onDone: (team: Team, displayName: string) => void;
}) {
  const [step, setStep] = useState<Step>('name');
  const [name, setName] = useState('');
  const [mode, setMode] = useState<'join' | 'create'>('join');
  const [error, setError] = useState('');

  async function finish(run: () => Promise<Team>) {
    setStep('working');
    setError('');
    try {
      const team = await run();
      onDone(team, name);
    } catch (e) {
      setError(e instanceof BackendError ? e.message : 'Something went wrong. Try again.');
      setStep(mode === 'join' ? 'code' : 'teamName');
    }
  }

  const mark = (done: boolean, active: boolean) => (done ? <Text color="green">✓</Text> : active ? <Text color="cyan">●</Text> : <Text dimColor>○</Text>);
  const teamDone = step === 'working';
  return (
    <Box flexDirection="column" marginTop={1}>
      <Text>
        <Text bold>crewai</Text>
        <Text dimColor> first run</Text>
      </Text>
      <Text>Welcome. A few quick steps, about a minute.</Text>
      <Box flexDirection="column" marginY={1}>
        <Text>{mark(true, false)} 1  Sign in   <Text dimColor>this device</Text></Text>
        <Text>{mark(teamDone, step !== 'working')} 2  Team      <Text dimColor>{name ? `as ${name}` : 'your name, then join or create'}</Text></Text>
        <Text>{mark(false, false)} 3  Project   <Text dimColor>this folder: {shortPath(folder)}</Text></Text>
        <Text>{mark(false, false)} 4  Agent     <Text dimColor>{agents.length ? `found: ${agents.join(', ')}` : 'none found yet (install Codex or Claude Code)'}</Text></Text>
      </Box>

      {step === 'name' && (
        <LineInput
          label="What should teammates call you?"
          placeholder="for example Hemant"
          validate={(v) => (validName(v) ? null : 'Use letters, numbers, dots, dashes or underscores, no spaces.')}
          onSubmit={(v) => {
            setName(v);
            setStep('mode');
          }}
        />
      )}
      {step === 'mode' && (
        <Select
          options={[
            { label: 'Join a team', hint: 'I have a 6-digit code' },
            { label: 'Create a team', hint: "I'm the manager" },
          ]}
          onSelect={(i) => {
            setMode(i === 0 ? 'join' : 'create');
            setStep(i === 0 ? 'code' : 'teamName');
          }}
          footer="↑↓ choose · enter confirm"
        />
      )}
      {step === 'code' && (
        <LineInput
          label="Enter the 6-digit code your manager gave you"
          placeholder="123456"
          validate={(v) => (/^\d{6}$/.test(v.replace(/\s/g, '')) ? null : 'The code is six digits.')}
          onSubmit={(v) => void finish(() => backend.joinTeam(v.replace(/\s/g, ''), name))}
          onCancel={() => setStep('mode')}
        />
      )}
      {step === 'teamName' && (
        <LineInput
          label="Name your team"
          placeholder="for example Acme"
          validate={(v) => (v.length >= 1 && v.length <= 40 ? null : 'Use 1 to 40 characters.')}
          onSubmit={(v) => void finish(() => backend.createTeam(v, name))}
          onCancel={() => setStep('mode')}
        />
      )}
      {step === 'working' && <Text color="yellow">◌ Setting things up…</Text>}
      {error ? <Text color="red">{error}</Text> : null}
    </Box>
  );
}
