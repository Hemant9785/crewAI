const ESC = '\u001b';
const BSU = `${ESC}[?2026h`;
const ESU = `${ESC}[?2026l`;
// What Ink's default renderer puts in front of every frame: erase each previous line, then go to column 1.
const ERASE_PREFIX = new RegExp(`^(?:${ESC}\\[2K${ESC}\\[1A)*${ESC}\\[2K${ESC}\\[G`);

export interface TermOut {
  rows?: number;
  columns?: number;
  write(chunk: string): boolean;
}

/**
 * A stream for Ink on the terminal's alternate screen. Ink redraws by erasing every previous line and
 * writing the whole frame again, which flickers in terminals that do not support synchronized output
 * (many Windows consoles). This keeps the previous frame and writes only the lines that changed, each at
 * an absolute screen position, so nothing is erased and unchanged lines are never touched.
 * Anything that is not a frame (hiding the cursor, for example) passes through untouched.
 */
export function createFrameWriter(out: TermOut): { write: (chunk: string) => boolean; reset: () => void } {
  let prev: string[] | null = null;
  let prevRows = 0;
  let prevCols = 0;

  const write = (chunk: string): boolean => {
    if (chunk === BSU || chunk === ESU) return out.write(chunk);
    const m = ERASE_PREFIX.exec(chunk);
    const isFrame = m !== null || (prev === null && chunk.includes('\n'));
    if (!isFrame) {
      prev = null; // something else touched the screen, so repaint everything next time
      return out.write(chunk);
    }
    const body = m ? chunk.slice(m[0].length) : chunk;
    const rows = out.rows ?? 30;
    const cols = out.columns ?? 100;
    let lines = body.split('\n');
    if (lines.length > 1 && lines[lines.length - 1] === '') lines = lines.slice(0, -1);
    lines = lines.slice(0, rows);

    let buf = '';
    if (prev === null || rows !== prevRows || cols !== prevCols) {
      buf = `${ESC}[2J${ESC}[H`;
      lines.forEach((l, i) => {
        buf += `${ESC}[${i + 1};1H${l}`;
      });
    } else {
      const n = Math.max(lines.length, prev.length);
      for (let i = 0; i < n; i++) {
        if (lines[i] !== prev[i]) buf += `${ESC}[${i + 1};1H${lines[i] ?? ''}${ESC}[K`;
      }
    }
    prev = lines;
    prevRows = rows;
    prevCols = cols;
    return buf ? out.write(BSU + buf + ESU) : true;
  };

  return { write, reset: () => (prev = null) };
}

/** Wraps a real stream so Ink writes through the frame writer but still sees its size and events. */
export function withFrameWriter<T extends NodeJS.WriteStream>(out: T): T {
  const fw = createFrameWriter(out as unknown as TermOut);
  return new Proxy(out, {
    get(target, prop, receiver) {
      if (prop === 'write') return fw.write;
      const v = Reflect.get(target, prop, target) as unknown;
      return typeof v === 'function' ? (v as (...a: unknown[]) => unknown).bind(target) : v;
    },
  }) as T;
}
