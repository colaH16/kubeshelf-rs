/* cola16 shared header, contract v1: <cola-header current="blog" subtitle="기록" home="https://cola16.app/">.
 * Canonical source: kubeshelf-rs web/src/cola-header.{js,css} and cola16-navigation.json.
 * build-plum.cjs embeds the CSS and the manifest and serves this as /branding/v1/cola-header.js.
 * Compatible changes stay in v1; changed attributes, slots or fallback classes need /branding/v2/.
 *
 * The element renders into its own shadow root and never changes its light DOM, so host
 * frameworks keep ownership of the fallback links and of the optional slot="account".
 * Light-DOM children other than the account slot are not rendered once it is defined.
 * The account slot comes before the theme button (same visual and keyboard order), so the
 * theme button keeps its place at the right edge with or without a login control.
 * Only public link data is fetched (credentials omitted); accounts stay in each host. */
(() => {
  'use strict';
  if (typeof customElements === 'undefined' || customElements.get('cola-header')) return;
  const CSS = /*@css*/'';
  const DEFAULTS = /*@navigation*/{ version: 1, items: [] };
  const THEMES = ['auto', 'light', 'dark'];
  const NAME = { auto: '자동', light: '라이트', dark: '다크' };
  const TO = { auto: '자동으로', light: '라이트로', dark: '다크로' };
  const ICON = {
    auto: '<svg viewBox="0 0 24 24" aria-hidden="true" focusable="false"><rect x="3" y="3" width="18" height="18" rx="5"/><path d="M8.5 16.5 12 7.5l3.5 9M9.8 13.3h4.4"/></svg>',
    light: '<svg viewBox="0 0 24 24" aria-hidden="true" focusable="false"><circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.93 4.93l1.41 1.41M17.66 17.66l1.41 1.41M2 12h2M20 12h2M6.34 17.66l-1.41 1.41M19.07 4.93l-1.41 1.41"/></svg>',
    dark: '<svg viewBox="0 0 24 24" aria-hidden="true" focusable="false"><path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z"/></svg>',
  };
  const SHELL = '<a class="ch-skip" href="#main">본문 바로가기</a><header class="ch-bar"><a class="ch-brand"></a><nav class="ch-links" aria-label="cola16의 공간"></nav><div class="ch-actions"><slot name="account"></slot><button class="ch-theme" type="button"></button></div><p class="ch-status ch-sr" role="status"></p></header>';
  const root = document.documentElement;
  const media = matchMedia('(prefers-color-scheme:dark)');
  const source = document.currentScript && document.currentScript.src;
  // Served next to /branding/navigation.json; a renamed copy falls back to the page origin.
  const manifestURL = new URL(source ? '../navigation.json' : '/branding/navigation.json', source || location.href).href;

  function parseNavigation(value) {
    if (!value || typeof value !== 'object' || value.version !== 1 || !Array.isArray(value.items) || !value.items.length || value.items.length > 12) return;
    const ids = new Set();
    const items = [];
    for (const entry of value.items) {
      if (!entry || typeof entry !== 'object') return;
      const { id, label, href } = entry;
      if (typeof id !== 'string' || !/^[a-z][a-z0-9-]{0,39}$/.test(id) || ids.has(id)) return;
      if (typeof label !== 'string' || !label.trim() || label.length > 40 || typeof href !== 'string' || href.length > 2048) return;
      let url;
      try { url = new URL(href); } catch { return; }
      if (url.protocol !== 'https:' || url.username || url.password) return;
      ids.add(id); items.push({ id, label, href });
    }
    return items;
  }
  // Where keyboard focus goes after the links change: the same link (by id, then address),
  // or, when it was removed, the link now in its place.
  function focusIndex(previous, items) {
    let index = items.findIndex(item => item.id === previous.id);
    if (index < 0) index = items.findIndex(item => item.href === previous.href);
    return index >= 0 ? index : Math.min(previous.index, items.length - 1);
  }
  // The skip link lives in the shadow root, so a plain #main jump would leave focus on the
  // host's <body>. Focus the host's target directly and scroll it below a sticky header.
  // A temporary tabindex makes a plain <main> focusable; an existing one is left untouched.
  function skipTo(target, header) {
    if (!target.hasAttribute('tabindex') && target.tabIndex < 0) {
      target.setAttribute('tabindex', '-1');
      target.addEventListener('blur', () => target.removeAttribute('tabindex'), { once: true });
    }
    target.focus({ preventScroll: true });
    const style = getComputedStyle(header);
    const covered = style.position === 'fixed' ? header.getBoundingClientRect().bottom
      : style.position === 'sticky' ? (parseFloat(style.top) || 0) + header.offsetHeight : 0;
    const padding = parseFloat(getComputedStyle(root).scrollPaddingTop) || 0;
    scrollTo({ top: scrollY + target.getBoundingClientRect().top - Math.max(covered, padding) });
  }
  const normalize = value => THEMES.includes(value) ? value : 'auto';
  const nextTheme = value => THEMES[(THEMES.indexOf(normalize(value)) + 1) % THEMES.length];
  function stored() { try { return localStorage.getItem('cola-theme'); } catch { return null; } }
  // Same attributes as each site's early theme-init script.
  function applyTheme(preference) {
    root.dataset.colaPreference = preference;
    root.dataset.colaTheme = preference === 'auto' ? (media.matches ? 'dark' : 'light') : preference;
    document.querySelector('meta[name="theme-color"]')?.setAttribute('content', root.dataset.colaTheme === 'dark' ? '#191719' : '#F4F0F1');
  }

  let manifest;
  function loadNavigation() {
    manifest ||= (async () => {
      const controller = new AbortController();
      const timer = setTimeout(() => controller.abort(), 3000);
      try {
        const response = await fetch(manifestURL, { credentials: 'omit', signal: controller.signal });
        return response.ok ? parseNavigation(await response.json()) : undefined;
      } catch { return undefined; } finally { clearTimeout(timer); }
    })();
    return manifest;
  }
  let sheet;
  function adoptStyles(shadow) {
    if ('adoptedStyleSheets' in shadow && 'replaceSync' in CSSStyleSheet.prototype) {
      if (!sheet) { sheet = new CSSStyleSheet(); sheet.replaceSync(CSS); }
      shadow.adoptedStyleSheets = [sheet];
    } else {
      const style = document.createElement('style');
      style.textContent = CSS;
      shadow.prepend(style);
    }
  }

  class ColaHeader extends HTMLElement {
    static observedAttributes = ['current', 'subtitle', 'home'];
    // Public for other hosts and tests; all but skipTo are pure.
    static parseNavigation = parseNavigation;
    static nextTheme = nextTheme;
    static focusIndex = focusIndex;
    static skipTo = skipTo;
    #items = parseNavigation(DEFAULTS) || [];
    #parts;
    #onMedia = () => { if (normalize(root.dataset.colaPreference) === 'auto') applyTheme('auto'); };
    #onStorage = event => {
      if (event.key !== 'cola-theme' && event.key !== null) return;
      applyTheme(normalize(event.newValue));
      this.#showTheme();
    };

    connectedCallback() {
      if (!this.#parts) this.#render();
      applyTheme(normalize(root.dataset.colaPreference || stored()));
      this.#update();
      this.#showTheme();
      media.addEventListener('change', this.#onMedia);
      addEventListener('storage', this.#onStorage);
      void loadNavigation().then(items => {
        if (!items || !this.isConnected || JSON.stringify(items) === JSON.stringify(this.#items)) return;
        this.#items = items;
        this.#update();
      });
    }
    disconnectedCallback() {
      media.removeEventListener('change', this.#onMedia);
      removeEventListener('storage', this.#onStorage);
    }
    attributeChangedCallback() { if (this.#parts) this.#update(); }

    #render() {
      const shadow = this.shadowRoot || this.attachShadow({ mode: 'open' });
      shadow.innerHTML = SHELL;
      adoptStyles(shadow);
      const find = selector => shadow.querySelector(selector);
      this.#parts = { brand: find('.ch-brand'), nav: find('.ch-links'), theme: find('.ch-theme'), status: find('.ch-status') };
      find('.ch-skip').addEventListener('click', event => {
        if (event.button || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
        const target = this.getRootNode().getElementById?.('main');
        if (!target) return;
        event.preventDefault();
        skipTo(target, this);
      });
      this.#parts.theme.addEventListener('click', () => {
        const preference = nextTheme(root.dataset.colaPreference);
        applyTheme(preference);
        let saved = true;
        try { localStorage.setItem('cola-theme', preference); } catch { saved = false; }
        this.#showTheme();
        const { status } = this.#parts;
        status.classList.toggle('ch-sr', saved);
        status.textContent = saved ? `화면 테마를 ${TO[preference]} 바꿨습니다.` : `화면 테마를 ${TO[preference]} 바꿨지만 브라우저에 저장하지 못해 이 화면에만 적용됩니다.`;
      });
    }
    #update() {
      const { brand, nav } = this.#parts;
      const current = this.getAttribute('current') || '';
      const subtitle = this.getAttribute('subtitle') || '';
      brand.href = this.getAttribute('home') || this.#items.find(item => item.id === current)?.href || '/';
      brand.setAttribute('aria-label', subtitle ? `cola16 ${subtitle} 홈` : 'cola16 홈');
      const small = document.createElement('small');
      small.textContent = subtitle;
      brand.replaceChildren(...(subtitle ? ['cola16 ', small] : ['cola16']));
      // Keep keyboard focus on the same destination when a newer manifest replaces the links.
      const active = this.shadowRoot.activeElement;
      const previous = active?.parentNode === nav && { id: active.dataset.id, href: active.getAttribute('href'), index: [...nav.children].indexOf(active) };
      nav.replaceChildren(...this.#items.map(item => {
        const link = document.createElement('a');
        link.dataset.id = item.id;
        link.href = item.href;
        link.textContent = item.label;
        if (item.id === current) link.setAttribute('aria-current', 'page');
        return link;
      }));
      if (previous) nav.children[focusIndex(previous, this.#items)]?.focus();
    }
    #showTheme() {
      const { theme } = this.#parts;
      const preference = normalize(root.dataset.colaPreference);
      const label = `화면 테마: ${NAME[preference]}. 누르면 ${TO[nextTheme(preference)]} 바뀝니다`;
      theme.innerHTML = ICON[preference];
      theme.setAttribute('aria-label', label);
      theme.title = label;
    }
  }
  customElements.define('cola-header', ColaHeader);
})();
