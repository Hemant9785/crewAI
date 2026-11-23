import type { Entry } from '../config.js';
import { hhmm } from '../format.js';
import { wrap } from './textlayout.js';

export interface Seg {
  t: string;
  color?: string;
  bold?: boolean;
  dim?: boolean;
}
export type Line = Seg[];

/** Width of the time column plus its gap. Body text lines up under the speaker. */
export const INDENT = 7;

export function dayName(ms: number, now: number): string {
  const d = new Date(ms);
  if (d.toDateString() === new Date(now).toDateString()) return 'Today';
  if (d.toDateString() === new Date(now - 86_400_000).toDateString()) return 'Yesterday';
  return d.toLocaleDateString(undefined, { weekday: 'short', month: 'short', day: 'numeric' });
}

const BLANK: Line = [{ t: '' }];
const pad = (n: number): Seg => ({ t: ' '.repeat(n) });

/** Turns the chat log into display lines for a given width: a time column, colored speakers, day markers. */
export function buildFeed(entries: Entry[], width: number, now: number): Line[] {
  const body = Math.max(8, width - INDENT);
  const out: Line[] = [];
  const blankBefore = () => {
    if (out.length && out[out.length - 1] !== BLANK) out.push(BLANK);
  };
  let day = '';
  for (const e of entries) {
    const d = dayName(e.at, now);
    if (d !== day) {
      day = d;
      blankBefore();
      out.push([{ t: `── ${d} ──`, dim: true }]);
    }
    const time: Seg = { t: `${hhmm(e.at)}  `, dim: true };
    const bodyLines = (text: string, color?: string, dim?: boolean): Line[] =>
      wrap(text, body).map((l) => [pad(INDENT), { t: l, color, dim }]);

    switch (e.kind) {
      case 'you':
        blankBefore();
        out.push([time, { t: 'you', color: 'cyan', bold: true }]);
        out.push(...bodyLines(e.text));
        break;
      case 'agent':
        blankBefore();
        out.push([time, { t: `✦ ${e.who}`, color: 'magenta', bold: true }]);
        out.push(...bodyLines(e.text));
        if (e.meta) out.push(...bodyLines(e.meta, undefined, true));
        break;
      case 'broadcast':
        blankBefore();
        out.push([time, { t: e.who, color: 'yellow', bold: true }, { t: ' broadcast', color: 'yellow' }]);
        out.push(...bodyLines(e.text));
        break;
      case 'task': {
        const lines = wrap(e.text, body - 2);
        lines.forEach((l, i) =>
          out.push(i === 0 ? [time, { t: '▸ ', color: 'blue' }, { t: l }] : [pad(INDENT + 2), { t: l }]),
        );
        break;
      }
      case 'thread': {
        const lines = wrap(e.text, body - 2);
        lines.forEach((l, i) =>
          out.push(i === 0 ? [time, { t: '\u2709 ', color: 'cyan' }, { t: l, color: 'cyan' }] : [pad(INDENT + 2), { t: l, color: 'cyan' }]),
        );
        break;
      }
      default: {
        wrap(e.text, body).forEach((l, i) => out.push(i === 0 ? [time, { t: l, dim: true }] : [pad(INDENT), { t: l, dim: true }]));
      }
    }
  }
  return out;
}
