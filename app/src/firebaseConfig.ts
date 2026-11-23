import { readFileSync } from 'node:fs';
import { join } from 'node:path';

/** JSON of the public Firebase web config, replaced at build time for the standalone binaries. */
declare const __CREWAI_FIREBASE__: string | undefined;

export interface FirebaseSettings {
  apiKey: string;
  authDomain?: string;
  projectId: string;
  appId?: string;
  emulator?: { firestore: string; auth: string };
}

/**
 * Firebase web config is a set of public identifiers, not secrets. Order:
 * environment variables, then <crewai home>/firebase.json. Returns null when unset.
 */
export function loadFirebaseSettings(home: string, env: NodeJS.ProcessEnv = process.env): FirebaseSettings | null {
  const fsEmu = env.CREWAI_FIRESTORE_EMULATOR;
  const authEmu = env.CREWAI_AUTH_EMULATOR;
  const emulator = fsEmu && authEmu ? { firestore: fsEmu, auth: authEmu } : undefined;

  let file: Partial<FirebaseSettings> = {};
  try {
    file = JSON.parse(readFileSync(join(home, 'firebase.json'), 'utf8')) as Partial<FirebaseSettings>;
  } catch {
    /* optional */
  }
  let embedded: Partial<FirebaseSettings> = {};
  try {
    if (typeof __CREWAI_FIREBASE__ === 'string' && __CREWAI_FIREBASE__) embedded = JSON.parse(__CREWAI_FIREBASE__) as Partial<FirebaseSettings>;
  } catch {
    /* ignore a malformed embedded value */
  }
  const projectId = env.CREWAI_FIREBASE_PROJECT_ID ?? file.projectId ?? embedded.projectId;
  const apiKey = env.CREWAI_FIREBASE_API_KEY ?? file.apiKey ?? embedded.apiKey ?? (emulator ? 'demo-key' : undefined);
  if (!projectId || !apiKey) return null;
  return {
    apiKey,
    projectId,
    authDomain: env.CREWAI_FIREBASE_AUTH_DOMAIN ?? file.authDomain ?? embedded.authDomain,
    appId: env.CREWAI_FIREBASE_APP_ID ?? file.appId ?? embedded.appId,
    emulator,
  };
}

export const SETUP_HINT =
  'Firebase is not set up yet. Set CREWAI_FIREBASE_PROJECT_ID and CREWAI_FIREBASE_API_KEY ' +
  '(see app/.env.example), or run with --demo to try the interface.';
