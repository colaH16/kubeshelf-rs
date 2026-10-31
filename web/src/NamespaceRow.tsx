import { EyeOff, FolderTree, Globe, LockKeyhole, Pencil } from 'lucide-react';
import type { NamespaceInfo } from './types';
import { policyLabel } from './selection';

export default function NamespaceRow({ namespace: n, onClick }: { namespace: NamespaceInfo; onClick: () => void }) {
  const sources = [
    { name: 'Ingress', count: n.ingressServices, policy: n.policy },
    { name: 'NodePort', count: n.nodePortServices, policy: n.nodePortPolicy },
  ].filter(s => s.count > 0);
  return <button className="namespace-row" onClick={onClick}>
    <span className="namespace-icon"><FolderTree size={20}/></span>
    <div><strong>{n.name}{!n.configured && <span className="new-badge">NEW</span>}</strong><small>{sources.map(s => `${s.name} ${s.count}`).join(' · ')}{!n.configured && ' · 권한 확인 필요'}</small></div>
    <span className="namespace-policy-tags">{sources.map(s => <span key={s.name} className="policy-tag" title={(s.policy.hidden ? '기본 숨김 · ' : '') + policyLabel(s.policy)}>{s.policy.hidden ? <EyeOff size={12}/> : s.policy.mode === 'public' ? <Globe size={12}/> : <LockKeyhole size={12}/>} {s.name} · {s.policy.hidden && '기본 숨김 · '}{s.policy.mode === 'public' ? '공개' : s.policy.mode === 'admin' ? '관리자' : '제한됨'}</span>)}</span>
    <Pencil size={15}/>
  </button>;
}
