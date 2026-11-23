import { useSyncExternalStore } from 'react';
import type { Engine, EngineState } from '../engine.js';

export function useEngine(engine: Engine): EngineState {
  return useSyncExternalStore(
    (cb) => engine.subscribe(cb),
    () => engine.state,
  );
}

export function shortPath(p: string): string {
  const home = process.env.USERPROFILE ?? process.env.HOME ?? '';
  return home && p.toLowerCase().startsWith(home.toLowerCase()) ? `~${p.slice(home.length).replace(/\\/g, '/')}` : p;
}
