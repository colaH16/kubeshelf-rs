import type { FavoriteProfile } from './types';

export const favoriteIDs = (p?: FavoriteProfile) => [...new Set(p?.collections.flatMap(c => c.cards) || [])];
export function assignFavorite(profile: FavoriteProfile, card: string, collections: string[]) {
 const next = structuredClone(profile);
 for (const c of next.collections) c.cards = collections.includes(c.id) ? [...new Set([...c.cards, card])] : c.cards.filter(id => id !== card);
 return next;
}
