import { randomBytes } from 'node:crypto';
import { homedir } from 'node:os';
import { join } from 'node:path';
import { afterAll } from 'vitest';
import type { Backend } from './backend.js';
import { FirebaseBackend } from './firebaseBackend.js';
import { loadFirebaseSettings } from './firebaseConfig.js';
import { runContract } from './testing/contract.js';

// Runs the shared backend contract against the REAL Firebase project from ~/.crewai.
// It creates throwaway users and teams there. Run on purpose: npm run test:live
const home = process.env.CREWAI_HOME ?? join(homedir(), '.crewai');
try {
  process.loadEnvFile(join(home, '.env'));
} catch {
  /* optional */
}
const cfg = loadFirebaseSettings(home);
if (!cfg || cfg.emulator) throw new Error('Set the real Firebase config in ~/.crewai/.env (and no emulator) to run live tests.');

const opened: Backend[] = [];
afterAll(async () => {
  await Promise.all(opened.map((b) => b.close()));
});

runContract('firebase live', () => ({
  user: async () => {
    const b = await FirebaseBackend.connect(cfg, {
      email: `livetest-${randomBytes(6).toString('hex')}@crewai.invalid`,
      password: randomBytes(12).toString('hex'),
    });
    opened.push(b);
    return b;
  },
}));
