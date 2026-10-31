import { useState } from 'react';
import { ArrowDown, ArrowUp, LoaderCircle, Plus, Save, Trash2 } from 'lucide-react';
import type { Card, FavoriteProfile, FavoriteState } from './types';
import { Modal } from './ui';
import { assignFavorite } from './favorites';

export default function FavoriteEditor({ state, card, busy, onSave, onClose, onReload }: {
 state: FavoriteState; card?: Card; busy: boolean;
 onSave: (p: FavoriteProfile, version: string) => void; onClose: () => void; onReload: () => void;
}) {
 const [draft, setDraft] = useState(() => structuredClone(state.profile));
 const [version] = useState(state.version);
 const [original] = useState(() => JSON.stringify(state.profile));
 const [name, setName] = useState(''), [error, setError] = useState('');
 const dirty = JSON.stringify(draft) !== original;
 const update = (fn: (p: FavoriteProfile) => void) => setDraft(old => { const next = structuredClone(old); fn(next); return next; });
 const close = () => { if (!busy && (!dirty || window.confirm('저장하지 않은 모음 변경사항을 닫을까요?'))) onClose(); };
 const save = async () => { if (busy || !dirty) return; setError(''); try { onSave(draft, version); onClose(); } catch (e) { setError((e as Error).message); } };
 const move = (index: number, delta: number) => update(p => { const [c] = p.collections.splice(index, 1); p.collections.splice(index + delta, 0, c); });
 const add = () => {
  const value = name.trim();
  if (!value) return;
  if (draft.collections.some(c => c.name.toLocaleLowerCase() === value.toLocaleLowerCase())) { setError('이미 사용 중인 모음 이름이에요'); return; }
  update(p => p.collections.push({ id: crypto.randomUUID(), name: value, cards: card ? [card.id] : [] })); setName(''); setError('');
 };
 return <Modal title={card ? '즐겨찾기에 담기' : '즐겨찾기 모음'} subtitle={card?.name || '내 계정의 모음과 시작 화면'} className="favorites-modal" onClose={close} footer={<><span className="save-note">{'선택 완료 후 하단에서 즐겨찾기를 저장하세요'}</span><button className="button primary" disabled={busy || !dirty} onClick={() => void save()}>{busy ? <LoaderCircle size={16} className="spin"/> : <Save size={16}/>}선택 완료</button></>}>
  {error && <div className="error-box" role="alert">{error}{error.includes('다른 기기') && <button className="text-button" disabled={busy} onClick={() => { if (window.confirm('편집 중인 내용을 버리고 최신 모음을 불러올까요?')) onReload(); }}>최신 모음 불러오기</button>}</div>}
  <fieldset className="favorites-fields" disabled={busy}>
   {card ? <><p className="favorites-help">이 서비스를 담을 모음을 선택하세요. 여러 개를 선택할 수 있어요.</p><div className="favorite-memberships">{draft.collections.map(c => <label key={c.id}><input type="checkbox" checked={c.cards.includes(card.id)} onChange={e => setDraft(p => assignFavorite(p, card.id, e.target.checked ? [...p.collections.filter(v => v.cards.includes(card.id)).map(v => v.id), c.id] : p.collections.filter(v => v.id !== c.id && v.cards.includes(card.id)).map(v => v.id)))}/><span>{c.name}</span></label>)}</div></> : <>
    <p className="favorites-help">모음 이름과 순서를 바꾸세요. 모음을 삭제하면 그 모음의 즐겨찾기 연결만 없어집니다.</p>
    <div className="collection-editor-list">{draft.collections.map((c, i) => <div className="collection-editor-row" key={c.id}><input aria-label={`모음 ${i + 1} 이름`} value={c.name} maxLength={40} onChange={e => update(p => { p.collections[i].name = e.target.value; })}/><div><button className="icon-button" aria-label={`${c.name} 위로`} disabled={i === 0} onClick={() => move(i, -1)}><ArrowUp size={16}/></button><button className="icon-button" aria-label={`${c.name} 아래로`} disabled={i === draft.collections.length - 1} onClick={() => move(i, 1)}><ArrowDown size={16}/></button><button className="icon-button danger" aria-label={`${c.name} 모음 삭제`} onClick={() => { if (window.confirm(`‘${c.name}’ 모음을 삭제할까요? 서비스 자체는 삭제되지 않습니다.`)) update(p => { p.collections.splice(i, 1); if (p.startCollection === c.id) p.startCollection = ''; }); }}><Trash2 size={16}/></button></div></div>)}</div>
   </>}
   {!draft.collections.length && <p className="favorites-help">새 모음을 만들어 주세요.</p>}
   <div className="collection-add"><input aria-label="새 모음 이름" placeholder="새 모음 이름" maxLength={40} value={name} onChange={e => setName(e.target.value)} onKeyDown={e => { if (e.key === 'Enter') { e.preventDefault(); add(); } }}/><button className="button secondary" disabled={!name.trim() || draft.collections.length >= 20} onClick={add}><Plus size={16}/>모음 추가</button></div>
   {!card && <label className="field collection-start">처음 열릴 화면<select value={draft.startCollection} onChange={e => update(p => { p.startCollection = e.target.value; })}><option value="">모든 서비스</option>{draft.collections.map(c => <option key={c.id} value={c.id}>{c.name}</option>)}</select><span className="muted">다음에 앱을 처음 열거나 로그인하면 이 화면으로 시작합니다. 직접 연 모음 링크는 해당 모음으로 열려요.</span></label>}
  </fieldset>
 </Modal>;
}
