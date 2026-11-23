/**
 * Wraps text to a width, keeping blank lines and a paragraph's leading indent.
 * Long words with no spaces are broken so a line never exceeds the width.
 */
export function wrap(text: string, width: number): string[] {
  const w = Math.max(4, Math.floor(width));
  const out: string[] = [];
  for (const para of text.replace(/\r/g, '').split('\n')) {
    if (para.trim() === '') {
      out.push('');
      continue;
    }
    const indent = ' '.repeat(Math.min(8, Math.max(0, w - 4), /^ */.exec(para)![0].length));
    const room = w - indent.length;
    let line = '';
    for (const word of para.trim().split(/ +/)) {
      let rest = word;
      while (rest.length > room) {
        if (line) {
          out.push(indent + line);
          line = '';
        }
        out.push(indent + rest.slice(0, room));
        rest = rest.slice(room);
      }
      if (!line) line = rest;
      else if (line.length + 1 + rest.length <= room) line += ` ${rest}`;
      else {
        out.push(indent + line);
        line = rest;
      }
    }
    if (line) out.push(indent + line);
  }
  return out.length ? out : [''];
}

/** At most `maxLines` wrapped lines; the last one ends with an ellipsis when text was cut. */
export function wrapClamp(text: string, width: number, maxLines: number): string[] {
  const lines = wrap(text, width);
  if (lines.length <= maxLines) return lines;
  const kept = lines.slice(0, maxLines);
  const last = kept[maxLines - 1]!;
  kept[maxLines - 1] = `${last.length >= width ? last.slice(0, Math.max(1, width - 1)) : last}…`;
  return kept;
}
