import { useEffect, useRef, useState } from 'react';
export const colaBrand = document.documentElement.dataset.brand === 'cola16';
export function BrandControls() {
  const [theme, setTheme] = useState(document.documentElement.dataset.colaPreference || 'auto');
  const [message, setMessage] = useState('');
  const menu = useRef<HTMLDetailsElement>(null);
  useEffect(() => { const close = (e: MouseEvent) => { if (menu.current && !menu.current.contains(e.target as Node)) menu.current.open = false; }; document.addEventListener('click', close); return () => document.removeEventListener('click', close); }, []);
  useEffect(() => {
    if (!colaBrand) return;
    const media = matchMedia('(prefers-color-scheme:dark)');
    const apply = () => { const value = theme === 'auto' ? (media.matches ? 'dark' : 'light') : theme; document.documentElement.dataset.colaTheme = value; document.querySelector('meta[name="theme-color"]')?.setAttribute('content', value === 'dark' ? '#201B20' : '#F4F0F1'); };
    document.documentElement.dataset.colaPreference = theme;
    apply(); media.addEventListener('change', apply); return () => media.removeEventListener('change', apply);
  }, [theme]);
  useEffect(() => {
    if (!colaBrand) return;
    const sync = (e: StorageEvent) => { if (e.key === 'cola-theme') setTheme(['auto','light','dark'].includes(e.newValue || '') ? e.newValue! : 'auto'); };
    window.addEventListener('storage', sync); return () => window.removeEventListener('storage', sync);
  }, []);
  if (!colaBrand) return null;
  return <><label className="brand-theme"><span>테마</span><select aria-label="화면 테마" value={theme} onChange={e => { setTheme(e.target.value); try { localStorage.setItem('cola-theme', e.target.value); setMessage(''); } catch { setMessage('테마는 적용했지만 선택을 저장하지 못했습니다.'); } }}><option value="auto">자동</option><option value="light">라이트</option><option value="dark">다크</option></select></label><details ref={menu} className="creator-menu" onBlur={e => { if (e.relatedTarget && !e.currentTarget.contains(e.relatedTarget as Node)) e.currentTarget.open = false; }} onKeyDown={e => { if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); e.currentTarget.open = false; e.currentTarget.querySelector('summary')?.focus(); } }}><summary>cola16의 공간</summary><nav aria-label="cola16의 공간"><a href="https://cola16.app/">기록</a><a href="https://pf.cola16.app/">포트폴리오</a><a href="/" aria-current="page">도구 · KubeShelf</a><a href="https://rag.cola16.app/">노트 검색</a><a href="https://rag.cola16.app/graph">노트 지도</a></nav></details><span className="brand-storage-status" role="status">{message}</span></>;
}
