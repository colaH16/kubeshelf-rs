import { describe, expect, it } from 'vitest';
import source from './cola-header.js?raw';
import manifest from './cola16-navigation.json';

type Item = { id: string; label: string; href: string };
type Header = {
  parseNavigation(value: unknown): unknown;
  nextTheme(value: unknown): string;
  focusIndex(previous: { id?: string; href: string | null; index: number }, items: Item[]): number;
  skipTo(target: unknown, header: unknown): void;
};
type Page = { styles: Map<unknown, Record<string, string>>; scrolled: number[]; scrollY: number };

// Run the served classic-script source and read the helpers from the defined element.
// Browser globals the helpers read come from a small fake page.
function load(page: Page): Header {
  let header: unknown;
  const registry = { get: () => undefined, define: (_: string, element: unknown) => { header = element; } };
  const documentElement = { dataset: {} };
  const document = { currentScript: { src: 'https://apps.cola16.app/branding/v1/cola-header.js' }, documentElement };
  page.styles.set(documentElement, { scrollPaddingTop: 'auto' });
  new Function('customElements', 'HTMLElement', 'document', 'matchMedia', 'getComputedStyle', 'scrollTo', 'scrollY', source)(
    registry, class {}, document, () => ({ matches: false }),
    (element: unknown) => page.styles.get(element) ?? {}, ({ top }: { top: number }) => page.scrolled.push(top), page.scrollY);
  return header as Header;
}
const header = load({ styles: new Map(), scrolled: [], scrollY: 0 });

describe('shared header manifest', () => {
  it('accepts the canonical manifest and centrally changed labels, order and destinations', () => {
    expect(header.parseNavigation(manifest)).toEqual(manifest.items);
    const items = [{ id: 'notes', label: '변경한 이름', href: 'https://rag.cola16.app/new' }, manifest.items[0]];
    expect(header.parseNavigation({ version: 1, items })).toEqual(items);
  });
  it('rejects unsafe links, duplicate IDs and unsupported schemas', () => {
    for (const href of ['javascript:alert(1)', 'data:text/html,test', '/local', 'https://user:pass@cola16.app/', 'http://cola16.app/']) {
      expect(header.parseNavigation({ version: 1, items: [{ id: 'test', label: '기록', href }] })).toBeUndefined();
    }
    expect(header.parseNavigation({ version: 1, items: [manifest.items[0], manifest.items[0]] })).toBeUndefined();
    expect(header.parseNavigation({ version: 2, items: manifest.items })).toBeUndefined();
    expect(header.parseNavigation({ version: 1, items: [] })).toBeUndefined();
  });
});

describe('shared header theme button', () => {
  it('cycles auto, light, dark and back; unknown stored values restart from auto', () => {
    expect(['auto', 'light', 'dark'].map(header.nextTheme)).toEqual(['light', 'dark', 'auto']);
    expect(header.nextTheme('sepia')).toBe('light');
    expect(header.nextTheme(undefined)).toBe('light');
  });
});

describe('shared header link focus', () => {
  const [blog, portfolio, tools] = manifest.items;
  it('follows the focused destination when the manifest reorders or adds links', () => {
    expect(header.focusIndex({ id: 'tools', href: tools.href, index: 2 }, [tools, blog, portfolio])).toBe(0);
    expect(header.focusIndex({ id: 'blog', href: blog.href, index: 0 }, [{ id: 'new', label: '새 공간', href: 'https://new.cola16.app/' }, blog])).toBe(1);
  });
  it('matches a renamed id by address, and otherwise moves to the link now in its place', () => {
    expect(header.focusIndex({ id: 'old', href: portfolio.href, index: 0 }, [blog, portfolio])).toBe(1);
    expect(header.focusIndex({ id: 'portfolio', href: portfolio.href, index: 1 }, [blog, tools])).toBe(1);
    expect(header.focusIndex({ id: 'tools', href: tools.href, index: 2 }, [blog, portfolio])).toBe(1);
  });
});

describe('shared header skip link', () => {
  function main(tabindex: string | null, tabIndex: number, top: number) {
    const attributes = new Map(tabindex === null ? [] : [['tabindex', tabindex]]);
    const calls: unknown[] = [];
    let blur = () => {};
    return {
      calls, attributes, blur: () => blur(), tabIndex,
      hasAttribute: (name: string) => attributes.has(name),
      setAttribute: (name: string, value: string) => attributes.set(name, value),
      removeAttribute: (name: string) => attributes.delete(name),
      addEventListener: (type: string, listener: () => void) => { if (type === 'blur') blur = listener; },
      focus: (options: unknown) => calls.push(options),
      getBoundingClientRect: () => ({ top }),
    };
  }
  it('focuses a plain main temporarily, then restores its markup on blur', () => {
    const page: Page = { styles: new Map(), scrolled: [], scrollY: 500 };
    const host = { offsetHeight: 77 };
    page.styles.set(host, { position: 'static', top: 'auto' });
    const target = main(null, -1, 300);
    load(page).skipTo(target, host);
    expect(target.calls).toEqual([{ preventScroll: true }]);
    expect(target.attributes.get('tabindex')).toBe('-1');
    expect(page.scrolled).toEqual([800]);
    target.blur();
    expect(target.hasAttribute('tabindex')).toBe(false);
  });
  it('keeps an existing tabindex and scrolls the target below a sticky header', () => {
    const page: Page = { styles: new Map(), scrolled: [], scrollY: 0 };
    const host = { offsetHeight: 77 };
    page.styles.set(host, { position: 'sticky', top: '0px' });
    const target = main('-1', -1, 400);
    load(page).skipTo(target, host);
    target.blur();
    expect(target.attributes.get('tabindex')).toBe('-1');
    expect(page.scrolled).toEqual([323]);
  });
});
