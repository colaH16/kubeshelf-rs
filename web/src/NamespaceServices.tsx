import { EyeOff } from 'lucide-react';
import { endpointHidden } from './display';
import type { Card, Settings } from './types';

export default function NamespaceServices({ namespace, kind, cards, settings }: { namespace: string; kind: 'ingress' | 'nodeport'; cards: Card[]; settings: Settings }) {
  // A presentation group may contain endpoints from several namespaces.
  const entries = cards.flatMap(card => card.endpoints
    .filter(endpoint => endpoint.namespace === namespace && endpoint.kind === kind)
    .map(endpoint => ({ card, endpoint })))
    .sort((a, b) => a.card.name.localeCompare(b.card.name) || a.endpoint.label.localeCompare(b.endpoint.label) || a.endpoint.id.localeCompare(b.endpoint.id));

  return <div className="namespace-services">
    <div className="namespace-service-scroll" role="region" aria-label={(kind === 'ingress' ? 'Ingress' : 'NodePort') + ' 목록'} tabIndex={0}>
      <p className="namespace-list-help">{kind === 'nodeport' ? 'NodePort는 별도 권한을 사용합니다. ' : ''}서비스·주소별 설정이 우선하며, 숨긴 항목도 표시합니다.</p>
      <ul className="namespace-resource-list">{entries.map(({ card, endpoint: e }) => {
        const app = settings.apps[e.appId];
        const override = app?.endpoints?.[e.id]?.visibility || app?.visibility;
        return <li key={e.id}>
          <div className="namespace-service-title"><strong>{card.name}</strong>{endpointHidden(settings, e) && <span><EyeOff size={11}/>숨김</span>}{e.local && <span className="local-badge">Local</span>}</div>
          <p className="namespace-service-address">{kind === 'ingress' ? e.url || e.label : `${e.scheme === 'tcp' ? e.protocol || 'TCP' : e.scheme.toUpperCase()} :${e.nodePort}`}</p>
          {e.ingresses?.length ? <p className="muted">Ingress · {e.ingresses.join(', ')}</p> : null}
          {(kind === 'ingress' || card.name !== e.service) && <p className="muted">Service · {e.service}{e.port ? `:${e.port}` : ''}</p>}
          {e.needsURL && <p className="namespace-service-note">접속 URL 지정 필요</p>}
          {override && <p className="namespace-service-note">별도 설정 · {override.mode === 'public' ? '누구나' : override.mode === 'admin' ? '관리자만' : '선택한 그룹·사용자'}</p>}
        </li>;
      })}</ul>
      {!entries.length && <p className="muted">현재 발견된 항목이 없습니다.</p>}
    </div>
  </div>;
}
