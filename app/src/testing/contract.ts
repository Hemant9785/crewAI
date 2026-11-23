import { describe, expect, it } from 'vitest';
import { BackendError, type Backend, type Unsubscribe } from '../backend.js';
import type { Broadcast, ReadRequest, Task } from '../types.js';

export interface Harness {
  /** A new, signed-in user on the shared service. */
  user(): Promise<Backend>;
  /** Only available when time can be controlled (the in-memory service). */
  advance?: (ms: number) => void;
  done?: () => Promise<void>;
}

const code = (p: Promise<unknown>) => p.then(() => 'ok', (e: unknown) => (e instanceof BackendError ? e.code : 'other'));

/** First value that satisfies `ok`. */
export function first<T>(sub: (cb: (v: T) => void) => Unsubscribe, ok: (v: T) => boolean = () => true, ms = 8000): Promise<T> {
  return new Promise((resolve, reject) => {
    let un: Unsubscribe = () => {};
    let finished = false;
    const timer = setTimeout(() => {
      finished = true;
      un();
      reject(new Error('timed out waiting for data'));
    }, ms);
    un = sub((v) => {
      if (finished || !ok(v)) return;
      finished = true;
      clearTimeout(timer);
      queueMicrotask(() => un());
      resolve(v);
    });
  });
}

/** Asserts nothing readable arrives (empty lists or errors both count as "no data"). */
export async function noData<T>(sub: (cb: (v: T[]) => void) => Unsubscribe, ms = 500): Promise<void> {
  let got = 0;
  const un = sub((v) => {
    got += v.length;
  });
  await new Promise((r) => setTimeout(r, ms));
  un();
  expect(got).toBe(0);
}

