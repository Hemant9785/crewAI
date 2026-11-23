// Builds a standalone crewai executable for the OS and CPU this script runs on.
// It bundles the app with esbuild and embeds it in a copy of the Node binary
// (Node "single executable application"). Run it on each target OS; CI does that.
//
//   node scripts/build-binary.mjs        ->  release/crewai-<os>-<arch>[.exe] (+ .sha256)
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { chmodSync, copyFileSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const pkg = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8'));
const os = { win32: 'windows', darwin: 'macos', linux: 'linux' }[process.platform];
if (!os) throw new Error(`Unsupported platform ${process.platform}`);
const arch = process.arch === 'arm64' ? 'arm64' : process.arch === 'x64' ? 'x64' : null;
if (!arch) throw new Error(`Unsupported CPU ${process.arch}`);

// Public Firebase web config to embed (identifiers, not secrets; the security rules protect the data).
// Taken from CREWAI_FIREBASE_* variables, or from ~/.crewai/.env when building locally.
function dotenv(file) {
  const pairs = [];
  try {
    for (const line of readFileSync(file, 'utf8').split('\n')) {
      const m = /^\s*([A-Z0-9_]+)\s*=\s*(.*?)\s*$/.exec(line);
      if (m) pairs.push([m[1], m[2]]);
    }
  } catch {
    /* no file */
  }
  return Object.fromEntries(pairs);
}
const env = { ...dotenv(join(homedir(), '.crewai', '.env')), ...process.env };
const fb = {
  projectId: env.CREWAI_FIREBASE_PROJECT_ID,
  apiKey: env.CREWAI_FIREBASE_API_KEY,
  authDomain: env.CREWAI_FIREBASE_AUTH_DOMAIN,
  appId: env.CREWAI_FIREBASE_APP_ID,
};
const embedFirebase = Boolean(fb.projectId && fb.apiKey);
console.log(embedFirebase ? `Embedding Firebase project ${fb.projectId}` : 'No Firebase config set: users must provide their own.');

const exe = process.platform === 'win32' ? '.exe' : '';
const out = join(root, 'release');
const work = join(root, 'dist-bin');
const target = join(out, `crewai-${os}-${arch}${exe}`);

rmSync(work, { recursive: true, force: true });
mkdirSync(work, { recursive: true });
mkdirSync(out, { recursive: true });

// 1. One ES-module file with every dependency inside (Ink needs top-level await).
await build({
  entryPoints: [join(root, 'src/cli.tsx')],
  outfile: join(work, 'crewai.mjs'),
  bundle: true,
  platform: 'node',
  format: 'esm',
  target: 'node24',
  jsx: 'automatic',
  minify: true,
  legalComments: 'none',
  alias: { 'react-devtools-core': join(root, 'scripts', 'devtools-stub.mjs') },
  banner: {
    js: "import { createRequire as __crewaiCreateRequire } from 'node:module'; const require = __crewaiCreateRequire(import.meta.url);",
  },
  define: {
    __CREWAI_VERSION__: JSON.stringify(pkg.version),
    __CREWAI_FIREBASE__: embedFirebase ? JSON.stringify(JSON.stringify(fb)) : 'undefined',
    'process.env.NODE_ENV': '"production"',
  },
  logLevel: 'warning',
});

// 2. The embedded entry (CommonJS) unpacks and runs the bundle. Then inject it into a copy of Node.
writeFileSync(
  join(work, 'sea-config.json'),
  JSON.stringify({
    main: join(root, 'scripts', 'sea-entry.cjs'),
    output: join(work, 'sea-prep.blob'),
    disableExperimentalSEAWarning: true,
    assets: { 'crewai.mjs': join(work, 'crewai.mjs') },
  }),
);
execFileSync(process.execPath, ['--experimental-sea-config', join(work, 'sea-config.json')], { stdio: 'inherit' });

copyFileSync(process.execPath, target);
if (process.platform === 'darwin') execFileSync('codesign', ['--remove-signature', target], { stdio: 'inherit' });
const postject = join(root, 'node_modules', 'postject', 'dist', 'cli.js');
const args = [postject, target, 'NODE_SEA_BLOB', join(work, 'sea-prep.blob'), '--sentinel-fuse', 'NODE_SEA_FUSE_fce680ab2cc467b6e072b8b5df1996b2'];
if (process.platform === 'darwin') args.push('--macho-segment-name', 'NODE_SEA');
execFileSync(process.execPath, args, { stdio: 'inherit' });
if (process.platform === 'darwin') execFileSync('codesign', ['--sign', '-', target], { stdio: 'inherit' });
if (process.platform !== 'win32') chmodSync(target, 0o755);

// 3. Checksum next to the binary.
const sum = createHash('sha256').update(readFileSync(target)).digest('hex');
writeFileSync(`${target}.sha256`, `${sum}  crewai-${os}-${arch}${exe}\n`);
console.log(`Built ${target}\nsha256 ${sum}`);
