import defaults from './cola16-navigation.json';

export type NavigationItem = { id: string; label: string; href: string };
export const navigationURL = '/branding/navigation.json';
export const defaultNavigation: NavigationItem[] = defaults.items;

// Only public link data crosses origins. Neither cookies nor account data are used.
export function parseNavigation(value: unknown): NavigationItem[] | undefined {
  if (!value || typeof value !== 'object') return;
  const manifest = value as { version?: unknown; items?: unknown };
  if (manifest.version !== 1 || !Array.isArray(manifest.items) || !manifest.items.length || manifest.items.length > 12) return;
  const ids = new Set<string>();
  const items: NavigationItem[] = [];
  for (const entry of manifest.items) {
    if (!entry || typeof entry !== 'object') return;
    const { id, label, href } = entry;
    if (typeof id !== 'string' || !/^[a-z][a-z0-9-]{0,39}$/.test(id) || ids.has(id)) return;
    if (typeof label !== 'string' || !label.trim() || label.length > 40 || typeof href !== 'string' || href.length > 2048) return;
    try {
      const url = new URL(href);
      if (url.protocol !== 'https:' || url.username || url.password) return;
    } catch { return; }
    ids.add(id); items.push({ id, label, href });
  }
  return items;
}
