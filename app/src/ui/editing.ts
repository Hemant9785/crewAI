import type { Key } from 'ink';

export interface EditResult {
  value: string;
  cursor: number;
}

const CONTROL = /[\u0000-\u0008\u000b-\u001f\u007f]/g;

/** Single-line text editing shared by the composer and the onboarding inputs. */
export function applyEdit(value: string, cursor: number, input: string, key: Key): EditResult | null {
  if (key.leftArrow) return { value, cursor: Math.max(0, cursor - 1) };
  if (key.rightArrow) return { value, cursor: Math.min(value.length, cursor + 1) };
  // Terminals disagree on Backspace versus Delete, so both remove the character before the cursor.
  if (key.backspace || key.delete) {
    if (cursor === 0) return { value, cursor };
    return { value: value.slice(0, cursor - 1) + value.slice(cursor), cursor: cursor - 1 };
  }
  if (key.ctrl) {
    if (input === 'a') return { value, cursor: 0 };
    if (input === 'e') return { value, cursor: value.length };
    if (input === 'u') return { value: value.slice(cursor), cursor: 0 };
    if (input === 'w') {
      const head = value.slice(0, cursor).replace(/\S+\s*$/, '');
      return { value: head + value.slice(cursor), cursor: head.length };
    }
    return null;
  }
  if (key.meta || key.escape || key.return || key.tab || key.upArrow || key.downArrow) return null;
  if (!input) return null;
  const clean = input.replace(/\r?\n/g, ' ').replace(CONTROL, '');
  if (!clean) return null;
  return { value: value.slice(0, cursor) + clean + value.slice(cursor), cursor: cursor + clean.length };
}
