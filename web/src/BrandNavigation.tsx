import { useEffect, useState } from 'react';
import { defaultNavigation, navigationURL, parseNavigation } from './navigation';

export function BrandNavigation() {
  const [items, setItems] = useState(defaultNavigation);
  useEffect(() => {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), 3000);
    void fetch(navigationURL, { credentials: 'omit', signal: controller.signal })
      .then(response => response.ok ? response.json() : undefined)
      .then(value => {
        const next = parseNavigation(value);
        if (next && !controller.signal.aborted) setItems(next);
      })
      .catch(() => { /* The bundled links remain usable during a network failure. */ })
      .finally(() => clearTimeout(timer));
    return () => { controller.abort(); clearTimeout(timer); };
  }, []);
  return <nav className="cola-nav" aria-label="cola16의 공간">{items.map(item => <a key={item.id} href={item.href} aria-current={item.id === 'tools' ? 'page' : undefined}>{item.label}</a>)}</nav>;
}
