import type { Backend } from './backend.js';
import { BackendError } from './backend.js';
import { resolveMention } from './commands.js';
import type { EntryKind } from './config.js';
import { clip } from './format.js';
import { parsePrUrl } from './pr.js';
import type { Member, Task, Thread } from './types.js';

export interface ThreadEntry {
  at: number;
  kind: EntryKind;
  who: string;
  text: string;
  key: string;
}

/** What a thread is about, in a few words: `T-4` for a task follow-up, `owner/repo#12` for a review. */
export function threadLabel(t: Thread): string {
  if (t.kind === 'review') {
    const pr = parsePrUrl(t.prUrl);
    return pr.ok ? pr.label : t.title;
  }
  return t.taskNum ? `T-${t.taskNum}` : clip(t.title, 40);
}

/**
 * Messages for the chat feed about conversations I am in: a new question or review request from someone else,
 * and replies, approvals or change requests since this chat began. Only people in a thread ever receive it.
 */
export function threadEntries(threads: Thread[], meUid: string, since: number): ThreadEntry[] {
  const out: ThreadEntry[] = [];
  for (const t of threads) {
    if (t.status !== 'open' || !t.participants.includes(meUid)) continue;
    const label = threadLabel(t);
    const fresh = t.updatedAt === t.createdAt;
    if (t.createdByUid !== meUid && t.createdAt >= since) {
      const about = t.kind === 'review' ? 'asked you to review ' + label : 'asked you' + (t.taskNum ? ' about ' + label : '');
      const quote = fresh ? ': ' + clip(t.lastText, 160) : '';
      out.push({ at: t.createdAt, kind: 'thread', who: '', text: t.createdByName + ' ' + about + quote, key: 'thn:' + t.id });
    }
    if (!fresh && t.lastFromUid !== meUid && t.updatedAt >= since) {
      let text: string;
      if (t.lastVerdict === 'approved') text = t.lastFromName + ' approved ' + label + (t.lastText ? ': ' + clip(t.lastText, 140) : '');
      else if (t.lastVerdict === 'changes') text = t.lastFromName + ' asked for changes on ' + label + ': ' + clip(t.lastText, 140);
      else text = t.lastFromName + ' replied on ' + label + ': ' + clip(t.lastText, 160);
      out.push({ at: t.updatedAt, kind: 'thread', who: '', text, key: 'thr:' + t.id + ':' + t.updatedAt });
    }
  }
  return out;
}

/** Threads with something I have not looked at: new, or a reply from someone else after I last opened it. */
export function unreadThreads(threads: Thread[], meUid: string, seen: Record<string, number>): Thread[] {
  return threads.filter((t) => t.status === 'open' && t.lastFromUid !== meUid && t.updatedAt > (seen[t.id] ?? 0));
}

export interface ThreadCtx {
  backend: Backend;
  teamId: string;
  me: Member;
  members: Member[];
  tasks: Task[];
}

const fail = (message: string): never => {
  throw new BackendError('invalid', message);
};

function resolveAll(names: string[], ctx: ThreadCtx): Member[] {
  const out: Member[] = [];
  for (const n of names) {
    const lower = n.toLowerCase();
    if (lower === 'team' || lower === 'all') {
      for (const m of ctx.members) if (m.uid !== ctx.me.uid) out.push(m);
      continue;
    }
    const r = resolveMention(n, ctx.members);
    if ('error' in r) return fail(r.error);
    if (r.member.uid === ctx.me.uid) return fail("That's you. Pick someone else.");
    out.push(r.member);
  }
  return [...new Map(out.map((m) => [m.uid, m])).values()];
}

/** Ask a follow-up question about a task. With no one named it goes to the other person on the task. */
export async function startFollowup(ctx: ThreadCtx, p: { taskNum: number; recipients: string[]; text: string }): Promise<string> {
  const task = ctx.tasks.find((t) => t.num === p.taskNum);
  if (!task) return fail('There is no task T-' + p.taskNum + '.');
  let people = resolveAll(p.recipients, ctx);
  if (people.length === 0) {
    const otherUid = task.assigneeUid === ctx.me.uid ? task.assignerUid : task.assigneeUid;
    const other = ctx.members.find((m) => m.uid === otherUid && m.uid !== ctx.me.uid);
    if (!other) return fail('Say who to ask: /followup T-' + p.taskNum + ' @name your question.');
    people = [other];
  }
  await ctx.backend.createThread(ctx.teamId, {
    kind: 'followup',
    taskId: task.id,
    title: clip(p.text, 80),
    participants: people.map((m) => ({ uid: m.uid, name: m.name })),
    text: p.text,
  });
  return 'Asked ' + people.map((m) => m.name).join(', ') + ' about T-' + task.num + '. Only you and ' + (people.length > 1 ? 'they' : people[0]!.name) + ' can see it.';
}

/** Share a pull request link with your comments so the people you pick can review it. */
export async function startReview(ctx: ThreadCtx, p: { url: string; reviewers: string[]; text: string }): Promise<string> {
  const pr = parsePrUrl(p.url);
  if (!pr.ok) return fail(pr.error);
  const people = resolveAll(p.reviewers, ctx);
  if (people.length === 0) return fail('Say who should review it: /review <link> @name your comments');
  const text = p.text.trim() || 'Please review this pull request.';
  await ctx.backend.createThread(ctx.teamId, {
    kind: 'review',
    title: pr.label,
    prUrl: pr.url,
    participants: people.map((m) => ({ uid: m.uid, name: m.name })),
    text,
  });
  return 'Asked ' + people.map((m) => m.name).join(', ') + ' to review ' + pr.label + '. Only you and the reviewers can see it.';
}
