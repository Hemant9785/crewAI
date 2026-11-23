import { randomBytes } from 'node:crypto';
import { collection, doc, getDoc, setDoc, updateDoc, type Firestore } from 'firebase/firestore';
import { afterAll, describe, expect, it } from 'vitest';
import type { Backend, BackendError } from './backend.js';
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

// Rules that only Firestore enforces (the in-memory service has no raw document access).
describe('firestore rules: invites, names and server times', () => {
  const connect = async () => {
    const b = await FirebaseBackend.connect(
      {
        apiKey: 'demo-key',
        projectId: 'demo-crewai',
        emulator: {
          firestore: process.env.CREWAI_FIRESTORE_EMULATOR ?? '127.0.0.1:8080',
          auth: process.env.CREWAI_AUTH_EMULATOR ?? '127.0.0.1:9099',
        },
      },
      { email: `r-${randomBytes(6).toString('hex')}@crewai.invalid`, password: randomBytes(12).toString('hex') },
    );
    opened.push(b);
    return b;
  };
  const db = (b: Backend) => (b as unknown as { db: Firestore }).db;
  const denied = (p: Promise<unknown>) => p.then(() => 'ok', (e: { code?: string }) => (String(e.code).includes('permission-denied') ? 'denied' : String(e.code)));

  it('blocks reading an invite without a join ticket for that code', async () => {
    const boss = await connect();
    const team = await boss.createTeam('Acme', 'Boss');
    const inv = await boss.createInvite(team.id);
    const outsider = await connect();
    expect(await denied(getDoc(doc(db(outsider), 'invites', inv.code)))).toBe('denied');
  });

  it('allows one join try every two seconds per account', async () => {
    const boss = await connect();
    await boss.createTeam('Acme', 'Boss');
    const guesser = await connect();
    expect(await guesser.joinTeam('000001', 'Gus').catch((e: BackendError) => e.code)).toBe('not-found');
    expect(await guesser.joinTeam('000002', 'Gus').catch((e: BackendError) => e.message)).toContain('Too many tries');
  });

  it('blocks burning an invite without joining', async () => {
    const boss = await connect();
    const team = await boss.createTeam('Acme', 'Boss');
    const inv = await boss.createInvite(team.id);
    const outsider = await connect();
    expect(await denied(updateDoc(doc(db(outsider), 'invites', inv.code), { used: true }))).toBe('denied');
    await (await connect()).joinTeam(inv.code, 'Ann');
  });

  it('blocks reserving a name without joining', async () => {
    const boss = await connect();
    const team = await boss.createTeam('Acme', 'Boss');
    const outsider = await connect();
    expect(await denied(setDoc(doc(db(outsider), 'teams', team.id, 'names', 'bob'), { uid: outsider.uid, name: 'bob' }))).toBe('denied');
  });

  it('rejects client clocks for event times', async () => {
    const boss = await connect();
    const team = await boss.createTeam('Acme', 'Boss');
    const ref = doc(collection(db(boss), 'teams', team.id, 'broadcasts'));
    expect(await denied(setDoc(ref, { fromUid: boss.uid, fromName: 'Boss', text: 'pinned', createdAt: Date.now() + 86_400_000 }))).toBe('denied');
  });

  it('reissues a code once it has been used', async () => {
    const boss = await connect();
    const team = await boss.createTeam('Acme', 'Boss');
    const inv = await boss.createInvite(team.id);
    await (await connect()).joinTeam(inv.code, 'Ann');
    await setDoc(doc(db(boss), 'invites', inv.code), { teamId: team.id, createdBy: boss.uid, expiresAt: boss.serverNow() + 60_000, used: false });
    await (await connect()).joinTeam(inv.code, 'Bob');
  });
});
