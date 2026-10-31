import { Trash2 } from 'lucide-react';
import type { Directory, Target } from './types';
import { PolicyEditor } from './ui';

export default function TargetEditor({ target: t, index, nodes, directory, onChange, onRemove }: {
  target: Target; index: number; nodes: Target[]; directory: Directory; onChange: (value: Partial<Target>) => void; onRemove: () => void;
}) {
  return <section className="target-editor">
    <div className="target-title"><strong>도메인 {index + 1}</strong><button className="icon-button danger" aria-label="도메인 삭제" onClick={onRemove}><Trash2 size={16}/></button></div>
    <div className="two-fields"><label className="field">표시 이름<input value={t.name} placeholder="예: 집 Kubernetes" onChange={e => onChange({ name: e.target.value })}/></label><label className="field">도메인<input value={t.host} placeholder="node.example.com" onChange={e => onChange({ host: e.target.value })}/></label></div>
    <p className="target-help">링크는 <strong>{t.host || 'node.example.com'}:NodePort</strong>로 열립니다. 접속할 IP는 브라우저에서 DNS로 찾습니다.</p>
    <div className="field">도메인 공개 범위<PolicyEditor inherit={false} directory={directory} value={t.visibility || { mode: 'admin' }} onChange={p => onChange({ visibility: p })}/></div>
    <details className="target-advanced"><summary>고급 설정 · Local 서비스 / TCP 진단{t.nodeName || t.tcpEnabled ? ' · 설정 있음' : ''}</summary>
      <label className="field">Local 서비스용 노드 지정 · 선택<select value={t.nodeName || ''} onChange={e => onChange({ nodeName: e.target.value })}><option value="">지정 안 함 · 일반 NodePort에 사용</option>{nodes.map(n => <option key={n.id} value={n.nodeName}>{n.name}</option>)}{t.nodeName && !nodes.some(n => n.nodeName === t.nodeName) && <option value={t.nodeName}>{t.nodeName} · 현재 사용 불가</option>}</select></label>
      <p className="target-help">Local 서비스는 실제 접속할 노드에 Ready Pod가 있어야 합니다. 이 지정은 그 조건을 확인하는 데 사용하며 DNS나 접속 주소를 바꾸지 않습니다. 지정하지 않으면 이 도메인은 일반 NodePort에서만 선택됩니다.</p>
      <label className="check-row"><input type="checkbox" checked={t.tcpEnabled} onChange={e => onChange({ tcpEnabled: e.target.checked })}/>수동 TCP 진단 버튼 표시</label>
      <p className="target-help">관리자가 버튼을 누를 때 KubeShelf 서버에서 도메인:포트의 TCP 연결을 확인합니다. 자동 검사나 웹 로그인 화면 확인은 하지 않으며, Pod 상태 표시에도 영향을 주지 않습니다.</p>
    </details>
  </section>;
}