export function runContract(label: string, make: () => Promise<Harness> | Harness): void {
  describe(`backend contract: ${label}`, () => {
    async function setup() {
      const h = await make();
      const boss = await h.user();
      const team = await boss.createTeam('Acme', 'Boss');
      const join = async (name: string) => {
        const b = await h.user();
        const inv = await boss.createInvite(team.id);
        await b.joinTeam(inv.code, name);
        return b;
      };
      return { h, boss, team, join };
    }

    it('lets a member join once with a valid code', async () => {
      const { h, boss, team } = await setup();
      const inv = await boss.createInvite(team.id);
      expect(inv.code).toMatch(/^\d{6}$/);
      await (await h.user()).joinTeam(inv.code, 'Ann');
      expect(await code((await h.user()).joinTeam(inv.code, 'Bob'))).toBe('conflict');
      await h.done?.();
    });

    it('rejects unknown codes, taken names and bad names', async () => {
      const { h, boss, team } = await setup();
      expect(await code((await h.user()).joinTeam('000000', 'Ann'))).toBe('not-found');
      const inv = await boss.createInvite(team.id);
      expect(await code((await h.user()).joinTeam(inv.code, 'boss'))).toBe('conflict');
      expect(await code((await h.user()).joinTeam(inv.code, 'has space'))).toBe('invalid');
      await h.done?.();
    });

    it('rejects an expired code', async () => {
      const { h, boss, team } = await setup();
      if (!h.advance) return;
      const inv = await boss.createInvite(team.id);
      h.advance(11 * 60_000);
      expect(await code((await h.user()).joinTeam(inv.code, 'Ann'))).toBe('expired');
    });

    it('only managers create invites', async () => {
      const { team, join, h } = await setup();
      const ann = await join('Ann');
      expect(await code(ann.createInvite(team.id))).toBe('permission-denied');
      await h.done?.();
    });

    it('blocks outsiders from writing and from reading team data', async () => {
      const { team, h } = await setup();
      const out = await h.user();
      expect(await code(out.sendBroadcast(team.id, 'hi'))).toBe('permission-denied');
      expect(await code(out.createTasks(team.id, [{ assigneeUid: out.uid, assigneeName: 'X', text: 'x' }]))).not.toBe('ok');
      out.onError = () => {};
      await noData((cb) => out.subscribeBroadcasts(team.id, cb));
      await noData((cb) => out.subscribeMembers(team.id, cb));
      await h.done?.();
    });

    it('numbers tasks and lets only involved people update them', async () => {
      const { team, join, boss, h } = await setup();
      const ann = await join('Ann');
      const bob = await join('Bob');
      const [t1, t2] = await boss.createTasks(team.id, [
        { assigneeUid: ann.uid, assigneeName: 'Ann', text: 'one' },
        { assigneeUid: bob.uid, assigneeName: 'Bob', text: 'two' },
      ]);
      expect([t1!.num, t2!.num]).toEqual([1, 2]);
      await ann.updateTask(team.id, t1!.id, { status: 'done', note: 'shipped' });
      expect(await code(bob.updateTask(team.id, t1!.id, { status: 'done' }))).toBe('permission-denied');
      await boss.updateTask(team.id, t2!.id, { status: 'cancelled' });
      const seen = await first<Task[]>((cb) => bob.subscribeTasks(team.id, cb), (v) => v.length === 2 && v.some((t) => t.status === 'done'));
      expect(seen.find((t) => t.num === 1)).toMatchObject({ status: 'done', note: 'shipped', assigneeName: 'Ann' });
      await h.done?.();
    });

    it('rejects empty and oversized text, and assigning to a stranger', async () => {
      const { team, boss, h } = await setup();
      expect(await code(boss.sendBroadcast(team.id, '   '))).toBe('invalid');
      expect(await code(boss.sendBroadcast(team.id, 'x'.repeat(5000)))).toBe('invalid');
      const stranger = await h.user();
      expect(await code(boss.createTasks(team.id, [{ assigneeUid: stranger.uid, assigneeName: 'Zed', text: 'x' }]))).not.toBe('ok');
      await h.done?.();
    });

    it('delivers broadcasts to members', async () => {
      const { team, join, boss, h } = await setup();
      const ann = await join('Ann');
      await boss.sendBroadcast(team.id, 'release freeze');
      const seen = await first<Broadcast[]>((cb) => ann.subscribeBroadcasts(team.id, cb), (v) => v.length === 1);
      expect(seen[0]).toMatchObject({ text: 'release freeze', fromName: 'Boss' });
      await h.done?.();
    });

    it('lets only the target answer, and bounds the answer', async () => {
      const { team, join, boss, h } = await setup();
      const ann = await join('Ann');
      const bob = await join('Bob');
      const r = await boss.createRequest(team.id, { targetUid: ann.uid, targetName: 'Ann', question: 'q' });
      expect(await code(bob.respondRequest(team.id, r.id, { status: 'answered', answer: 'x' }))).not.toBe('ok');
      await ann.respondRequest(team.id, r.id, { status: 'running' });
      await ann.respondRequest(team.id, r.id, { status: 'answered', answer: 'y'.repeat(20000), agent: 'codex' });
      const seen = await first<ReadRequest[]>((cb) => boss.subscribeRequests(team.id, cb), (v) => v[0]?.status === 'answered');
      expect(seen[0]!.answer.length).toBeLessThanOrEqual(8000);
      expect(seen[0]!.agent).toBe('codex');
      await h.done?.();
    });

    it('moves a question to one new owner and blocks the old target', async () => {
      const { team, join, boss, h } = await setup();
      const ann = await join('Ann');
      const bob = await join('Bob');
      const r = await boss.createRequest(team.id, { targetUid: ann.uid, targetName: 'Ann', question: 'q' });
      await boss.rerouteRequest(team.id, r.id, { uid: bob.uid, name: 'Bob' });
      expect(await code(ann.respondRequest(team.id, r.id, { status: 'answered', answer: 'late' }))).not.toBe('ok');
      const annSees = await first<ReadRequest[]>((cb) => ann.subscribeRequests(team.id, cb), (v) => v.some((x) => x.targetUid === bob.uid));
      expect(annSees[0]).toMatchObject({ targetUid: bob.uid, movedFrom: [ann.uid] });
      await bob.respondRequest(team.id, r.id, { status: 'answered', answer: 'ok' });
      await h.done?.();
    });

    it('cannot move a running question, and a cancelled one stays cancelled', async () => {
      const { team, join, boss, h } = await setup();
      const ann = await join('Ann');
      const bob = await join('Bob');
      const r = await boss.createRequest(team.id, { targetUid: ann.uid, targetName: 'Ann', question: 'q' });
      await ann.respondRequest(team.id, r.id, { status: 'running' });
      expect(await code(boss.rerouteRequest(team.id, r.id, { uid: bob.uid, name: 'Bob' }))).toBe('conflict');
      await boss.cancelRequest(team.id, r.id);
      expect(await code(ann.respondRequest(team.id, r.id, { status: 'answered', answer: 'x' }))).not.toBe('ok');
      await h.done?.();
    });

    it('hides a question from uninvolved teammates', async () => {
      const { team, join, boss, h } = await setup();
      const ann = await join('Ann');
      const bob = await join('Bob');
      await boss.createRequest(team.id, { targetUid: ann.uid, targetName: 'Ann', question: 'secret question' });
      await noData((cb) => bob.subscribeRequests(team.id, cb));
      await h.done?.();
    });
  });
}
