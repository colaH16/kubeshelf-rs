import { useCallback, useEffect, useRef, useState } from 'react';
import { Activity, ArrowDownToLine, ArrowUpRight, Boxes, Check, ChevronDown, Copy, EyeOff, FolderTree, Globe, Inbox, Layers3, LoaderCircle, LogIn, LogOut, Menu, Network, Pencil, Plus, Search, Server, Save, Settings2, ShieldCheck, Star, X } from 'lucide-react';
import { api } from './api';
import Editor, { type EditRequest } from './Editor';
import NamespaceRow from './NamespaceRow';
import { displayCards, setServiceDisplay } from './display';
import { AppIcon } from './ui';
import { chooseTarget } from './selection';
import type { AdminState, ApplyStatus, Card, Catalog, Endpoint, Settings } from './types';
import { adminView, normalNavigation, useViewRoute, viewPaths, viewURL, type View } from './routing';
import { useFavorites } from './useFavorites';
import { assignFavorite, favoriteIDs } from './favorites';
import FavoriteEditor from './FavoriteEditor';
import FavoriteBar from './FavoriteBar';
import type { FavoriteProfile } from './types';
function firstTarget(catalog?: Catalog) { return catalog?.cards.flatMap(c => c.endpoints).find(e => e.kind === 'nodeport' && e.local && e.targets.length)?.targets[0]?.targetId || catalog?.targets[0]?.id || ''; }
type Preferences = { favorites: string[]; selected: Record<string, string>; target: string; previous: Record<string, string> };
const emptyPrefs = (): Preferences => ({ favorites: [], selected: {}, target: '', previous: {} });
function safePrefs(key: string): Preferences { try { const v = JSON.parse(localStorage.getItem(key) || '{}'); return { favorites: Array.isArray(v.favorites) ? v.favorites.filter((x: unknown) => typeof x === 'string') : [], selected: v.selected && typeof v.selected === 'object' ? v.selected : {}, target: typeof v.target === 'string' ? v.target : '', previous: v.previous && typeof v.previous === 'object' ? v.previous : {} }; } catch { return emptyPrefs(); } }
export default function App() {
  const [catalog, setCatalog] = useState<Catalog>();
  const [error, setError] = useState('');
  const [view, setView, collectionID] = useViewRoute();
  const [favoriteEditor, setFavoriteEditor] = useState<{ card?: Card }>();
  const startHandled = useRef('');
  const [query, setQuery] = useState('');
  const [prefs, setPrefs] = useState<Preferences>(emptyPrefs);
  const [admin, setAdmin] = useState<AdminState>();
  const [editor, setEditor] = useState<EditRequest>();
  const [status, setStatus] = useState<ApplyStatus>();
  const [saving, setSaving] = useState(false);
  const [opening, setOpening] = useState(false);
  const [toast, setToast] = useState('');
  const [mobile, setMobile] = useState(false);
  const who = useRef<string | null>(null);
  const saveLock = useRef(false);
  const mounted = useRef(true);
  const requestNumber = useRef(0);
  const loggingOut = useRef(false);
  const refresh = useCallback(async () => {
    if (loggingOut.current) return;
    const request = ++requestNumber.current;
    try {
      const cat = await api<Catalog>('/api/catalog');
      if (!mounted.current || request !== requestNumber.current || loggingOut.current) return;
      const identity = cat.identity.subject || 'anonymous';
      if (who.current !== identity) { who.current = identity; setPrefs(safePrefs('kubeshelf:' + identity)); setAdmin(undefined); setEditor(undefined); setFavoriteEditor(undefined); setStatus(undefined); setQuery(''); }
      setCatalog(cat); setError('');
      if (cat.identity.admin && !saveLock.current) { const next = await api<ApplyStatus>('/api/admin/status'); if (mounted.current && request === requestNumber.current && who.current === identity) setStatus(next); }
    } catch (e) { if (mounted.current && request === requestNumber.current && !loggingOut.current) setError((e as Error).message); }
  }, []);
  useEffect(() => { mounted.current = true; void refresh(); const timer = setInterval(() => void refresh(), 5000); return () => { mounted.current = false; clearInterval(timer); }; }, [refresh]);
  useEffect(() => { if (!toast) return; const timer = setTimeout(() => setToast(''), 4000); return () => clearTimeout(timer); }, [toast]);
  useEffect(() => { if (who.current !== null) { try { localStorage.setItem('kubeshelf:' + who.current, JSON.stringify(prefs)); } catch { /* private browser storage can be unavailable */ } } }, [prefs]);
  const notice = (message: string) => setToast(message);
  const openEditor = async (request: EditRequest) => { const identity = who.current; setOpening(true); try { const data = await api<AdminState>('/api/admin/state'); if (loggingOut.current || who.current !== identity) return; setAdmin(data); setEditor(request); } catch (e) { notice((e as Error).message); } finally { setOpening(false); } };
  const save = async (settings: Settings, baseCommit: string, reviewEndpoints: string[] = []) => {
    if (saveLock.current) throw new Error('다른 저장이 끝날 때까지 기다려 주세요');
    saveLock.current = true; ++requestNumber.current; setSaving(true);
    try {
      const result = await api<{ status: ApplyStatus; baseCommit: string; settings: Settings }>('/api/admin/state', { method: 'PUT', headers: { 'X-CSRF-Token': catalog?.csrf || '' }, body: JSON.stringify({ settings, baseCommit, reviewEndpoints }) });
      setStatus(result.status); setAdmin(old => old ? { ...old, settings: result.settings, baseCommit: result.baseCommit, status: result.status } : old); notice('Git에 저장했어요. 설정 적용을 기다리고 있습니다.');
      return { baseCommit: result.baseCommit, revision: result.status.desiredRevision, settings: result.settings };
    } finally { saveLock.current = false; setSaving(false); }
  };
  const favorites = useFavorites(catalog?.identity.subject || '', catalog?.csrf || '', saving);
  const choiceBusy = saving || favorites.saving;
  const account = !!catalog?.identity.subject;
  const preferredTarget = account ? favorites.state?.profile.defaultTarget : prefs.target;
  const selectedTarget = catalog?.targets.some(t => t.id === preferredTarget) ? preferredTarget! : firstTarget(catalog);
  const loginURL = '/auth/login?returnTo=' + encodeURIComponent(viewURL(view, collectionID));
  const stagePersonal = (update: (p: FavoriteProfile) => void) => {
    if (!favorites.state || choiceBusy) return;
    const next = structuredClone(favorites.state.profile); update(next); favorites.stage(next);
  };
  useEffect(() => {
    const subject = catalog?.identity.subject;
    if (!subject) { startHandled.current = ''; return; }
    if (!favorites.state || startHandled.current === subject) return;
    startHandled.current = subject;
    const start = favorites.state.profile.startCollection;
    if (start && window.location.pathname === '/' && !window.history.state?.kubeshelfNavigated) setView('favorites', catalog.identity.admin ? start : '', true);
  }, [catalog?.identity.subject, catalog?.identity.admin, favorites.state, setView]);
  useEffect(() => {
    if (!catalog) return;
    setPrefs(old => {
      const next = structuredClone(old);
      if (!catalog.targets.some(t => t.id === next.target)) next.target = firstTarget(catalog);
      for (const e of catalog.cards.flatMap(c => c.endpoints)) { if (e.kind !== 'nodeport') continue; const target = chooseTarget(selectedTarget, next.previous[e.id], e.targets); if (target) next.previous[e.id] = target.targetId; }
      return JSON.stringify(next) === JSON.stringify(old) ? old : next;
    });
  }, [catalog, selectedTarget]);
  useEffect(() => { setMobile(false); }, [view]);
  const restore = async (card: Card) => { try { const data = await api<AdminState>('/api/admin/state'); const next = structuredClone(data.settings); setServiceDisplay(next, card.id, card.endpoints, 'show'); await save(next, data.baseCommit); } catch (e) { notice((e as Error).message); } };
  const logout = async () => {
    if (loggingOut.current) return;
    loggingOut.current = true; ++requestNumber.current;
    setCatalog(undefined); setAdmin(undefined); setEditor(undefined); setFavoriteEditor(undefined); setStatus(undefined); setQuery(''); setPrefs(emptyPrefs()); who.current = null;
    try { await api('/auth/logout', { method: 'POST', headers: { 'X-CSRF-Token': catalog?.csrf || '' } }); }
    catch (e) { notice((e as Error).message); }
    finally { loggingOut.current = false; await refresh(); }
  };
  const copy = async (text: string) => { try { await navigator.clipboard.writeText(text); notice('주소를 복사했어요'); } catch { notice('주소를 복사하지 못했어요'); } };
  const favorite = (card: Card) => {
    if (choiceBusy) return;
    if (!account) { setPrefs(p => ({ ...p, favorites: p.favorites.includes(card.id) ? p.favorites.filter(v => v !== card.id) : [...p.favorites, card.id] })); return; }
    if (!favorites.state) return;
    if (catalog?.identity.admin) setFavoriteEditor({ card });
    else {
      const group = favorites.state.profile.collections[0];
      favorites.stage(assignFavorite(favorites.state.profile, card.id, group.cards.includes(card.id) ? [] : [group.id]));
    }
  };
  const shownCards = displayCards(catalog?.cards || [], false);
  const hiddenCards = displayCards(catalog?.cards || [], true);
  const allFavorites = account ? favoriteIDs(favorites.state?.profile) : prefs.favorites;
  const activeCollection = favorites.state?.profile.collections.find(c => c.id === collectionID) || favorites.state?.profile.collections.find(c => c.id === favorites.state?.profile.startCollection) || favorites.state?.profile.collections[0];
  const shownFavorites = account ? activeCollection?.cards || [] : prefs.favorites;
  useEffect(() => {
    if (view === 'favorites' && catalog?.identity.admin && activeCollection && collectionID !== activeCollection.id) setView('favorites', activeCollection.id, true);
  }, [view, catalog?.identity.admin, collectionID, activeCollection?.id, setView]);
  const counts = { total: shownCards.length, favorites: shownCards.filter(c => allFavorites.includes(c.id)).length, discovery: shownCards.filter(c => c.new || c.changed).length, namespaces: catalog?.namespaces?.filter(n => !n.configured).length || 0, hidden: hiddenCards.length };
  const move = (v: View) => { setView(v, v === 'favorites' && catalog?.identity.admin ? activeCollection?.id : undefined); setMobile(false); };
  const titles: Record<View, [string, string]> = { all: ['내 서비스', '흩어져 있던 앱을, 한 곳에.'], favorites: ['즐겨찾기', '자주 찾는 서비스에 더 빠르게.'], discovery: ['새로 발견했어요', '클러스터에서 찾은 서비스와 바뀐 주소를 확인하세요.'], namespaces: ['네임스페이스', '새로 발견되는 서비스의 기본 공개 범위를 관리하세요.'], hidden: ['숨긴 서비스', '필요할 때 다시 대시보드에 꺼내 놓으세요.'] };
  const cards = (view === 'hidden' ? hiddenCards : shownCards).filter(c => {
    if (view === 'favorites' && !shownFavorites.includes(c.id)) return false;
    if (view === 'discovery' && !c.new && !c.changed) return false;
    const text = [c.name, c.description, c.namespace, ...c.endpoints.flatMap(e => [e.label, e.url, e.service])].join(' ').toLowerCase();
    return text.includes(query.toLowerCase());
  });
  const nodeSelect = <div className="node-control"><Server size={15}/><select aria-label="NodePort 접속 노드" value={selectedTarget} disabled={choiceBusy || (account && !favorites.state)} onChange={e => { if (account) stagePersonal(p => { p.defaultTarget = e.target.value; }); else setPrefs(p => ({ ...p, target: e.target.value })); }}>{catalog?.targets.map(t => <option key={t.id} value={t.id}>{t.name}{t.manual ? ' · 도메인' : ''}{t.id === favorites.saved?.profile.defaultTarget ? ' · 기본' : ''}</option>)}</select><ChevronDown size={13}/>{catalog?.identity.admin && <button className="icon-button" aria-label="NodePort 도메인 관리" disabled={opening || choiceBusy} onClick={() => void openEditor({ kind: 'targets' })}><Settings2 size={16}/></button>}</div>;
  const cardView = (card: Card) => {
    const selected = account ? favorites.state?.profile.defaultEndpoints[card.id] : prefs.selected[card.id];
    const endpoint = card.endpoints.find(e => e.id === selected) || card.endpoints[0];
    if (!endpoint) return null;
    const target = endpoint.kind === 'nodeport' ? chooseTarget(selectedTarget, prefs.previous[endpoint.id], endpoint.targets) : undefined;
    const href = endpoint.kind === 'nodeport' ? target?.url : endpoint.url;
    const address = href || target?.address || '';
    const color = ['amber', 'blue', 'green', 'purple', 'pink'][Array.from(card.name).reduce((n, c) => n + c.charCodeAt(0), 0) % 5];
    return <article className={'service-card' + (card.hidden ? ' is-hidden' : '')} key={card.id}>
      <div className="card-controls"><span className={'health-dot ' + endpoint.health.state} title={`${endpoint.health.reason}${endpoint.health.total ? ` · ${endpoint.health.ready}/${endpoint.health.total}` : ''}`} aria-label={endpoint.health.reason}/><div className="card-badges">{(card.new || card.changed) && catalog?.identity.admin && <span className="new-badge">{card.changed ? 'CHANGED' : 'NEW'}</span>}{endpoint.local && <span className="local-badge" title="externalTrafficPolicy: Local">Local</span>}</div><button className={'icon-button favorite' + ((view === 'favorites' ? shownFavorites : allFavorites).includes(card.id) ? ' active' : '')} disabled={choiceBusy || (account && !favorites.state)} onClick={() => favorite(card)} aria-label={catalog?.identity.admin ? `${card.name} 즐겨찾기 모음 선택` : `${card.name} ${allFavorites.includes(card.id) ? '즐겨찾기 해제' : '즐겨찾기 추가'}`} aria-pressed={allFavorites.includes(card.id)}><Star size={16}/></button>{catalog?.identity.admin && <button className="icon-button card-edit" aria-label={`${card.name} 설정`} disabled={opening || choiceBusy} onClick={() => void openEditor({ kind: 'card', card })}><Pencil size={15}/></button>}</div>
      <a className="card-main" href={href || undefined} target={href ? '_blank' : undefined} rel="noopener noreferrer" aria-disabled={!href} onClick={e => { if (!href) { e.preventDefault(); if (address) void copy(address); } }}><span className={'app-icon tone-' + color}><AppIcon icon={card.icon} name={card.name}/></span><div><h3>{card.name}</h3><p title={card.description || card.namespace}>{card.description || card.namespace || '직접 추가한 서비스'}</p></div></a>
      <div className="card-address">{card.endpoints.length > 1 ? <select aria-label={`${card.name} 접속 주소`} value={endpoint.id} disabled={choiceBusy || (account && !favorites.state)} onChange={e => { if (account) stagePersonal(p => { p.defaultEndpoints[card.id] = e.target.value; }); else setPrefs(p => ({ ...p, selected: { ...p.selected, [card.id]: e.target.value } })); }}>{card.endpoints.map(e => <option key={e.id} value={e.id}>{e.label || e.url || e.service}{e.id === favorites.saved?.profile.defaultEndpoints[card.id] ? ' · 기본' : ''}</option>)}</select> : <span title={address}>{endpoint.kind === 'nodeport' ? target ? `${target.name} : ${endpoint.nodePort}` : '사용 가능한 노드 없음' : endpoint.needsURL ? '접속 주소 설정 필요' : endpoint.label || endpoint.url}</span>}{address && <button className="icon-button copy" aria-label={`${card.name} 링크 복사`} onClick={() => void copy(address)}><Copy size={14}/></button>}{href ? <a href={href} target="_blank" rel="noopener noreferrer" className="open-link" aria-label={`${card.name} 열기`}><ArrowUpRight size={17}/></a> : <span className="muted">{endpoint.scheme === 'tcp' && address ? 'TCP' : '—'}</span>}</div>
      {(endpoint.local || endpoint.health.state === 'down' || card.hidden) && <div className="card-note">{endpoint.local && <span>{target ? `${target.name} 사용 중` : 'Ready Pod 없음'}{target && target.targetId !== selectedTarget ? ' · 마지막 사용 가능 노드' : ''}</span>}{!endpoint.local && endpoint.health.state === 'down' && <span className="down-text">준비된 Pod 없음</span>}{card.hidden && <button disabled={choiceBusy} className="text-button" onClick={() => void restore(card)}>다시 표시</button>}</div>}
      {catalog?.identity.admin && target && catalog.targets.some(t => t.id === target.targetId && t.tcpEnabled) && <button className="tcp-button" onClick={async () => { try { const v = await api<{ message: string }>('/api/admin/tcp', { method: 'POST', headers: { 'X-CSRF-Token': catalog.csrf || '' }, body: JSON.stringify({ endpointId: endpoint.id, targetId: target.targetId }) }); notice(v.message + ' · 브라우저 접속 여부와는 별도입니다'); } catch (e) { notice((e as Error).message); } }}>TCP 연결 확인</button>}
    </article>;
  };
  return <div className={'app-shell' + (favorites.dirty ? ' has-default-changes' : '')}>
    {mobile && <div className="sidebar-shade" onClick={() => setMobile(false)}/>}
    <aside className={'sidebar' + (mobile ? ' expanded' : '')}><a className="brand" href="/" onClick={e => { if (normalNavigation(e)) { e.preventDefault(); move('all'); } }}><span className="brand-mark"><Layers3 size={23}/></span><span>KubeShelf<small>YOUR CLUSTER, ORGANIZED</small></span></a><div className="workspace"><span className="workspace-icon"><Boxes size={18}/></span><div><strong>My workspace</strong><small><span className={'tiny-dot ' + (catalog?.connected ? 'healthy' : 'unknown')}/>{catalog?.connected ? 'Kubernetes 연결됨' : '연결 확인 중'}</small></div><ChevronDown size={14}/></div><span className="nav-heading">WORKSPACE</span><nav><a href={viewPaths.favorites} className={view === 'favorites' ? 'active' : ''} aria-current={view === 'favorites' ? 'page' : undefined} onClick={e => { if (normalNavigation(e)) { e.preventDefault(); move('favorites'); } }}><Star size={18}/>즐겨찾기<span>{counts.favorites}</span></a><a href={viewPaths.all} className={view === 'all' ? 'active' : ''} aria-current={view === 'all' ? 'page' : undefined} onClick={e => { if (normalNavigation(e)) { e.preventDefault(); move('all'); } }}><Layers3 size={18}/>모든 서비스<span>{counts.total}</span></a>{catalog?.identity.admin && <><span className="nav-heading">MANAGE</span><a href={viewPaths.discovery} className={view === 'discovery' ? 'active' : ''} aria-current={view === 'discovery' ? 'page' : undefined} onClick={e => { if (normalNavigation(e)) { e.preventDefault(); move('discovery'); } }}><Inbox size={18}/>발견함{counts.discovery > 0 && <span className="nav-badge">{counts.discovery}</span>}</a><a href={viewPaths.namespaces} className={view === 'namespaces' ? 'active' : ''} aria-current={view === 'namespaces' ? 'page' : undefined} onClick={e => { if (normalNavigation(e)) { e.preventDefault(); move('namespaces'); } }}><FolderTree size={18}/>네임스페이스{counts.namespaces > 0 && <span className="nav-badge">{counts.namespaces}</span>}</a><a href={viewPaths.hidden} className={view === 'hidden' ? 'active' : ''} aria-current={view === 'hidden' ? 'page' : undefined} onClick={e => { if (normalNavigation(e)) { e.preventDefault(); move('hidden'); } }}><EyeOff size={18}/>숨긴 서비스<span>{counts.hidden}</span></a></>}</nav><div className="sidebar-bottom"><div className="quiet-note"><span className="tiny-dot healthy"/>A little order for your cluster.</div><div className="profile"><span className="avatar">{catalog?.identity.subject ? (catalog.identity.name || catalog.identity.username).slice(0, 1).toUpperCase() : <Globe size={18}/>}</span><div><strong>{catalog?.identity.subject ? catalog.identity.name || catalog.identity.username : '둘러보는 중'}</strong><small>{catalog?.identity.admin ? '관리자' : catalog?.identity.subject ? '로그인됨' : '공개 서비스'}</small></div>{catalog?.identity.subject ? <button className="icon-button" aria-label="로그아웃" disabled={choiceBusy} onClick={() => void logout()}><LogOut size={17}/></button> : <a className="icon-button" aria-label="SSO 로그인" href={loginURL}><LogIn size={18}/></a>}</div></div></aside>
    <div className="content-shell"><header className="topbar"><button className="icon-button mobile-menu" aria-label="메뉴" onClick={() => setMobile(true)}><Menu size={21}/></button><div className="breadcrumb">Workspace <span>/</span> <strong>{titles[view][0]}</strong></div><div className="top-actions">{catalog?.demo && <span className="demo-badge">DEMO</span>}{catalog?.identity.admin ? <span className="admin-badge"><ShieldCheck size={14}/>관리자</span> : !catalog?.identity.subject && <a className="button secondary small" href={loginURL}><LogIn size={15}/>로그인</a>}<span className="top-avatar">KS</span></div></header>
    <main><div className="page-heading"><div><div className="eyebrow">YOUR PERSONAL APP SHELF</div><h1>{titles[view][0]}<span className="heading-dot">.</span></h1><p>{titles[view][1]}</p></div>{catalog?.identity.admin && <button className="button primary" disabled={opening || choiceBusy} onClick={() => void openEditor({ kind: 'manual' })}>{opening ? <LoaderCircle size={17} className="spin"/> : <Plus size={17}/>}서비스 추가</button>}</div>
      {error && <div className="error-box" role="alert">{error}<button className="text-button" onClick={() => void refresh()}>다시 확인</button></div>}
      {account && favorites.error && !favorites.dirty && <div className="error-box" role="alert">{favorites.error}<button className="text-button" onClick={() => void favorites.load()}>즐겨찾기 다시 불러오기</button></div>}
      {catalog?.problem && <div className="notice-bar warning"><Activity size={17}/>{catalog.problem}</div>}
      {(saving || status?.pending || status?.error) && <div className={'notice-bar ' + (status?.error ? 'warning' : '')} role="status">{status?.error ? <Activity size={17}/> : <LoaderCircle size={17} className="spin"/>}<div><strong>{saving ? '설정을 Git에 저장하고 있어요' : status?.error ? '설정을 적용하지 못했어요' : '저장됨 · 설정 적용 중…'}</strong><span>{status?.error || (saving ? '저장이 끝나면 다시 저장할 수 있습니다.' : 'Fleet이 전달한 설정을 기다리는 중입니다. 계속 편집해도 괜찮아요.')}</span></div>{status?.commit && <code>{status.commit.slice(0, 7)}</code>}</div>}
      <div className="toolbar"><label className="search"><Search size={18}/><input placeholder={view === 'namespaces' ? '네임스페이스 검색…' : '서비스, 도메인, 네임스페이스 검색…'} value={query} onChange={e => setQuery(e.target.value)}/>{query && <button className="icon-button" aria-label="검색 지우기" onClick={() => setQuery('')}><X size={15}/></button>}</label><div className="toolbar-info">{view === 'namespaces' ? `${catalog?.namespaces?.length || 0} namespaces` : `${cards.length} services`}<span className="divider"/><span className="health-dot healthy"/>Ready</div></div>
      {view === 'favorites' && account && favorites.state && <FavoriteBar state={favorites.state} admin={!!catalog?.identity.admin} selected={activeCollection?.id || ''} cards={shownCards} busy={choiceBusy} onSelect={id => { setView('favorites', id); setQuery(''); }} onManage={() => setFavoriteEditor({})} onStart={enabled => stagePersonal(p => { p.startCollection = enabled ? 'favorites' : ''; })}/>}
      {view === 'favorites' && account && !favorites.state && !favorites.error && <div className="notice-bar"><LoaderCircle className="spin" size={17}/>내 즐겨찾기를 불러오고 있어요</div>}
      {view === 'favorites' && catalog && !account && <div className="notice-bar"><Star size={17}/><div><strong>이 브라우저의 즐겨찾기</strong><span>로그인하면 계정에 저장해 다른 기기에서도 사용할 수 있어요.</span></div></div>}
      {view === 'discovery' && counts.namespaces > 0 && <button className="discovery-callout" onClick={() => move('namespaces')}><FolderTree size={20}/><span><strong>아직 공개 범위를 정하지 않은 네임스페이스 {counts.namespaces}개</strong><small>Ingress·NodePort 각각의 기본 공개 범위를 확인하세요.</small></span><ArrowUpRight size={19}/></button>}
      {adminView(view) && !catalog?.identity.admin ? <div className="empty-state"><ShieldCheck size={34}/><h2>{catalog ? '관리자만 볼 수 있는 페이지예요' : '로그인을 확인하고 있어요'}</h2>{catalog && !catalog.identity.subject && <a className="button secondary" href={loginURL}><LogIn size={16}/>SSO 로그인</a>}</div> : view === 'namespaces' ? <div className="namespace-list">{catalog?.namespaces?.filter(n => n.name.toLowerCase().includes(query.toLowerCase())).sort((a, b) => Number(a.configured) - Number(b.configured) || a.name.localeCompare(b.name)).map(n => <NamespaceRow key={n.name} namespace={n} onClick={() => void openEditor({ kind: 'namespace', namespace: n.name })}/>)}</div> : <>{(['ingress', 'nodeport', 'custom'] as const).map(source => { const sectionCards = cards.filter(c => c.source === source); if (!sectionCards.length) return null; const Icon = source === 'ingress' ? Globe : source === 'nodeport' ? Network : ArrowDownToLine; return <section className="service-section" key={source}><header className="section-heading"><div><Icon size={18}/><h2>{source === 'ingress' ? 'Ingress' : source === 'nodeport' ? 'NodePort' : 'Custom'}</h2><span className="section-count">{sectionCards.length}</span><span className="section-description">{source === 'ingress' ? '도메인으로 연결' : source === 'nodeport' ? '노드로 직접 연결' : '직접 모아둔 서비스'}</span></div>{source === 'nodeport' && nodeSelect}</header><div className="card-grid">{sectionCards.map(cardView)}</div></section>; })}{!cards.length && <div className="empty-state">{!catalog ? <LoaderCircle size={34} className="spin"/> : view === 'favorites' ? <Star size={34}/> : view === 'discovery' ? <Check size={34}/> : <Layers3 size={34}/>}<h2>{!catalog ? '서비스를 불러오고 있어요' : query ? '검색 결과가 없어요' : view === 'favorites' ? '자주 찾는 서비스를 별표로 모아보세요' : view === 'discovery' ? '모두 확인했어요' : view === 'hidden' ? '숨긴 서비스가 없어요' : '아직 표시할 서비스가 없어요'}</h2><p>{!catalog ? '잠시만 기다려 주세요.' : query ? '다른 이름이나 도메인으로 찾아보세요.' : view === 'favorites' ? (catalog.identity.admin ? '모든 서비스에서 별을 눌러 이 모음에 담아보세요.' : '모든 서비스에서 별을 눌러 즐겨찾기를 추가하세요.') : catalog.identity.admin ? '네임스페이스 공개 범위를 확인하거나 서비스를 직접 추가해 보세요.' : '로그인하면 사용할 수 있는 서비스가 더 있을 수 있어요.'}</p>{catalog && !catalog.identity.subject && <a className="button secondary" href={loginURL}><LogIn size={16}/>SSO 로그인</a>}</div>}</>}
      <footer className="page-footer"><span><Layers3 size={13}/>KubeShelf <span className="muted">{catalog?.version || 'dev'}</span></span><span>서비스 상태는 Kubernetes의 Pod Ready 기준입니다.</span></footer>
    </main></div>
    {editor && admin && <Editor key={editor.kind === 'card' ? editor.card.id : editor.kind === 'namespace' ? editor.namespace : editor.kind} request={editor} data={admin} saving={choiceBusy} onSave={save} onClose={() => setEditor(undefined)}/>}
    {favoriteEditor && catalog?.identity.admin && favorites.state && <FavoriteEditor key={favoriteEditor.card?.id || 'collections'} state={favorites.state} card={favoriteEditor.card} busy={choiceBusy} onSave={favorites.stage} onClose={() => setFavoriteEditor(undefined)} onReload={() => { setFavoriteEditor(undefined); void favorites.reload(); }}/>}
    {account && favorites.dirty && <aside className="defaults-bar personal-save-bar" aria-label="개인 설정 저장"><div><strong>{favorites.favoriteChanges && favorites.addressChanges ? '즐겨찾기와 기본 주소 변경' : favorites.addressChanges ? '내 기본 접속 주소 변경' : '즐겨찾기 변경사항'}</strong><p>저장하면 내 계정의 다른 기기에서도 사용합니다.</p>{favorites.error && <p className="defaults-error" role="alert">{favorites.error}{favorites.error.includes('다른 기기') && <button className="text-button" disabled={choiceBusy} onClick={() => { if (window.confirm('저장하지 않은 변경사항을 버리고 최신 개인 설정을 불러올까요?')) void favorites.reload(); }}>최신 설정 불러오기</button>}</p>}</div><div className="defaults-actions"><button className="button secondary" disabled={choiceBusy} onClick={favorites.reset}>되돌리기</button><button className="button primary" disabled={choiceBusy} onClick={async () => { if ((await favorites.persist())?.ok) notice('개인 설정을 저장했어요'); }}>{favorites.saving ? <LoaderCircle size={16} className="spin"/> : <Save size={16}/>} {favorites.saving ? '저장 중…' : favorites.favoriteChanges && favorites.addressChanges ? '즐겨찾기·기본 주소 저장' : favorites.addressChanges ? '기본값으로 저장' : '즐겨찾기 저장'}</button></div></aside>}
    {toast && <div role="status" className="toast"><Check size={17}/>{toast}</div>}
  </div>;
}
