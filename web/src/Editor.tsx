import { useState } from 'react';
import { Check, ChevronDown, Plus, Save, Trash2 } from 'lucide-react';
import type { AdminState, AppSettings, Card, Endpoint, Settings } from './types';
import { AppIcon, iconNames, Modal, PolicyEditor } from './ui';
import NamespacePolicyEditor from './NamespacePolicyEditor';
import DisplayEditor from './DisplayEditor';
import TargetEditor from './TargetEditor';
import { serviceDisplay, setServiceDisplay } from './display';
import { inheritedPolicy, servicePolicyParents, servicePolicyState } from './policy';
export type EditRequest = { kind: 'card'; card: Card } | { kind: 'namespace'; namespace: string } | { kind: 'targets' } | { kind: 'manual'; id?: string };
function comparable(s: Settings) { const v = { ...s, revision: '' }; return JSON.stringify(v); }
export default function Editor({ request, data, saving, onSave, onClose }: { request: EditRequest; data: AdminState; saving: boolean; onSave: (s: Settings, base: string, reviewEndpoints?: string[]) => Promise<{ baseCommit: string; revision: string; settings: Settings }>; onClose: () => void }) {
  const [newManualID] = useState(() => request.kind === 'manual' ? request.id || 'manual-' + crypto.randomUUID() : undefined);
  const [draft, setDraft] = useState<Settings>(() => {
    const s = structuredClone(data.settings);
    if (request.kind === 'manual' && !request.id) { const id = newManualID!; s.manual.push({ id, name: '', urls: [{ id: 'url-' + crypto.randomUUID(), label: '기본 주소', url: '' }] }); s.apps[id] = { hidden: false, visibility: { mode: 'admin' } }; }
    return s;
  });
  const [base, setBase] = useState(data.baseCommit);
  const [saved, setSaved] = useState(comparable(data.settings));
  const [error, setError] = useState('');
  const dirty = comparable(draft) !== saved;
  const change = (f: (s: Settings) => void) => setDraft(old => { const next = structuredClone(old); f(next); return next; });
  const app = (s: Settings, id: string): AppSettings => s.apps[id] ||= { hidden: false };
  const eSettings = (s: Settings, e: Endpoint) => { const a = app(s, e.appId); a.endpoints ||= {}; return a.endpoints[e.id] ||= {}; };
  const persist = async (value = draft, reviewEndpoints: string[] = []) => { setError(''); try { const result = await onSave(value, base, reviewEndpoints); setBase(result.baseCommit); setSaved(comparable(result.settings)); setDraft(current => { if (comparable(current) === comparable(value)) return structuredClone(result.settings); const next = structuredClone(current); next.revision = result.revision; for (const [id, a] of Object.entries(result.settings.apps)) { if (a.reviewed) app(next, id).reviewed = a.reviewed; } return next; }); } catch (e) { setError((e as Error).message); } };
  const close = () => { if (!dirty || window.confirm('저장하지 않은 변경사항을 닫을까요?')) onClose(); };
  const card = request.kind === 'card' ? data.catalog.cards.find(c => c.id === request.card.id && c.source === request.card.source) || request.card : undefined;
  const manualID = request.kind === 'manual' ? newManualID : card?.source === 'custom' ? card.endpoints[0]?.appId : undefined;
  const manual = draft.manual.find(m => m.id === manualID);
  const reviewEndpointIDs = card && card.source !== 'custom' ? card.endpoints.map(e => e.id) : [];
  // Both actions acknowledge the displayed endpoints. The server fingerprints
  // the submitted settings so edited addresses are reviewed in their saved form.
  const review = () => { if (!card || saving || dirty) return; void persist(draft, reviewEndpointIDs); };
  const title = request.kind === 'targets' ? 'NodePort 도메인' : request.kind === 'namespace' ? '공개 범위' : request.kind === 'manual' ? '서비스 직접 추가' : '서비스 설정';
  const subtitle = request.kind === 'namespace' ? request.namespace : card?.name;
  const editID = card?.id || manualID;
  const meta = editID ? draft.apps[editID] || { hidden: false } : undefined;
  const serviceIDs = [...new Set(card?.endpoints.map(e => e.appId) || (editID ? [editID] : []))];
  const servicePolicy = servicePolicyState(draft, serviceIDs);
  return <Modal className={request.kind === 'namespace' ? 'namespace-modal' : ''} title={title} subtitle={subtitle} onClose={close} footer={<><span className="save-note">{request.kind === 'namespace' ? (saving ? '저장 중…' : dirty ? '변경사항 있음' : '저장됨') : (saving ? 'Git에 저장하는 중…' : dirty ? '저장하지 않은 변경사항' : 'Git에 저장됨')}</span><div className="footer-actions">{card && card.source !== 'custom' && <button className="button secondary" disabled={saving || dirty} title={dirty ? '변경사항을 먼저 저장해 주세요' : 'NEW·CHANGED 표시를 확인 완료로 변경합니다'} onClick={review}><Check size={16}/>설정 확인 완료</button>}<button className="button primary" disabled={saving || !dirty} onClick={() => void persist(draft, reviewEndpointIDs)}><Save size={16}/>{saving ? '저장 중…' : '저장'}</button></div></>}>
    {error && <div role="alert" className="error-box">{error}{error.includes('다른 곳') && <p>다른 창이나 Git에서 변경된 설정을 덮어쓰지 않았습니다. 입력 내용을 확인한 뒤 창을 다시 열어 최신 설정을 불러와 주세요.</p>}</div>}
    {editID && meta && <>
      <div className="editor-preview"><span className="app-icon tone-amber"><AppIcon name={meta.name || manual?.name || card?.name || 'Service'} icon={meta.icon || ''}/></span><div><strong>{meta.name || manual?.name || card?.name || '새 서비스'}</strong><small>표시 이름과 아이콘을 원하는 대로 바꿔보세요.</small></div></div>
      <label className="field">표시 이름<input value={meta.name ?? manual?.name ?? card?.name ?? ''} maxLength={200} onChange={e => change(s => { app(s, editID).name = e.target.value; const m = s.manual.find(v => v.id === manualID); if (m) m.name = e.target.value; })}/></label>
      <label className="field">설명<input value={meta.description || ''} maxLength={2000} placeholder="이 서비스는 어디에 쓰나요?" onChange={e => change(s => { app(s, editID).description = e.target.value; })}/></label>
      <div className="field">아이콘<div className="icon-picker">{iconNames.map(icon => <button key={icon} aria-label={`아이콘 ${icon}`} className={meta.icon === icon ? 'selected' : ''} onClick={() => change(s => { app(s, editID).icon = icon; })}><AppIcon icon={icon} name=""/></button>)}</div><input aria-label="직접 지정한 아이콘 URL" value={meta.icon && !iconNames.includes(meta.icon) ? meta.icon : ''} placeholder="또는 아이콘 이미지 URL (https://…)" onChange={e => change(s => { app(s, editID).icon = e.target.value; })}/></div>
      <DisplayEditor value={serviceDisplay(draft, editID, card?.endpoints || [])} settings={draft} endpoints={card?.endpoints || []} onChange={mode => change(s => setServiceDisplay(s, editID, card?.endpoints || [], mode))}/>
      <div className="field">서비스 표시 범위<PolicyEditor label="서비스 표시 범위" directory={data.directory} value={servicePolicy.value} mixed={servicePolicy.mixed} parents={servicePolicyParents(draft, card?.endpoints || [])} onChange={p => change(s => { for (const id of serviceIDs) app(s, id).visibility = p; })}/></div>
    </>}
    {manual && <div className="field">접속 주소{manual.urls.map((u, i) => <div className="address-edit" key={u.id}><input aria-label={`주소 ${i + 1} 이름`} value={u.label} placeholder="주소 이름" onChange={e => change(s => { s.manual.find(m => m.id === manualID)!.urls[i].label = e.target.value; })}/><input aria-label={`주소 ${i + 1} URL`} value={u.url} placeholder="https://…" onChange={e => change(s => { s.manual.find(m => m.id === manualID)!.urls[i].url = e.target.value; })}/><button className="icon-button danger" aria-label={`주소 ${i + 1} 삭제`} onClick={() => change(s => { s.manual.find(m => m.id === manualID)!.urls.splice(i, 1); })}><Trash2 size={16}/></button></div>)}<button className="button secondary" onClick={() => change(s => { s.manual.find(m => m.id === manualID)!.urls.push({ id: 'url-' + crypto.randomUUID(), label: '', url: '' }); })}><Plus size={16}/>주소 추가</button>{request.kind === 'card' && <button className="text-button danger" onClick={() => { if (window.confirm('수동 등록 서비스를 삭제할까요?')) { const next = structuredClone(draft); next.manual = next.manual.filter(m => m.id !== manualID); delete next.apps[manualID!]; setDraft(next); void onSave(next, base).then(onClose).catch(e => setError(e.message)); } }}>수동 서비스 삭제</button>}</div>}
    {card && card.source !== 'custom' && <div className="field">발견된 접속 주소{card.endpoints.map(e => { const override = draft.apps[e.appId]?.endpoints?.[e.id] || {}; return <details className="endpoint-editor" key={e.id} open={card.endpoints.length === 1}><summary><span>{e.label || e.service}</span><ChevronDown size={15}/></summary><p className="muted">{e.namespace} / {e.service} · {e.nodePort ? `NodePort ${e.nodePort}` : `포트 ${e.port}`}</p><label className="field">주소 이름<input value={override.label ?? e.label} onChange={ev => change(s => { eSettings(s, e).label = ev.target.value; })}/></label><label className="field">프로토콜<select value={override.scheme || e.scheme} onChange={ev => change(s => { eSettings(s, e).scheme = ev.target.value; })}><option value="http">HTTP</option><option value="https">HTTPS</option><option value="tcp">웹 서비스 아님 · 주소 복사</option></select></label>{e.kind === 'ingress' && <label className="field">직접 지정할 접속 URL<input value={override.url || ''} placeholder={e.url || 'https://실제-접속-주소/'} onChange={ev => change(s => { eSettings(s, e).url = ev.target.value; })}/></label>}<div className="field">이 주소의 표시 범위<PolicyEditor label="이 주소의 표시 범위" directory={data.directory} parents={[inheritedPolicy(draft, e, 'address')]} value={override.visibility} onChange={p => change(s => { eSettings(s, e).visibility = p; })}/></div><label className="field">카드 묶기<select value={draft.assignments[e.id] || ''} onChange={ev => change(s => { const v = ev.target.value; if (!v) delete s.assignments[e.id]; else if (v === '__split') { const id = 'group-' + crypto.randomUUID(); s.assignments[e.id] = id; s.apps[id] = { name: e.label || e.service, hidden: false }; } else s.assignments[e.id] = v; })}><option value="">자동 묶음 사용</option>{data.catalog.cards.filter(c => c.source === card.source).map(c => <option value={c.id} key={c.id}>{c.name}</option>)}{draft.assignments[e.id] && !data.catalog.cards.some(c => c.id === draft.assignments[e.id]) && <option value={draft.assignments[e.id]}>새로 분리한 카드</option>}<option value="__split">이 주소를 새 카드로 분리</option></select></label><p className="muted">카드를 묶어도 각 주소의 공개 범위는 유지됩니다.</p></details>; })}</div>}
    {request.kind === 'namespace' && <NamespacePolicyEditor namespace={request.namespace} cards={data.catalog.cards} directory={data.directory} settings={draft} onChange={(kind, policy) => change(s => { if (kind === 'nodeport') { s.nodePortNamespaces ||= {}; s.nodePortNamespaces[request.namespace] = policy; } else { s.namespaces[request.namespace] = policy; } })}/>}
    {request.kind === 'targets' && <>
      <p className="muted">노드 IP 대신 사용할 도메인을 등록하세요. 프로토콜이나 포트 없이 도메인만 입력하면 각 서비스의 NodePort가 붙습니다. 등록 정보는 저장 버튼을 눌러 Git에 저장합니다.</p>
      {draft.targets.map((t, i) => <TargetEditor key={t.id} target={t} index={i} nodes={data.catalog.targets.filter(n => !n.manual)} directory={data.directory} onChange={patch => change(s => { Object.assign(s.targets[i], patch); })} onRemove={() => change(s => { s.targets.splice(i, 1); })}/>)}
      <button className="button secondary" onClick={() => change(s => { s.targets.push({ id: 'domain-' + crypto.randomUUID(), name: '', host: '', manual: true, tcpEnabled: false, visibility: { mode: 'admin' } }); })}><Plus size={16}/>도메인 추가</button>
    </>}
  </Modal>;
}
