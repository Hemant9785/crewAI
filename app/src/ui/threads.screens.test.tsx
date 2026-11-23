import { afterEach, describe, expect, it, vi } from 'vitest';
import { MemoryStore } from '../memoryBackend.js';
import type { Thread } from '../types.js';
import { KEY, cleanupWorlds, tick, world } from '../testing/uiWorld.js';

afterEach(cleanupWorlds);

const PR = 'https://github.com/acme/web/pull/88';
// Panels wrap long text, so compare with the box edges and line breaks removed.
const flat = (f: string) => f.replace(/[│╭╮╰╯─\n]+/g, ' ').replace(/\s+/g, ' ');
const mine = (cloud: MemoryStore, teamId: string, who: string) => {
  let latest: Thread[] = [];
  cloud.backendFor(who).subscribeThreads(teamId, (t) => (latest = t))();
  return latest;
};

describe('private conversations on screen', () => {
  it('shows an empty state with the commands to start one', async () => {
    const w = await world(0);
    const m = await w.mount(120, 30);
    await m.type('/threads');
    await m.type(KEY.enter);
    await vi.waitFor(() => expect(m.frame()).toContain('0 CONVERSATIONS'));
    expect(flat(m.frame())).toContain('/followup T-4');
    expect(flat(m.frame())).toContain('/review link @name');
    await m.type(KEY.esc);
    await vi.waitFor(() => expect(m.frame()).toContain('CONVERSATION'));
  });

  it('badges a new question, lets you read and reply, and clears the badge once read', async () => {
    const w = await world(0);
    const hem = w.cloud.backendFor('u-Hemant');
    const m = await w.mount(120, 30);
    await hem.createThread(w.team.id, {
      kind: 'followup', title: 'Which design?', participants: [{ uid: 'u-boss', name: 'Boss' }], text: 'Which design should the form follow?',
    });
    await vi.waitFor(() => expect(m.frame()).toContain('✉ 1 new · /threads'));
    await m.type('/threads');
    await m.type(KEY.enter);
    await vi.waitFor(() => expect(m.frame()).toContain('1 CONVERSATION'));
    expect(m.frame()).toContain('Hemant: Which'); // the row is cut to the list width
    await m.type(KEY.enter); // open it
    await vi.waitFor(() => expect(m.frame()).toContain('Follow-up · with Hemant'));
    expect(m.frame()).toContain('Which design should the form follow?');
    for (const ch of 'Use the new one') await m.type(ch);
    await m.type(KEY.enter);
    await vi.waitFor(() => expect(mine(w.cloud, w.team.id, 'u-Hemant')[0]?.lastText).toBe('Use the new one'));
    await vi.waitFor(() => expect(m.frame()).toContain('Use the new one'));
    await m.type(KEY.esc); // list
    await m.type(KEY.esc); // back to chat
    await vi.waitFor(() => expect(m.frame()).toContain('CONVERSATION'));
    expect(m.frame()).not.toContain('new · /threads');
  });

  it('keeps a conversation out of a teammate\'s screen', async () => {
    const w = await world(0);
    const hem = w.cloud.backendFor('u-Hemant');
    const m = await w.mount(120, 30);
    // Hemant starts a private question to Ravi, not to Boss.
    await hem.createThread(w.team.id, { kind: 'followup', title: 'secret', participants: [{ uid: 'u-Ravi', name: 'Ravi' }], text: 'private question for Ravi only' });
    await tick(200);
    expect(m.frame()).not.toContain('new · /threads');
    expect(m.frame()).not.toContain('private question');
    await m.type('/threads');
    await m.type(KEY.enter);
    await vi.waitFor(() => expect(m.frame()).toContain('0 CONVERSATIONS'));
    expect(m.frame()).not.toContain('private question');
  });

  it('opens a review, shows the link, and sends a verdict with the reply', async () => {
    const w = await world(0);
    const hem = w.cloud.backendFor('u-Hemant');
    const m = await w.mount(120, 30);
    await hem.createThread(w.team.id, {
      kind: 'review', title: 'acme/web#88', prUrl: PR, participants: [{ uid: 'u-boss', name: 'Boss' }],
      text: 'Please check the retry logic in api/client.ts.',
    });
    await vi.waitFor(() => expect(m.frame()).toContain('✉ 1 new'));
    expect(m.frame()).toContain('asked you to review acme/web#88');
    await m.type('/threads');
    await m.type(KEY.enter);
    await vi.waitFor(() => expect(m.frame()).toContain('acme/web#88'));
    await m.type(KEY.enter);
    await vi.waitFor(() => expect(m.frame()).toContain(PR));
    expect(m.frame()).toContain('ctrl+o opens it');
    expect(m.frame()).toContain('Approve');
    expect(m.frame()).toContain('Request changes');
    await m.type(KEY.tab); // Comment -> Approve
    for (const ch of 'Looks good to me') await m.type(ch);
    await m.type(KEY.enter);
    await vi.waitFor(() => expect(mine(w.cloud, w.team.id, 'u-Hemant')[0]).toMatchObject({ lastVerdict: 'approved', lastText: 'Looks good to me' }));
    await vi.waitFor(() => expect(m.frame()).toContain('approved'));
  });

  it('lets the author of a review follow it but not approve it', async () => {
    const w = await world(0);
    const m = await w.mount(120, 30);
    await m.type(`/review ${PR} @hemant Please check the retry logic`);
    await m.type(KEY.enter);
    await vi.waitFor(() => expect(m.frame()).toContain('to review acme/web#88'));
    await m.type('/threads');
    await m.type(KEY.enter);
    await vi.waitFor(() => expect(m.frame()).toContain('acme/web#88'));
    await m.type(KEY.enter);
    await vi.waitFor(() => expect(m.frame()).toContain(PR));
    expect(m.frame()).not.toContain('Request changes'); // the author does not pick a verdict
    expect(m.frame()).toContain('ctrl+x close');
  });

  it('asks a follow-up from the task view, and it reaches only the assigner', async () => {
    const w = await world(0);
    const hem = w.cloud.backendFor('u-Hemant');
    await hem.createTasks(w.team.id, [{ assigneeUid: 'u-boss', assigneeName: 'Boss', text: 'Build the form' }]);
    const m = await w.mount(120, 30);
    await m.type(KEY.tab);
    await vi.waitFor(() => expect(m.frame()).toContain('enter to open'));
    await m.type(KEY.enter);
    await vi.waitFor(() => expect(m.frame()).toContain('[f] Ask follow-up'));
    await m.type('f');
    await vi.waitFor(() => expect(m.frame()).toContain('Goes only to the other person on this task'));
    for (const ch of 'Which design?') await m.type(ch);
    await m.type(KEY.enter);
    await vi.waitFor(() => expect(m.frame()).toContain('Asked Hemant about T-1'));
    expect(mine(w.cloud, w.team.id, 'u-Hemant')).toHaveLength(1);
    expect(mine(w.cloud, w.team.id, 'u-Ravi')).toHaveLength(0);
    await vi.waitFor(() => expect(flat(m.frame())).toContain('[t] Follow-ups (1)'));
  });
});
