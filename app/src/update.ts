import { execFile } from 'node:child_process';
import { createHash } from 'node:crypto';
import { chmodSync, existsSync, renameSync, rmSync, writeFileSync } from 'node:fs';
import { basename } from 'node:path';

export const REPO = 'Hemant9785/crewAI';
const TAG_PREFIX = 'crewai-v';

export interface Response {
  ok: boolean;
  status: number;
  text(): Promise<string>;
  arrayBuffer(): Promise<ArrayBuffer>;
}
export type Fetcher = (url: string, init?: { headers?: Record<string, string> }) => Promise<Response>;

export const realFetcher: Fetcher = (url, init) =>
  fetch(url, { headers: { 'user-agent': 'crewai-updater', ...init?.headers }, signal: AbortSignal.timeout(300_000) });

/** The file name of this system's binary in a release, or null when there is no build for it. */
export function assetName(platform: string = process.platform, arch: string = process.arch): string | null {
  const os = platform === 'win32' ? 'windows' : platform === 'darwin' ? 'macos' : platform === 'linux' ? 'linux' : null;
  const cpu = arch === 'x64' ? 'x64' : arch === 'arm64' ? 'arm64' : null;
  if (!os || !cpu) return null;
  if (os === 'windows' && cpu !== 'x64') return null;
  return `crewai-${os}-${cpu}${os === 'windows' ? '.exe' : ''}`;
}

export const tagVersion = (tag: string): string => tag.replace(new RegExp(`^${TAG_PREFIX}`), '');

/** Compares the numeric parts of two versions: negative when a is older than b. */
export function compareVersions(a: string, b: string): number {
  const nums = (v: string) => v.split('-')[0]!.split('.').map((n) => Number.parseInt(n, 10) || 0);
  const x = nums(a);
  const y = nums(b);
  for (let i = 0; i < Math.max(x.length, y.length); i++) {
    const d = (x[i] ?? 0) - (y[i] ?? 0);
    if (d !== 0) return d;
  }
  return 0;
}

export function parseLatestFromApi(json: unknown): string | null {
  if (!Array.isArray(json)) return null;
  for (const r of json as { tag_name?: string; draft?: boolean; prerelease?: boolean }[]) {
    if (r.tag_name?.startsWith(TAG_PREFIX) && !r.draft && !r.prerelease) return r.tag_name;
  }
  return null;
}

export function parseLatestFromAtom(xml: string): string | null {
  const m = new RegExp(`releases/tag/(${TAG_PREFIX}[^"<]+)`).exec(xml);
  return m ? m[1]! : null;
}

/** Newest crewai release tag. The API is rate limited per address, so fall back to the public release feed. */
export async function latestTag(fetcher: Fetcher): Promise<string> {
  try {
    const r = await fetcher(`https://api.github.com/repos/${REPO}/releases?per_page=30`, { headers: { accept: 'application/vnd.github+json' } });
    if (r.ok) {
      const tag = parseLatestFromApi(JSON.parse(await r.text()));
      if (tag) return tag;
    }
  } catch {
    /* fall through to the feed */
  }
  const f = await fetcher(`https://github.com/${REPO}/releases.atom`);
  if (!f.ok) throw new Error(`Could not look up releases (HTTP ${f.status}).`);
  const tag = parseLatestFromAtom(await f.text());
  if (!tag) throw new Error(`No crewai release was found at https://github.com/${REPO}/releases yet.`);
  return tag;
}

export function checksumFor(sums: string, file: string): string | null {
  for (const line of sums.split(/\r?\n/)) {
    const m = /^([0-9a-fA-F]{64})\s+\*?(.+?)\s*$/.exec(line);
    if (m && m[2] === file) return m[1]!.toLowerCase();
  }
  return null;
}

/** Runs the downloaded binary and returns the version it reports. */
export const runVersion = (file: string): Promise<string> =>
  new Promise((resolve, reject) => {
    execFile(file, ['--version'], { timeout: 30_000, windowsHide: true }, (err, stdout) => (err ? reject(err) : resolve(String(stdout).trim())));
  });

/** True when this process is the installed program, not node running the source. */
export const isStandalone = (execPath: string): boolean => /^crewai(-.*)?(\.exe)?$/i.test(basename(execPath));

export interface UpdateOptions {
  current: string;
  execPath?: string;
  fetcher?: Fetcher;
  env?: NodeJS.ProcessEnv;
  platform?: string;
  arch?: string;
  dryRun?: boolean;
  verify?: (file: string) => Promise<string>;
  log?: (line: string) => void;
}
export interface UpdateResult {
  status: 'up-to-date' | 'available' | 'updated' | 'error';
  from: string;
  to?: string;
  message: string;
}

