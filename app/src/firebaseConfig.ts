import { readFileSync } from 'node:fs';
import { join } from 'node:path';

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
  const projectId = env.CREWAI_FIREBASE_PROJECT_ID ?? file.projectId;
  const apiKey = env.CREWAI_FIREBASE_API_KEY ?? file.apiKey ?? (emulator ? 'demo-key' : undefined);
  if (!projectId || !apiKey) return null;
  return {
    apiKey,
    projectId,
    authDomain: env.CREWAI_FIREBASE_AUTH_DOMAIN ?? file.authDomain,
    appId: env.CREWAI_FIREBASE_APP_ID ?? file.appId,
    emulator,
  };
}

export const SETUP_HINT =
  'Firebase is not set up yet. Set CREWAI_FIREBASE_PROJECT_ID and CREWAI_FIREBASE_API_KEY ' +
  '(see app/.env.example), or run with --demo to try the interface.';
