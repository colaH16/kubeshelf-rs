import { useCallback, useEffect, useRef, useState } from 'react';
import { api } from './api';
import type { FavoriteProfile, FavoriteState } from './types';

type Draft = FavoriteState & { owner: string; original: string };
export function useFavorites(subject: string, csrf: string, blocked: boolean) {
 const [saved, setSaved] = useState<{ owner: string; data: FavoriteState }>();
 const [draft, setDraft] = useState<Draft>();
 const [error, setError] = useState('');
 const [saving, setSaving] = useState(false);
 const owner = useRef(subject); owner.current = subject;
 const draftRef = useRef(draft); draftRef.current = draft;
 const sequence = useRef(0), lock = useRef(false);
 const state = draft?.owner === subject ? draft : saved?.owner === subject ? saved.data : undefined;
 const dirty = draft?.owner === subject && JSON.stringify(draft.profile) !== draft.original;
 const load = useCallback(async () => {
  if (!subject || lock.current) return;
  const request = ++sequence.current;
  try {
   const data = await api<FavoriteState>('/api/me/favorites');
   if (owner.current === subject && sequence.current === request) { setSaved({ owner: subject, data }); if (!draftRef.current) setError(''); }
  } catch (e) { if (owner.current === subject && sequence.current === request) setError((e as Error).message); }
 }, [subject]);
 useEffect(() => {
  setSaved(undefined); setDraft(undefined); setError(''); ++sequence.current;
  void load();
  const timer = setInterval(() => void load(), 10000);
  const focus = () => void load();
  window.addEventListener('focus', focus);
  return () => { ++sequence.current; clearInterval(timer); window.removeEventListener('focus', focus); };
 }, [load]);
 useEffect(() => {
  if (!dirty) return;
  const warn = (e: BeforeUnloadEvent) => { e.preventDefault(); e.returnValue = ''; };
  window.addEventListener('beforeunload', warn);
  return () => window.removeEventListener('beforeunload', warn);
 }, [dirty]);
 const stage = (profile: FavoriteProfile, version = state?.version || '') => {
  if (!subject || !state || lock.current || blocked) throw new Error('다른 저장이 끝날 때까지 기다려 주세요');
  const original = draft?.owner === subject ? draft.original : JSON.stringify(state.profile);
  setDraft(JSON.stringify(profile) === original ? undefined : { owner: subject, profile: structuredClone(profile), version, original });
  setError('');
 };
 const persist = async () => {
  const value = draft;
  if (!subject || !value || !dirty || lock.current || blocked) return;
  lock.current = true; ++sequence.current; setSaving(true); setError('');
  try {
   const data = await api<FavoriteState>('/api/me/favorites', { method: 'PUT', headers: { 'X-CSRF-Token': csrf }, body: JSON.stringify({ profile: value.profile, version: value.version }) });
   if (owner.current !== subject) return;
   setSaved({ owner: subject, data }); setDraft(undefined); return { ok: true };
  } catch (e) {
   const message = (e as Error).message;
   if (owner.current === subject) {
    setError(message);
   }
   return { ok: false, error: message };
  }
  finally { lock.current = false; setSaving(false); }
 };
 const reset = () => { if (!lock.current && !blocked) { setDraft(undefined); setError(''); } };
 const reload = async () => { if (!lock.current && !blocked) { reset(); await load(); } };
 const before: FavoriteProfile | undefined = draft ? JSON.parse(draft.original) : undefined;
 const addressChanges = !!draft && (draft.profile.defaultTarget !== before?.defaultTarget || JSON.stringify(draft.profile.defaultEndpoints) !== JSON.stringify(before?.defaultEndpoints));
 const favoriteChanges = !!draft && (JSON.stringify(draft.profile.collections) !== JSON.stringify(before?.collections) || draft.profile.startCollection !== before?.startCollection);
 return { state, saved: saved?.owner === subject ? saved.data : undefined, dirty, addressChanges, favoriteChanges, saving, error, stage, persist, load, reset, reload };
}