/**
 * Updates the installed binary in place. The download is checked against the release's SHA256SUMS, and the
 * new binary must run and report the expected version before it replaces the old one, so a bad download can
 * never leave the user without a working crewai.
 */
export async function runUpdate(o: UpdateOptions): Promise<UpdateResult> {
  const log = o.log ?? (() => {});
  const env = o.env ?? process.env;
  const execPath = o.execPath ?? process.execPath;
  const fetcher = o.fetcher ?? realFetcher;
  const platform = o.platform ?? process.platform;
  const from = o.current;
  const fail = (message: string): UpdateResult => ({ status: 'error', from, message });

  if (!isStandalone(execPath)) {
    return fail("You're running crewai from source, not the installed program. Update it with git pull and npm run build.");
  }
  const file = assetName(platform, o.arch ?? process.arch);
  if (!file) return fail(`There is no crewai build for ${platform} on this processor yet.`);

  let tag = env.CREWAI_VERSION;
  const pinned = Boolean(tag);
  try {
    if (!tag) {
      if (env.CREWAI_BASE_URL) return fail('Set CREWAI_VERSION too when you set CREWAI_BASE_URL.');
      log('Looking up the latest crewai release...');
      tag = await latestTag(fetcher);
    }
  } catch (e) {
    return fail(e instanceof Error ? e.message : 'Could not look up the latest release.');
  }
  const to = tagVersion(tag);
  const same = compareVersions(to, from) === 0;
  if (pinned ? same : compareVersions(to, from) <= 0) {
    return { status: 'up-to-date', from, to, message: `crewai ${from} is up to date.` };
  }
  if (o.dryRun) return { status: 'available', from, to, message: `crewai ${to} is available (you have ${from}). Run crewai --update to install it.` };

  const base = (env.CREWAI_BASE_URL ?? `https://github.com/${REPO}/releases/download/${tag}`).replace(/\/+$/, '');
  const staged = `${execPath}.new`;
  const old = `${execPath}.old`;
  try {
    log(`Downloading ${file} (${to}) ...`);
    const bin = await fetcher(`${base}/${file}`);
    if (!bin.ok) return fail(`Could not download ${file} (HTTP ${bin.status}).`);
    const sums = await fetcher(`${base}/SHA256SUMS`);
    if (!sums.ok) return fail(`Could not download SHA256SUMS (HTTP ${sums.status}).`);
    const expected = checksumFor(await sums.text(), file);
    if (!expected) return fail(`SHA256SUMS has no entry for ${file}. Nothing was changed.`);
    const data = Buffer.from(await bin.arrayBuffer());
    const actual = createHash('sha256').update(data).digest('hex');
    if (actual !== expected) return fail(`The download does not match its checksum (expected ${expected}, got ${actual}). Nothing was changed.`);
    log('Checksum verified.');

    rmSync(staged, { force: true });
    writeFileSync(staged, data, { mode: 0o755 });
    if (platform !== 'win32') chmodSync(staged, 0o755);
    let reported = '';
    try {
      reported = await (o.verify ?? runVersion)(staged);
    } catch {
      rmSync(staged, { force: true });
      return fail("The downloaded program would not start, so it was not installed. Nothing was changed.");
    }
    if (reported !== to) {
      rmSync(staged, { force: true });
      return fail(`The downloaded program reports version ${reported || 'unknown'}, not ${to}, so it was not installed. Nothing was changed.`);
    }

    if (platform === 'win32') {
      // A running .exe cannot be overwritten, but it can be renamed. The old copy is deleted on the next start.
      rmSync(old, { force: true });
      renameSync(execPath, old);
      try {
        renameSync(staged, execPath);
      } catch (e) {
        renameSync(old, execPath); // put the working version back
        throw e;
      }
    } else {
      renameSync(staged, execPath); // atomic; running copies keep the old file until they exit
    }
  } catch (e) {
    rmSync(staged, { force: true });
    const code = (e as NodeJS.ErrnoException).code;
    if (code === 'EACCES' || code === 'EPERM') {
      return fail(`No permission to write ${execPath}. Run the update again as an administrator${platform === 'win32' ? '' : ' (sudo)'}, or reinstall with the installer.`);
    }
    return fail(`The update failed: ${e instanceof Error ? e.message : String(e)}`);
  }
  return {
    status: 'updated',
    from,
    to,
    message: `Updated crewai ${from} → ${to}. Close and reopen any crewai windows that are still open to use it.`,
  };
}

/** Removes leftovers of an earlier update. Safe to call at every start. */
export function cleanupAfterUpdate(execPath: string = process.execPath): void {
  if (!isStandalone(execPath)) return;
  for (const f of [`${execPath}.old`, `${execPath}.new`]) {
    if (existsSync(f)) {
      try {
        rmSync(f, { force: true });
      } catch {
        /* still in use by an open window; try again next time */
      }
    }
  }
}
