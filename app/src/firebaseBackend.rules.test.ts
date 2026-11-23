import { randomBytes } from 'node:crypto';
import { afterAll } from 'vitest';
import type { Backend } from './backend.js';
import { FirebaseBackend } from './firebaseBackend.js';
import { runContract } from './testing/contract.js';

// Runs the shared backend contract against the real Firestore rules in the emulator.
// Start with: npm run emulators
const opened: Backend[] = [];
afterAll(async () => {
  await Promise.all(opened.map((b) => b.close()));
});

runContract('firebase emulator', () => ({
  user: async () => {
    const b = await FirebaseBackend.connect(
      {
        apiKey: 'demo-key',
        projectId: 'demo-crewai',
        emulator: {
          firestore: process.env.CREWAI_FIRESTORE_EMULATOR ?? '127.0.0.1:8080',
          auth: process.env.CREWAI_AUTH_EMULATOR ?? '127.0.0.1:9099',
        },
      },
      { email: `t-${randomBytes(6).toString('hex')}@crewai.invalid`, password: randomBytes(12).toString('hex') },
    );
    opened.push(b);
    return b;
  },
}));
