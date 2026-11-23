import { measureElement, useStdout, type DOMElement } from 'ink';
import { useEffect, useRef, useState } from 'react';

export interface Size {
  columns: number;
  rows: number;
}
export type LayoutMode = 'wide' | 'medium' | 'narrow';

export const WIDE_AT = 110;
export const MEDIUM_AT = 80;

export function layoutMode(columns: number): LayoutMode {
  return columns >= WIDE_AT ? 'wide' : columns >= MEDIUM_AT ? 'medium' : 'narrow';
}

/** Terminal size, kept current when the window is resized. */
export function useTerminalSize(): Size {
  const { stdout } = useStdout();
  const read = (): Size => ({ columns: stdout?.columns || 100, rows: stdout?.rows || 30 });
  const [size, setSize] = useState<Size>(read);
  useEffect(() => {
    if (!stdout) return;
    const onResize = () => setSize(read());
    stdout.on('resize', onResize);
    onResize(); // the window may have changed between the first render and this listener attaching
    return () => {
      stdout.off('resize', onResize);
    };
  }, [stdout]);
  return size;
}

/** Measures how many lines a box was given by the layout, so lists can size their windows. */
export function useHeight(initial: number): [React.RefObject<DOMElement | null>, number] {
  const ref = useRef<DOMElement | null>(null);
  const [h, setH] = useState(initial);
  useEffect(() => {
    if (!ref.current) return;
    const m = measureElement(ref.current);
    if (m.height > 0 && m.height !== h) setH(m.height);
  });
  return [ref, h];
}
