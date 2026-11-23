import { describe, expect, it } from 'vitest';
import { activeMention, parseInput, resolveMention } from './commands.js';
import type { Member } from './types.js';

const m = (uid: string, name: string): Member => ({
  uid, name, role: 'member', lastSeen: 0, agent: null, project: '', joinedAt: 0,
});

describe('parseInput', () => {
  it('treats plain text as a local question', () => {
    expect(parseInput('what does this do?')).toEqual({ kind: 'ask', text: 'what does this do?' });
  });
  it('parses /read with the mention at the start or the end', () => {
    expect(parseInput('/read @hemant what is this project about')).toEqual({
      kind: 'read', question: 'what is this project about', target: 'hemant',
    });
    expect(parseInput('/read what is this project about @hemant')).toMatchObject({ target: 'hemant' });
  });
  it('parses /read without a mention as local', () => {
    expect(parseInput('/read how does login work')).toEqual({ kind: 'read', question: 'how does login work', target: undefined });
  });
  it('parses /task with several assignees and keeps the text', () => {
    expect(parseInput('/task @hemant @Shwetank implement submit button')).toEqual({
      kind: 'task', assignees: ['hemant', 'shwetank'], text: 'implement submit button',
    });
  });
  it('keeps urls and email-like text inside the body', () => {
    expect(parseInput('/broadcast see https://x.io/a@b now')).toEqual({ kind: 'broadcast', text: 'see https://x.io/a@b now' });
  });
  it('rejects malformed input without dispatching', () => {
    expect(parseInput('/read')).toMatchObject({ kind: 'error' });
    expect(parseInput('/read @a @b question')).toMatchObject({ kind: 'error' });
    expect(parseInput('/task fix it')).toMatchObject({ kind: 'error' });
    expect(parseInput('/task @a')).toMatchObject({ kind: 'error' });
    expect(parseInput('/broadcast')).toMatchObject({ kind: 'error' });
    expect(parseInput('/nope')).toMatchObject({ kind: 'error' });
    expect(parseInput('x'.repeat(5000))).toMatchObject({ kind: 'error' });
    expect(parseInput('   ')).toEqual({ kind: 'empty' });
  });
  it('dedupes repeated assignees', () => {
    expect(parseInput('/task @a @A do it')).toMatchObject({ assignees: ['a'] });
  });
});

describe('resolveMention', () => {
  const people = [m('1', 'Hemant'), m('2', 'Shwetank'), m('3', 'Shweta')];
  it('prefers an exact, case-insensitive match', () => {
    expect(resolveMention('shweta', people)).toMatchObject({ member: { uid: '3' } });
  });
  it('accepts a unique prefix', () => {
    expect(resolveMention('hem', people)).toMatchObject({ member: { uid: '1' } });
  });
  it('refuses ambiguous and unknown names instead of guessing', () => {
    expect(resolveMention('shw', people)).toHaveProperty('error');
    expect(resolveMention('zed', people)).toHaveProperty('error');
  });
});

describe('activeMention', () => {
  it('finds the token being typed', () => {
    expect(activeMention('/task @he')).toEqual({ query: 'he', start: 6 });
    expect(activeMention('/task @he done')).toBeNull();
    expect(activeMention('mail a@b')).toBeNull();
  });
});

describe('review fixes: parsing', () => {
  it('does not treat an email address in a question as a mention', () => {
    expect(parseInput('/read what sends mail to ops@example.com')).toEqual({
      kind: 'read', question: 'what sends mail to ops@example.com', target: undefined,
    });
    expect(parseInput('/read @bob who owns ops@example.com')).toMatchObject({ target: 'bob', question: 'who owns ops@example.com' });
  });
  it('says what is missing when /task has names but no text', () => {
    expect(parseInput('/task @bob')).toMatchObject({ kind: 'error', message: expect.stringContaining('what needs doing') });
  });
  it('limits how many people one /task can assign', () => {
    const many = Array.from({ length: 9 }, (_, i) => `@p${i}`).join(' ');
    expect(parseInput(`/task ${many} do it`)).toMatchObject({ kind: 'error' });
  });
  it('parses team management commands', () => {
    expect(parseInput('/team remove @Bob')).toEqual({ kind: 'team', sub: 'remove', target: 'Bob' });
    expect(parseInput('/team leave')).toEqual({ kind: 'team', sub: 'leave', target: undefined });
    expect(parseInput('/team')).toEqual({ kind: 'team', sub: undefined, target: undefined });
  });
});
