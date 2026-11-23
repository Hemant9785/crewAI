import { createHash } from 'node:crypto';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { latestTag, runUpdate, type Fetcher, type Response } from './update.js';

const dirs: string[] = [];
function tempDir() {
  const dir = mkdtempSync(join(tmpdir(), 'crewai-update-'));
  dirs.push(dir);
  return dir;
}

afterEach(() => {
  for (const dir of dirs.splice(0)) rmSync(dir, { recursive: true, force: true });
});

function response(body: string | Buffer, status = 200): Response {
  const data = Buffer.isBuffer(body) ? body : Buffer.from(body);
  return {
    ok: status >= 200 && status < 300,
    status,
    text: async () => data.toString(),
    arrayBuffer: async () => data.buffer.slice(data.byteOffset, data.byteOffset + data.byteLength) as ArrayBuffer,
  };
}

function checksums(file: string, data: Buffer) {
  return `${createHash('sha256').update(data).digest('hex')}  ${file}\n`;
}

describe('crewai updater', () => {
  it('falls back to the public release feed when the GitHub API is rate limited', async () => {
    const urls: string[] = [];
    const fetcher: Fetcher = async (url) => {
      urls.push(url);
      if (url.includes('api.github.com')) return response('rate limited', 403);
      return response('<feed><entry><link href="https://github.com/Hemant9785/crewAI/releases/tag/crewai-v0.3.2" /></entry></feed>');
    };

    await expect(latestTag(fetcher)).resolves.toBe('crewai-v0.3.2');
    expect(urls).toHaveLength(2);
    expect(urls[1]).toContain('/releases.atom');
  });

  it('reports up to date without downloading assets', async () => {
    const fetcher: Fetcher = async () => response(JSON.stringify([{ tag_name: 'crewai-v0.3.1' }]));
    const result = await runUpdate({ current: '0.3.1', execPath: '/tmp/crewai', fetcher, platform: 'linux', arch: 'x64' });

    expect(result.status).toBe('up-to-date');
  });

  it('only reports an available version during a dry run', async () => {
    const urls: string[] = [];
    const fetcher: Fetcher = async (url) => {
      urls.push(url);
      return response(JSON.stringify([{ tag_name: 'crewai-v0.3.2' }]));
    };
    const result = await runUpdate({ current: '0.3.1', execPath: '/tmp/crewai', fetcher, platform: 'linux', arch: 'x64', dryRun: true });

    expect(result.status).toBe('available');
    expect(urls).toHaveLength(1);
  });

  it('refuses a bad checksum and leaves the installed binary untouched', async () => {
    const dir = tempDir();
    const execPath = join(dir, 'crewai');
    const original = Buffer.from('working old binary');
    writeFileSync(execPath, original);
    const downloaded = Buffer.from('unexpected payload');
    const fetcher: Fetcher = async (url) => url.endsWith('SHA256SUMS')
      ? response(checksums('crewai-linux-x64', Buffer.from('different payload')))
      : response(downloaded);

    const result = await runUpdate({
      current: '0.3.1', execPath, env: { CREWAI_VERSION: 'crewai-v0.3.2', CREWAI_BASE_URL: 'https://test/releases' },
      fetcher, platform: 'linux', arch: 'x64', verify: async () => '0.3.2',
    });

    expect(result.status).toBe('error');
    expect(result.message).toContain('checksum');
    expect(readFileSync(execPath)).toEqual(original);
  });

  it('verifies the staged version before replacing the installed binary', async () => {
    const dir = tempDir();
    const execPath = join(dir, 'crewai');
    const original = Buffer.from('working old binary');
    const downloaded = Buffer.from('new binary');
    writeFileSync(execPath, original);
    const fetcher: Fetcher = async (url) => url.endsWith('SHA256SUMS')
      ? response(checksums('crewai-linux-x64', downloaded))
      : response(downloaded);

    const badVersion = await runUpdate({
      current: '0.3.1', execPath, env: { CREWAI_VERSION: 'crewai-v0.3.2', CREWAI_BASE_URL: 'https://test/releases' },
      fetcher, platform: 'linux', arch: 'x64', verify: async () => '0.3.9',
    });
    expect(badVersion.status).toBe('error');
    expect(readFileSync(execPath)).toEqual(original);

    const updated = await runUpdate({
      current: '0.3.1', execPath, env: { CREWAI_VERSION: 'crewai-v0.3.2', CREWAI_BASE_URL: 'https://test/releases' },
      fetcher, platform: 'linux', arch: 'x64', verify: async () => '0.3.2',
    });
    expect(updated.status).toBe('updated');
    expect(readFileSync(execPath)).toEqual(downloaded);
  });
});
