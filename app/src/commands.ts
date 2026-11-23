import { LIMITS, type Member } from './types.js';

export interface CommandInfo {
  name: string;
  usage: string;
  desc: string;
}

export const COMMANDS: CommandInfo[] = [
  { name: 'read', usage: '/read [@name] question', desc: 'ask a project (add @name for a teammate)' },
  { name: 'task', usage: '/task @name… what to do', desc: 'assign work to one or more people' },
  { name: 'broadcast', usage: '/broadcast message', desc: 'message the whole team' },
  { name: 'tasks', usage: '/tasks', desc: 'open the task board' },
  { name: 'followup', usage: '/followup T-4 [@name] question', desc: 'ask about a task, privately' },
  { name: 'review', usage: '/review <link> @name comments', desc: 'share a pull request for review' },
  { name: 'threads', usage: '/threads', desc: 'your private conversations' },
  { name: 'team', usage: '/team [invite|leave]', desc: 'people, invites, leaving' },
  { name: 'agent', usage: '/agent [codex|claude|auto]', desc: 'switch your coding agent' },
  { name: 'new', usage: '/new', desc: 'start a new chat' },
  { name: 'help', usage: '/help', desc: 'show commands' },
  { name: 'quit', usage: '/quit', desc: 'leave crewai' },
];

export type Parsed =
  | { kind: 'read'; question: string; target?: string }
  | { kind: 'task'; assignees: string[]; text: string }
  | { kind: 'broadcast'; text: string }
  | { kind: 'tasks' }
  | { kind: 'followup'; taskNum: number; recipients: string[]; text: string }
  | { kind: 'review'; url: string; reviewers: string[]; text: string }
  | { kind: 'threads' }
  | { kind: 'team'; sub?: string; target?: string }
  | { kind: 'agent'; name?: string }
  | { kind: 'new' | 'help' | 'quit' }
  | { kind: 'ask'; text: string }
  | { kind: 'empty' }
  | { kind: 'error'; message: string };

const MENTION = /@([\w.-]+)/g;
/** A mention starts a word, so "ops@example.com" is not one. */
const WORD_MENTION = /(^|\s)@([\w.-]+)/g;

export function parseInput(raw: string): Parsed {
  const input = raw.trim();
  if (!input) return { kind: 'empty' };
  if (input.length > LIMITS.text) {
    return { kind: 'error', message: `That's too long (max ${LIMITS.text} characters).` };
  }
  if (!input.startsWith('/')) return { kind: 'ask', text: input };

  const m = /^\/(\S*)\s*([\s\S]*)$/.exec(input)!;
  const name = (m[1] ?? '').toLowerCase();
  const rest = (m[2] ?? '').trim();

  switch (name) {
    case 'read': {
      const mentions = [...rest.matchAll(WORD_MENTION)].map((x) => x[2]!);
      if (mentions.length > 1) {
        return { kind: 'error', message: 'Ask one teammate at a time, for example /read @hemant question.' };
      }
      const question = rest.replace(WORD_MENTION, '$1').replace(/\s+/g, ' ').trim();
      if (!question) return { kind: 'error', message: 'Add a question after /read.' };
      return { kind: 'read', question, target: mentions[0] };
    }
    case 'task': {
      const lead = /^((?:@[\w.-]+(?:\s+|$))+)([\s\S]*)$/.exec(rest);
      if (!lead) return { kind: 'error', message: 'Start with who it is for: /task @name what to do.' };
      const assignees = [...new Set([...lead[1]!.matchAll(MENTION)].map((x) => x[1]!.toLowerCase()))];
      const text = lead[2]!.trim();
      if (!text) return { kind: 'error', message: 'Add what needs doing after the names: /task @name what to do.' };
      if (assignees.length > LIMITS.assignees) {
        return { kind: 'error', message: `Assign at most ${LIMITS.assignees} people at once.` };
      }
      return { kind: 'task', assignees, text };
    }
    case 'broadcast':
      if (!rest) return { kind: 'error', message: 'Add a message after /broadcast.' };
      return { kind: 'broadcast', text: rest };
    case 'tasks':
      return { kind: 'tasks' };
    case 'threads':
    case 'inbox':
      return { kind: 'threads' };
    case 'followup': {
      const m = /^#?(?:T-?)?(\d+)\s+([\s\S]*)$/i.exec(rest);
      if (!m) return { kind: 'error', message: 'Say which task: /followup T-4 @name your question.' };
      const body = m[2]!.trim();
      const lead = /^((?:@[\w.-]+\s+)+)([\s\S]*)$/.exec(body);
      const recipients = lead ? [...lead[1]!.matchAll(MENTION)].map((x) => x[1]!.toLowerCase()) : [];
      const text = (lead ? lead[2]! : body).trim();
      if (!text) return { kind: 'error', message: 'Add your question after the task number.' };
      return { kind: 'followup', taskNum: Number(m[1]), recipients: [...new Set(recipients)], text };
    }
    case 'review': {
      const m = /^(\S+)\s*([\s\S]*)$/.exec(rest);
      if (!m) return { kind: 'error', message: 'Paste the pull request link: /review https://... @name your comments' };
      const body = m[2]!.trim();
      const lead = /^((?:@[\w.-]+\s*)+)([\s\S]*)$/.exec(body);
      const reviewers = lead ? [...lead[1]!.matchAll(MENTION)].map((x) => x[1]!.toLowerCase()) : [];
      if (reviewers.length === 0) return { kind: 'error', message: 'Say who should review it: /review <link> @name your comments (or @team).' };
      return { kind: 'review', url: m[1]!, reviewers: [...new Set(reviewers)], text: (lead ? lead[2]! : '').trim() };
    }
    case 'team': {
      const [sub, ...more] = rest.split(/\s+/).filter(Boolean);
      const target = more.join(' ').replace(/^@/, '') || undefined;
      return { kind: 'team', sub: sub?.toLowerCase(), target };
    }
    case 'agent':
      return { kind: 'agent', name: rest.toLowerCase() || undefined };
    case 'new':
    case 'help':
      return { kind: name };
    case 'quit':
    case 'exit':
      return { kind: 'quit' };
    default:
      return { kind: 'error', message: `Unknown command /${name}. Type / to see commands.` };
  }
}

export type Resolved = { member: Member } | { error: string };

/** Case-insensitive exact match first, then a unique prefix. */
export function resolveMention(name: string, members: Member[]): Resolved {
  const key = (s: string) => mentionName(s).toLowerCase();
  const q = key(name);
  const exact = members.filter((x) => key(x.name) === q);
  if (exact.length === 1) return { member: exact[0]! };
  if (exact.length > 1) return { error: `More than one person is called ${name}.` };
  const prefix = members.filter((x) => key(x.name).startsWith(q));
  if (prefix.length === 1) return { member: prefix[0]! };
  if (prefix.length > 1) return { error: `${name} could be ${prefix.map((p) => p.name).join(' or ')}.` };
  return { error: `Nobody called ${name} is in this team.` };
}

/** Current @-token being typed at the end of the composer, if any. */
export function activeMention(text: string): { query: string; start: number } | null {
  const m = /(^|\s)@([\w.-]*)$/.exec(text);
  if (!m) return null;
  return { query: m[2]!, start: text.length - m[2]!.length - 1 };
}

export function mentionName(name: string): string {
  return name.replace(/\s+/g, '_');
}
