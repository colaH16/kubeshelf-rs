import { useEffect, useState } from 'react';
export const colaBrand = document.documentElement.dataset.brand === 'cola16';
export function BrandControls() {
  const [theme, setTheme] = useState(document.documentElement.dataset.colaPreference || 'auto');
  const [message, setMessage] = useState('');
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
  return <><label className="brand-theme"><span>테마</span><select aria-label="화면 테마" value={theme} onChange={e => { setTheme(e.target.value); try { localStorage.setItem('cola-theme', e.target.value); setMessage(''); } catch { setMessage('테마는 적용했지만 선택을 저장하지 못했습니다.'); } }}><option value="auto">자동</option><option value="light">라이트</option><option value="dark">다크</option></select></label><span className="brand-storage-status" role="status">{message}</span></>;
}
