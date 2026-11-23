// Entry point embedded in the standalone binary. Node 24 can only start CommonJS from a
// single executable, but the app is an ES module (Ink uses top-level await). This launcher
// unpacks the embedded bundle into the user's own folder and imports it from there.
// It never uses a shared temp directory, so another local user cannot plant code.
const sea = require('node:sea');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const crypto = require('node:crypto');
const { pathToFileURL } = require('node:url');

try {
  const code = sea.getAsset('crewai.mjs', 'utf8');
  const hash = crypto.createHash('sha256').update(code).digest('hex').slice(0, 16);
  const home = process.env.CREWAI_HOME || path.join(os.homedir(), '.crewai');
  const dir = path.join(home, 'bin', hash);
  const file = path.join(dir, 'crewai.mjs');
  let current = '';
  try {
    current = fs.readFileSync(file, 'utf8');
  } catch {
    /* first run */
  }
  if (current !== code) {
    fs.mkdirSync(dir, { recursive: true });
    const tmp = `${file}.${process.pid}.tmp`;
    fs.writeFileSync(tmp, code);
    fs.renameSync(tmp, file);
    // A new version was unpacked: remove bundles left by older versions so they do not pile up.
    // One may still be in use by an older window; failures are ignored and retried next upgrade.
    try {
      for (const name of fs.readdirSync(path.join(home, 'bin'))) {
        if (name !== hash && /^[0-9a-f]{16}$/.test(name)) {
          try {
            fs.rmSync(path.join(home, 'bin', name), { recursive: true, force: true });
          } catch {
            /* in use */
          }
        }
      }
    } catch {
      /* nothing to clean */
    }
  }
  import(pathToFileURL(file).href).catch((e) => {
    process.stderr.write(`crewai failed to start: ${e && e.message ? e.message : e}\n`);
    process.exit(1);
  });
} catch (e) {
  process.stderr.write(`crewai failed to start: ${e && e.message ? e.message : e}\n`);
  process.exit(1);
}
