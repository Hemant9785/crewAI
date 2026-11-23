import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { loadFirebaseSettings } from './firebaseConfig.js';

const dirs: string[] = [];
const home = () => {
  const d = mkdtempSync(join(tmpdir(), 'crewai-fbcfg-'));
  dirs.push(d);
  return d;
};
afterEach(() => dirs.splice(0).forEach((d) => rmSync(d, { recursive: true, force: true })));

describe('loadFirebaseSettings', () => {
  it('returns null when nothing is configured', () => {
    expect(loadFirebaseSettings(home(), {})).toBeNull();
  });
  it('reads the environment, which wins over the saved file', () => {
    const h = home();
    writeFileSync(join(h, 'firebase.json'), JSON.stringify({ projectId: 'from-file', apiKey: 'file-key' }));
    expect(loadFirebaseSettings(h, { CREWAI_FIREBASE_PROJECT_ID: 'from-env', CREWAI_FIREBASE_API_KEY: 'env-key' })).toMatchObject({
      projectId: 'from-env', apiKey: 'env-key',
    });
    expect(loadFirebaseSettings(h, {})).toMatchObject({ projectId: 'from-file', apiKey: 'file-key' });
  });
  it('uses a placeholder key and the emulators when both emulator hosts are set', () => {
    const s = loadFirebaseSettings(home(), {
      CREWAI_FIREBASE_PROJECT_ID: 'demo-x',
      CREWAI_FIRESTORE_EMULATOR: '127.0.0.1:8080', CREWAI_AUTH_EMULATOR: '127.0.0.1:9099',
    });
    expect(s?.emulator).toEqual({ firestore: '127.0.0.1:8080', auth: '127.0.0.1:9099' });
  });
  it('ignores a malformed saved file', () => {
    const h = home();
    writeFileSync(join(h, 'firebase.json'), '{nope');
    expect(loadFirebaseSettings(h, {})).toBeNull();
  });
});
