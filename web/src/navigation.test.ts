import { describe, expect, it } from 'vitest';
import { defaultNavigation, parseNavigation } from './navigation';

describe('public navigation manifest', () => {
  it('reads centrally changed labels, order and destinations', () => {
    const items = [{ id: 'notes', label: '변경한 이름', href: 'https://rag.cola16.app/new' }, defaultNavigation[0]];
    expect(parseNavigation({ version: 1, items })).toEqual(items);
  });
  it('rejects unsafe links, duplicate IDs and unsupported schemas', () => {
    for (const href of ['javascript:alert(1)', 'data:text/html,test', '/local', 'https://user:pass@cola16.app/', 'http://cola16.app/']) {
      expect(parseNavigation({ version: 1, items: [{ id: 'test', label: '기록', href }] })).toBeUndefined();
    }
    expect(parseNavigation({ version: 1, items: [defaultNavigation[0], defaultNavigation[0]] })).toBeUndefined();
    expect(parseNavigation({ version: 2, items: defaultNavigation })).toBeUndefined();
    expect(parseNavigation({ version: 1, items: [] })).toBeUndefined();
  });
});
