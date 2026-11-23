import { describe, expect, it } from 'vitest';
import { parsePrUrl } from './pr.js';

describe('parsePrUrl', () => {
  it('names a GitHub pull request', () => {
    expect(parsePrUrl('https://github.com/acme/web/pull/88')).toMatchObject({ ok: true, label: 'acme/web#88', host: 'github.com' });
    expect(parsePrUrl('  https://github.com/acme/web/pull/88/files#diff  ')).toMatchObject({ ok: true, label: 'acme/web#88' });
  });
  it('names GitLab and Bitbucket requests', () => {
    expect(parsePrUrl('https://gitlab.com/group/sub/app/-/merge_requests/7')).toMatchObject({ ok: true, label: 'group/sub/app!7' });
    expect(parsePrUrl('https://bitbucket.org/acme/web/pull-requests/12')).toMatchObject({ ok: true, label: 'acme/web#12' });
  });
  it('accepts other https links with a short generic name', () => {
    expect(parsePrUrl('https://git.example.com/review/42')).toMatchObject({ ok: true, label: 'git.example.com/review/42' });
  });
  it('rejects anything that is not a plain https link', () => {
    for (const bad of [
      '', '   ', 'github.com/acme/web/pull/1', 'http://github.com/acme/web/pull/1', 'javascript:alert(1)',
      'file:///etc/passwd', 'ftp://github.com/x', 'https://user:pw@github.com/acme/web/pull/1', 'https://github.com/a b/pull/1',
      'https://localhost/x', 'not a link', `https://github.com/${'a'.repeat(600)}`,
    ]) {
      expect(parsePrUrl(bad)).toMatchObject({ ok: false });
    }
  });
});
