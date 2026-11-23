import { describe, expect, it } from 'vitest';
import { BackendError } from '../backend.js';
import type { Thread, ThreadMessage } from '../types.js';
import { first, noData, type Harness } from './contract.js';

const code = (p: Promise<unknown>) => p.then(() => 'ok', (e: unknown) => (e instanceof BackendError ? e.code : 'other'));
const PR = 'https://github.com/acme/web/pull/88';

/** Private follow-up questions and pull request reviews. Runs against every backend. */
export function runThreadContract(label: string, make: () => Promise<Harness> | Harness): void {
  describe(`thread contract: ${label}`, () => {
    async function setup() {
      const h = await make();
      const boss = await h.user();
      const team = await boss.createTeam('Acme', 'Boss');
      const join = async (name: string) => {
        const b = await h.user();
        await b.joinTeam((await boss.createInvite(team.id)).code, name);
        return b;
      };
      return { h, boss, team, join };
    }
    const person = (b: { uid: string }, name: string) => ({ uid: b.uid, name });

    it('keeps a follow-up private to the two people in it', async () => {
      const { h, boss, team, join } = await setup();
      const ann = await join('Ann');
      const bob = await join('Bob');
      const [task] = await boss.createTasks(team.id, [{ assigneeUid: ann.uid, assigneeName: 'Ann', text: 'build the form' }]);
      const t = await boss.createThread(team.id, {
        kind: 'followup', taskId: task!.id, title: 'Which design?', participants: [person(ann, 'Ann')], text: 'Which design should the form follow?',
      });
      expect(t).toMatchObject({ kind: 'followup', taskNum: 1, status: 'open', participants: [boss.uid, ann.uid], prUrl: '' });

      const annSees = await first<Thread[]>((cb) => ann.subscribeThreads(team.id, cb), (v) => v.length === 1);
      expect(annSees[0]).toMatchObject({ createdByName: 'Boss', lastText: 'Which design should the form follow?' });
      const msgs = await first<ThreadMessage[]>((cb) => ann.subscribeMessages(team.id, t.id, cb), (v) => v.length === 1);
      expect(msgs[0]).toMatchObject({ fromName: 'Boss', text: 'Which design should the form follow?', verdict: '' });

      // Bob is on the same team but not in the conversation: nothing arrives, and he cannot write to it.
      bob.onError = () => {};
      await noData((cb) => bob.subscribeThreads(team.id, cb));
      await noData((cb) => bob.subscribeMessages(team.id, t.id, cb));
      expect(await code(bob.postMessage(team.id, t.id, { text: 'hello?' }))).not.toBe('ok');
      expect(await code(bob.setThreadStatus(team.id, t.id, 'closed'))).not.toBe('ok');
      await h.done?.();
    });

    it('delivers replies in order and shows the latest on the thread', async () => {
      const { h, boss, team, join } = await setup();
      const ann = await join('Ann');
      const t = await boss.createThread(team.id, { kind: 'followup', title: 'Quick one', participants: [person(ann, 'Ann')], text: 'Are you blocked?' });
      await ann.postMessage(team.id, t.id, { text: 'Yes, waiting on the API.' });
      await boss.postMessage(team.id, t.id, { text: 'I will ping them.' });
      const msgs = await first<ThreadMessage[]>((cb) => boss.subscribeMessages(team.id, t.id, cb), (v) => v.length === 3);
      expect(msgs.map((m) => m.text)).toEqual(['Are you blocked?', 'Yes, waiting on the API.', 'I will ping them.']);
      const list = await first<Thread[]>((cb) => ann.subscribeThreads(team.id, cb), (v) => v[0]?.lastText === 'I will ping them.');
      expect(list[0]).toMatchObject({ lastFromName: 'Boss' });
      await h.done?.();
    });

    it('runs a review: reviewers give verdicts, the author cannot approve their own work', async () => {
      const { h, boss, team, join } = await setup();
      const ann = await join('Ann');
      const bob = await join('Bob');
      const t = await boss.createThread(team.id, {
        kind: 'review', title: 'acme/web#88', prUrl: PR, participants: [person(ann, 'Ann'), person(bob, 'Bob')],
        text: 'Please look at the retry logic in api/client.ts. I am unsure about the backoff.',
      });
      expect(t).toMatchObject({ kind: 'review', prUrl: PR, participants: [boss.uid, ann.uid, bob.uid] });

      await ann.postMessage(team.id, t.id, { text: 'Backoff looks right to me.', verdict: 'approved' });
      await bob.postMessage(team.id, t.id, { text: 'Needs a cap on retries.', verdict: 'changes' });
      expect(await code(boss.postMessage(team.id, t.id, { text: 'Approving my own PR', verdict: 'approved' }))).not.toBe('ok');
      await boss.postMessage(team.id, t.id, { text: 'Added a cap.' }); // a plain reply is fine

      const msgs = await first<ThreadMessage[]>((cb) => ann.subscribeMessages(team.id, t.id, cb), (v) => v.length === 4);
      expect(msgs.map((m) => m.verdict)).toEqual(['', 'approved', 'changes', '']);
      // Reviewers can see each other's comments in the same thread.
      expect(msgs.map((m) => m.fromName)).toEqual(['Boss', 'Ann', 'Bob', 'Boss']);
      await h.done?.();
    });

    it('only the author closes a thread, and a closed thread takes no new messages', async () => {
      const { h, boss, team, join } = await setup();
      const ann = await join('Ann');
      const t = await boss.createThread(team.id, { kind: 'review', title: 'acme/web#9', prUrl: 'https://github.com/acme/web/pull/9', participants: [person(ann, 'Ann')], text: 'Please review.' });
      expect(await code(ann.setThreadStatus(team.id, t.id, 'closed'))).not.toBe('ok');
      await boss.setThreadStatus(team.id, t.id, 'closed');
      expect(await code(ann.postMessage(team.id, t.id, { text: 'late comment' }))).not.toBe('ok');
      await first<Thread[]>((cb) => ann.subscribeThreads(team.id, cb), (v) => v[0]?.status === 'closed');
      await boss.setThreadStatus(team.id, t.id, 'open');
      await ann.postMessage(team.id, t.id, { text: 'now it works' });
      await h.done?.();
    });

    it('rejects bad links, empty text, strangers and too many people', async () => {
      const { h, boss, team, join } = await setup();
      const ann = await join('Ann');
      const stranger = await h.user(); // signed in, but not in this team
      const base = { kind: 'review' as const, title: 'x', prUrl: PR, participants: [person(ann, 'Ann')], text: 'hi' };
      for (const bad of ['http://github.com/acme/web/pull/1', 'javascript:alert(1)', 'not a link', '', 'https://user:pw@github.com/a/b/pull/1']) {
        expect(await code(boss.createThread(team.id, { ...base, prUrl: bad }))).toBe('invalid');
      }
      expect(await code(boss.createThread(team.id, { ...base, text: '   ' }))).toBe('invalid');
      expect(await code(boss.createThread(team.id, { ...base, title: '' }))).toBe('invalid');
      expect(await code(boss.createThread(team.id, { ...base, participants: [] }))).toBe('invalid');
      expect(await code(boss.createThread(team.id, { ...base, participants: [person(boss, 'Boss')] }))).toBe('invalid');
      expect(await code(boss.createThread(team.id, { ...base, participants: [person(stranger, 'Zed')] }))).toBe('invalid');
      expect(await code(boss.createThread(team.id, { kind: 'followup', title: 'x', prUrl: PR, participants: [person(ann, 'Ann')], text: 'hi' }))).toBe('invalid');
      const crowd = Array.from({ length: 25 }, (_, i) => ({ uid: `ghost-${i}`, name: `G${i}` }));
      expect(await code(boss.createThread(team.id, { ...base, participants: crowd }))).toBe('invalid');
      expect(await code(boss.createThread(team.id, { kind: 'followup', taskId: 'no-such-task', title: 'x', participants: [person(ann, 'Ann')], text: 'hi' }))).toBe('not-found');
      // A reply cannot carry a verdict in a follow-up, and the text is bounded.
      const f = await boss.createThread(team.id, { kind: 'followup', title: 'q', participants: [person(ann, 'Ann')], text: 'q?' });
      expect(await code(ann.postMessage(team.id, f.id, { text: 'ok', verdict: 'approved' }))).toBe('invalid');
      expect(await code(ann.postMessage(team.id, f.id, { text: 'x'.repeat(5000) }))).toBe('invalid');
      expect(await code(ann.postMessage(team.id, f.id, { text: '   ' }))).toBe('invalid');
      await h.done?.();
    });

    it('does not let someone outside the team start a conversation', async () => {
      const { h, boss, team, join } = await setup();
      const ann = await join('Ann');
      const stranger = await h.user();
      expect(await code(stranger.createThread(team.id, { kind: 'followup', title: 'hi', participants: [person(ann, 'Ann')], text: 'hello' }))).not.toBe('ok');
      void boss;
      await h.done?.();
    });
  });
}
