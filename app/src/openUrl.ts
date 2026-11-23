import { spawn } from 'node:child_process';
import { parsePrUrl } from './pr.js';

/**
 * Opens a link in the person's default browser. Only plain https links that pass the same check as a pull
 * request link are opened, and the address is passed as one argument (never through a shell), so a link can
 * never run a command. Returns false when the link is not acceptable or no opener could be started.
 */
export function openUrl(url: string, platform: string = process.platform): boolean {
  const checked = parsePrUrl(url);
  if (!checked.ok) return false;
  const [cmd, args]: [string, string[]] =
    platform === 'win32' ? ['rundll32', ['url.dll,FileProtocolHandler', checked.url]] : platform === 'darwin' ? ['open', [checked.url]] : ['xdg-open', [checked.url]];
  try {
    const child = spawn(cmd, args, { detached: true, stdio: 'ignore', windowsHide: true });
    child.on('error', () => {});
    child.unref();
    return true;
  } catch {
    return false;
  }
}
