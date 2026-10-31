import { useState } from 'react';
import { Globe, Network } from 'lucide-react';
import type { Card, Directory, Policy, Settings } from './types';
import { PolicyEditor } from './ui';
import NamespaceServices from './NamespaceServices';

export default function NamespacePolicyEditor({ namespace, cards, directory, settings, onChange }: {
  namespace: string; cards: Card[]; directory: Directory; settings: Settings;
  onChange: (kind: 'ingress' | 'nodeport', policy: Policy) => void;
}) {
  const endpoints = cards.flatMap(c => c.endpoints).filter(e => e.namespace === namespace);
  const counts = { ingress: endpoints.filter(e => e.kind === 'ingress').length, nodeport: endpoints.filter(e => e.kind === 'nodeport').length };
  const kinds = (['ingress', 'nodeport'] as const).filter(k => counts[k] > 0 || k === 'ingress' && !counts.nodeport);
  const [selected, setSelected] = useState<'ingress' | 'nodeport'>(kinds[0]);
  const kind = kinds.includes(selected) ? selected : kinds[0];
  const policy = kind === 'nodeport' ? settings.nodePortNamespaces?.[namespace] : settings.namespaces[namespace];
  const name = kind === 'ingress' ? 'Ingress' : 'NodePort';

  return <div className="namespace-policy-editor">
    <div className="namespace-kind-tabs" role="tablist" aria-label="접속 방식">
      {kinds.map(k => {
        const Icon = k === 'ingress' ? Globe : Network;
        return <button key={k} id={'namespace-tab-' + k} role="tab" aria-selected={kind === k} aria-controls="namespace-policy-panel" onClick={() => setSelected(k)}><Icon size={15}/>{k === 'ingress' ? 'Ingress' : 'NodePort'}<span>{counts[k]}</span></button>;
      })}
    </div>
    <div className="namespace-policy-panel" id="namespace-policy-panel" role="tabpanel" aria-labelledby={'namespace-tab-' + kind}>
      <div className="namespace-policy-controls">
        <label className="check-row namespace-hide-control"><input type="checkbox" aria-label={name + ' 기본 숨김'} checked={!!policy?.hidden} onChange={e => onChange(kind, { ...(policy || { mode: 'admin' }), hidden: e.target.checked })}/>대시보드에서 기본 숨기기</label>
        <div className="namespace-policy-caption"><span>기본 공개 범위</span>{!policy && <button className="text-button" onClick={() => onChange(kind, { mode: 'admin' })}>관리자만으로 확인</button>}</div>
        <PolicyEditor key={kind} compact inherit={false} label={name + ' 공개 범위'} directory={directory} value={policy || { mode: 'admin' }} onChange={p => onChange(kind, { ...p!, hidden: policy?.hidden })}/>
      </div>
      <NamespaceServices key={kind} namespace={namespace} kind={kind} cards={cards} settings={settings}/>
    </div>
  </div>;
}
