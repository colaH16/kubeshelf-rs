import { useCallback, useEffect, useState, type MouseEvent } from 'react';

export const viewPaths = { all: '/', favorites: '/favorites', discovery: '/discovery', namespaces: '/namespaces', hidden: '/hidden' } as const;
export type View = keyof typeof viewPaths;
export function viewForPath(path: string): View {
  return (Object.keys(viewPaths) as View[]).find(view => viewPaths[view] === path) || 'all';
}
export function adminView(view: View) { return view === 'discovery' || view === 'namespaces' || view === 'hidden'; }
export function normalNavigation(e: MouseEvent) { return e.button === 0 && !e.metaKey && !e.ctrlKey && !e.shiftKey && !e.altKey; }
export function viewURL(view: View, collection = '') {
 return viewPaths[view] + (view === 'favorites' && /^[a-zA-Z0-9_-]{1,80}$/.test(collection) ? '?collection=' + encodeURIComponent(collection) : '');
}
function currentRoute() { return { view: viewForPath(window.location.pathname), collection: new URLSearchParams(window.location.search).get('collection') || '' }; }
export function useViewRoute() {
 const [route, setRoute] = useState(currentRoute);
 useEffect(() => {
  const update = () => setRoute(currentRoute());
  window.addEventListener('popstate', update);
  return () => window.removeEventListener('popstate', update);
 }, []);
 const navigate = useCallback((view: View, collection = '', replace = false) => {
  const url = viewURL(view, collection);
  const state = { ...window.history.state, kubeshelfNavigated: true };
  if (replace) window.history.replaceState(state, '', url);
  else if (window.location.pathname + window.location.search !== url) window.history.pushState(state, '', url);
  else window.history.replaceState(state, '', url);
  setRoute({ view, collection });
 }, []);
 return [route.view, navigate, route.collection] as const;
}
