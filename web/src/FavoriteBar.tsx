import { Plus, Settings2 } from 'lucide-react';
import type { Card, FavoriteState } from './types';
import { normalNavigation, viewURL } from './routing';

export default function FavoriteBar({ state, admin, selected, cards, busy, onSelect, onManage, onStart }: {
 state: FavoriteState; admin: boolean; selected: string; cards: Card[]; busy: boolean;
 onSelect: (id: string) => void; onManage: () => void; onStart: (enabled: boolean) => void;
}) {
 const profile = state.profile;
 return <div className="favorites-tools">
  {admin ? <div className="collection-navigation"><nav className="collection-tabs" aria-label="즐겨찾기 모음">{profile.collections.map(c => <a href={viewURL('favorites', c.id)} key={c.id} className={c.id === selected ? 'active' : ''} aria-current={c.id === selected ? 'page' : undefined} onClick={e => { if (normalNavigation(e)) { e.preventDefault(); onSelect(c.id); } }}>{c.name}<span>{cards.filter(card => c.cards.includes(card.id)).length}</span></a>)}<button className="icon-button" aria-label="즐겨찾기 모음 추가" disabled={busy} onClick={onManage}><Plus size={18}/></button></nav><button className="button secondary small" disabled={busy} onClick={onManage}><Settings2 size={15}/>모음 관리</button></div> : <label className="check-row favorite-start"><input type="checkbox" checked={!!profile.startCollection} disabled={busy} onChange={e => onStart(e.target.checked)}/>즐겨찾기를 시작 화면으로</label>}

 </div>;
}
