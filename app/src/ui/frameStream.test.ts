import { describe, expect, it } from 'vitest';
import { createFrameWriter } from './frameStream.js';

const ESC = '\u001b';
// What Ink's default renderer writes for a frame after a previous one with `prevLines` lines.
const inkFrame = (lines: string[], prevLines: number) =>
  `${`${ESC}[2K${ESC}[1A`.repeat(Math.max(0, prevLines - 1))}${prevLines ? `${ESC}[2K${ESC}[G` : ''}${lines.join('\n')}\n`;

function setup(rows = 6, columns = 20) {
  const out = { rows, columns, writes: [] as string[], write(c: string) { this.writes.push(c); return true; } };
  const fw = createFrameWriter(out);
  return { out, fw, last: () => out.writes[out.writes.length - 1] ?? '' };
}

describe('frame writer', () => {
  it('paints the first frame in full at absolute positions', () => {
    const { fw, last } = setup();
    fw.write(inkFrame(['one', 'two', 'three'], 0));
    expect(last()).toContain(`${ESC}[2J`);
    expect(last()).toContain(`${ESC}[1;1Hone`);
    expect(last()).toContain(`${ESC}[3;1Hthree`);
  });

  it('writes only the lines that changed, and never erases the screen', () => {
    const { fw, last, out } = setup();
    fw.write(inkFrame(['one', 'two', 'three'], 0));
    fw.write(inkFrame(['one', 'twenty', 'three'], 3));
    const second = last();
    expect(second).toContain(`${ESC}[2;1Htwenty${ESC}[K`);
    expect(second).not.toContain('one');
    expect(second).not.toContain('three');
    expect(second).not.toContain(`${ESC}[2J`);
    expect(second).not.toContain(`${ESC}[2K`); // no erase-then-redraw gap
    expect(out.writes).toHaveLength(2);
  });

  it('writes nothing when the frame is unchanged', () => {
    const { fw, out } = setup();
    fw.write(inkFrame(['one', 'two'], 0));
    fw.write(inkFrame(['one', 'two'], 2));
    expect(out.writes).toHaveLength(1);
  });

  it('clears rows that are no longer used when the frame gets shorter', () => {
    const { fw, last } = setup();
    fw.write(inkFrame(['one', 'two', 'three'], 0));
    fw.write(inkFrame(['one'], 3));
    expect(last()).toContain(`${ESC}[2;1H${ESC}[K`);
    expect(last()).toContain(`${ESC}[3;1H${ESC}[K`);
  });

  it('repaints everything after a resize', () => {
    const { fw, last, out } = setup();
    fw.write(inkFrame(['one', 'two'], 0));
    out.columns = 40;
    fw.write(inkFrame(['one', 'two'], 2));
    expect(last()).toContain(`${ESC}[2J`);
    expect(last()).toContain('two');
  });

  it('passes other output straight through and repaints afterwards', () => {
    const { fw, last, out } = setup();
    fw.write(inkFrame(['one', 'two'], 0));
    fw.write(`${ESC}[?25l`);
    expect(last()).toBe(`${ESC}[?25l`);
    fw.write(inkFrame(['one', 'two'], 2));
    expect(last()).toContain(`${ESC}[2J`);
    expect(out.writes).toHaveLength(3);
  });

  it('never draws past the bottom of the screen', () => {
    const { fw, last } = setup(3);
    fw.write(inkFrame(['a', 'b', 'c', 'd', 'e'], 0));
    expect(last()).toContain('c');
    expect(last()).not.toContain('d');
  });

  it('writes far less than a full redraw for one changed line', () => {
    const { fw, out } = setup(40, 140);
    const frame = (n: number) => Array.from({ length: 39 }, (_, i) => `row ${i} ${'x'.repeat(120)}${i === 20 ? n : ''}`);
    fw.write(inkFrame(frame(0), 0));
    const before = out.writes.join('').length;
    fw.write(inkFrame(frame(1), 39));
    const delta = out.writes.join('').length - before;
    expect(delta).toBeLessThan(300);
  });
});
