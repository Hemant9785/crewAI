import { THREAD_LIMITS } from './types.js';

export type PrLink = { ok: true; url: string; label: string; host: string } | { ok: false; error: string };

/**
 * Checks a pull request link and gives it a short name such as `owner/repo#123`. Only plain https links are
 * accepted (no other schemes, no embedded passwords). crewai never opens or fetches the link itself; teammates
 * open it in their own browser.
 */
export function parsePrUrl(input: string): PrLink {
  const raw = input.trim();
  if (!raw) return { ok: false, error: 'Paste the link to the pull request.' };
  if (raw.length > THREAD_LIMITS.url) return { ok: false, error: 'That link is too long.' };
  if (/\s/.test(raw)) return { ok: false, error: "A link can't contain spaces." };
  let u: URL;
  try {
    u = new URL(raw);
  } catch {
    return { ok: false, error: "That doesn't look like a link. Paste the full address, starting with https://" };
  }
  if (u.protocol !== 'https:') return { ok: false, error: 'Use a link that starts with https://' };
  if (u.username || u.password) return { ok: false, error: "Links with a username or password aren't accepted." };
  const host = u.hostname.toLowerCase();
  if (!host.includes('.')) return { ok: false, error: "That link's address looks incomplete." };

  const path = u.pathname.replace(/\/+$/, '');
  let label: string | null = null;
  const gh = /^\/([^/]+)\/([^/]+)\/pull\/(\d+)/.exec(path);
  const gl = /^\/(.+?)\/-\/merge_requests\/(\d+)/.exec(path);
  const bb = /^\/([^/]+)\/([^/]+)\/pull-requests\/(\d+)/.exec(path);
  if (gh) label = `${gh[1]}/${gh[2]}#${gh[3]}`;
  else if (gl) label = `${gl[1]}!${gl[2]}`;
  else if (bb) label = `${bb[1]}/${bb[2]}#${bb[3]}`;
  if (!label) label = `${host}${path}`.slice(0, 80);
  return { ok: true, url: u.href, label, host };
}
