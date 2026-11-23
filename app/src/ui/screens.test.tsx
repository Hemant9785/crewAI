import { afterEach, describe, expect, it, vi } from 'vitest';
import { KEY, cleanupWorlds, tick, world } from '../testing/uiWorld.js';

afterEach(cleanupWorlds);

describe('wide layout (split view)', () => {
  it('shows team, tasks and conversation side by side with 20 tasks grouped and clipped', async () => {
    const w = await world(20);
    const m = await w.mount(140, 60); // tall enough to see every group
    const f = m.frame();
    for (const part of ['TEAM', 'TASKS · 20', 'CONVERSATION', 'NEEDS HELP · 1', 'IN PROGRESS · 3', 'TO DO']) expect(f).toContain(part);
    expect(f).toContain('✓ 5 done');
    expect(f).toContain('show (v)'); // finished tasks stay collapsed
    expect(f).toContain('mine');
    const longRow = f.split('\n').find((l) => l.includes('T-1 Implement'))!;
    expect(longRow).toContain('…'); // one clipped line, not a wall of text
    expect(f.split('\n').every((l) => l.length <= 140)).toBe(true);
  });

  it('marks what is hidden when the list does not fit', async () => {
    const w = await world(30);
    const m = await w.mount(140, 24);
    expect(m.frame()).toMatch(/▼ \d+ more/);
  });

  it('keeps the message box active until Tab, then moves keys to the task panel and back', async () => {
    const w = await world(6);
    const m = await w.mount(140);
    await m.type('half a thought');
    await m.type(KEY.tab); // a draft is in the box, so Tab is not taken
    expect(m.frame()).toContain('half a thought');
    expect(m.frame()).not.toContain('enter to open');
    await m.type('\u0015'); // ctrl+u clears
    await m.type(KEY.tab);
    await vi.waitFor(() => expect(m.frame()).toContain('enter to open'));
    await m.type(KEY.esc);
    await vi.waitFor(() => expect(m.frame()).toContain('tab to focus'));
  });

  it('peeks at the selected task, opens it in full, and wraps the whole description', async () => {
    const w = await world(6);
    const m = await w.mount(140);
    await m.type(KEY.tab);
    await vi.waitFor(() => expect(m.frame()).toContain('enter to open'));
    // Move down to T-1 (the long one); its first lines show in the peek area under the list.
    for (let i = 0; i < 8 && !m.frame().includes('T-1 · you · from Boss'); i++) await m.type(KEY.down);
    expect(m.frame()).toContain('T-1 · you · from Boss');
    expect(m.frame()).toContain('Implement the submit button'); // peek shows the start of the text
    await m.type(KEY.enter);
    await vi.waitFor(() => expect(m.frame()).toContain('esc back to chat'));
    const f = m.frame();
    expect(f).toContain('Figma spec');
    expect(f).toContain('From Boss');
    expect(f).toContain('Boss assigned it');
    expect(f).toContain('[d] Done');
    expect(f.split('\n').every((l) => l.length <= 140)).toBe(true);
    await m.type(KEY.esc);
    await vi.waitFor(() => expect(m.frame()).toContain('CONVERSATION'));
  });

  it('marks a task done from the panel and tells the team', async () => {
    const w = await world(3);
    const m = await w.mount(140);
    await m.type(KEY.tab);
    await tick(80);
    await m.type('d');
    await vi.waitFor(() => expect(w.holder.engine!.state.tasks.some((t) => t.status === 'done')).toBe(true));
    await vi.waitFor(() => expect(m.frame()).toContain('done by Boss'));
  });

  it('says what to do when no agent is installed', async () => {
    const w = await world(0, { agents: [] });
    const m = await w.mount(140);
    expect(m.frame()).toContain('no agent · install Codex or Claude Code, then /agent');
  });
});

