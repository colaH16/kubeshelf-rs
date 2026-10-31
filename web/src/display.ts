import type { AppSettings, Card, DisplayMode, Endpoint, Settings } from './types';

export function appDisplay(app?: AppSettings): DisplayMode {
  return app?.display || (app?.hidden ? 'hide' : 'inherit');
}

export function namespaceDisplay(settings: Settings, endpoint?: Endpoint) {
  if (!endpoint?.namespace) return { source: '기본 설정', hidden: false };
  const nodePort = endpoint.kind === 'nodeport';
  const policy = (nodePort ? settings.nodePortNamespaces : settings.namespaces)?.[endpoint.namespace];
  return { source: `${endpoint.namespace} · ${nodePort ? 'NodePort' : 'Ingress'}`, hidden: !!policy?.hidden };
}

export function endpointHidden(settings: Settings, endpoint: Endpoint) {
  const group = settings.assignments[endpoint.id];
  if (group && appDisplay(settings.apps[group]) === 'hide') return true;
  const display = appDisplay(settings.apps[endpoint.appId]);
  return display === 'inherit' ? namespaceDisplay(settings, endpoint).hidden : display === 'hide';
}

export function serviceDisplay(settings: Settings, id: string, endpoints: Endpoint[]): DisplayMode | 'mixed' {
  if (endpoints.some(e => e.appId !== id) && appDisplay(settings.apps[id]) === 'hide') return 'hide';
  const modes = endpoints.length ? endpoints.map(e => appDisplay(settings.apps[e.appId])) : [appDisplay(settings.apps[id])];
  return modes.every(m => m === modes[0]) ? modes[0] : 'mixed';
}

export function setServiceDisplay(settings: Settings, id: string, endpoints: Endpoint[], mode: DisplayMode) {
  // Clear legacy group hiding; current original services receive the explicit
  // choice, while future grouped addresses keep their own namespace defaults.
  const ids = new Set(endpoints.length ? endpoints.map(e => e.appId) : [id]);
  if (!ids.has(id) && settings.apps[id]) { settings.apps[id].hidden = false; settings.apps[id].display = 'inherit'; }
  for (const appID of ids) {
    const app = settings.apps[appID] ||= { hidden: false };
    app.hidden = false;
    app.display = mode;
  }
}

// Admin catalogs include hidden addresses for editing. Keep those addresses out
// of dashboard cards, including groups containing both shown and hidden URLs.
export function displayCards(cards: Card[], hidden: boolean): Card[] {
  return cards.flatMap(card => {
    const endpoints = card.endpoints.filter(e => (e.hidden ?? card.hidden) === hidden);
    return endpoints.length ? [{ ...card, hidden, endpoints }] : [];
  });
}