describe('long folder paths', () => {
  const LONG_DIR = 'a-very-long-project-folder-name-'.repeat(3);

  it('keeps the no-agent hint whole when the path is long, at every width', async () => {
    for (const columns of [140, 100, 80]) {
      const w = await world(0, { agents: [] });
      const m = await w.mount(columns, 30, LONG_DIR);
      expect(m.frame()).toContain('no agent · install Codex or Claude Code, then /agent');
    }
  });

  it('keeps "esc back to chat" in the task view header when the path is long', async () => {
    const w = await world(3);
    const m = await w.mount(100, 30, LONG_DIR);
    await m.type(KEY.tab);
    await vi.waitFor(() => expect(m.frame()).toContain('esc back to chat'));
    expect(m.frame().split('\n')[0]!.length).toBeLessThanOrEqual(100);
  });
});

describe('narrower terminals', () => {
  it('collapses the side column into a one-line strip at 80 to 109 columns, and Tab opens the task view', async () => {
    const w = await world(8);
    const m = await w.mount(100);
    const f = m.frame();
    expect(f).not.toContain('TEAM');
    expect(f).toMatch(/tasks ▸ \d+ · ! \d+ · ○ \d+ · ✓ \d+/);
    await m.type(KEY.tab);
    await vi.waitFor(() => expect(m.frame()).toContain('esc back to chat'));
    expect(m.frame()).toContain('8 TASKS');
    await m.type(KEY.esc);
    await vi.waitFor(() => expect(m.frame()).toContain('CONVERSATION'));
  });

  it('is one column under 80 columns, with a summary line and a list then detail flow', async () => {
    const w = await world(5);
    const m = await w.mount(70);
    expect(m.frame()).toMatch(/\d+ online · \d+ open tasks · \/tasks/);
    await m.type('/tasks');
    await m.type(KEY.enter);
    await vi.waitFor(() => expect(m.frame()).toContain('5 TASKS'));
    expect(m.frame()).not.toContain('From Boss'); // list only
    await m.type(KEY.enter);
    await vi.waitFor(() => expect(m.frame()).toContain('From Boss'));
    expect(m.frame()).not.toContain('5 TASKS'); // detail only
    await m.type(KEY.esc);
    await vi.waitFor(() => expect(m.frame()).toContain('5 TASKS'));
    expect(m.frame().split('\n').every((l) => l.length <= 70)).toBe(true);
  });

  it('shows approvals above the message box when there is no side column', async () => {
    const w = await world(0);
    const hem = w.cloud.backendFor('u-Hemant');
    await hem.createRequest(w.team.id, { targetUid: 'u-boss', targetName: 'Boss', question: 'how is auth done?' });
    const m = await w.mount(100);
    await vi.waitFor(() => expect(m.frame()).toContain('Hemant asks about your project'));
    expect(m.frame()).toContain('[Allow once]');
  });

  it('puts approvals in a "needs you" panel in the wide layout', async () => {
    const w = await world(0);
    const hem = w.cloud.backendFor('u-Hemant');
    await hem.createRequest(w.team.id, { targetUid: 'u-boss', targetName: 'Boss', question: 'how is auth done?' });
    const m = await w.mount(140);
    await vi.waitFor(() => expect(m.frame()).toContain('NEEDS YOU · 1'));
    expect(m.frame()).toContain('Hemant asks about your');
  });
});

describe('scrolling the conversation', () => {
  it('scrolls with page keys, shows where you are, and holds the place while new messages arrive', async () => {
    const w = await world(0);
    const m = await w.mount(120, 24);
    for (let i = 1; i <= 40; i++) await w.boss.sendBroadcast(w.team.id, `message number ${i}`);
    await vi.waitFor(() => expect(m.frame()).toContain('message number 40'));
    expect(m.frame()).toMatch(/↑ \d+ earlier/);
    expect(m.frame()).not.toContain('message number 1\n');

    await m.type(KEY.pgup);
    await vi.waitFor(() => expect(m.frame()).toMatch(/↓ \d+ newer/));
    const holding = m.frame().split('\n').find((l) => /message number \d+/.test(l))!;
    await w.boss.sendBroadcast(w.team.id, 'message number 41');
    await tick(150);
    expect(m.frame()).toContain(holding.trim().slice(0, 30)); // the line you were reading did not jump away

    await m.type(KEY.pgdn);
    await m.type(KEY.pgdn);
    await vi.waitFor(() => expect(m.frame()).toContain('message number 41'));
    expect(m.frame()).not.toMatch(/↓ \d+ newer/);
  });
});
