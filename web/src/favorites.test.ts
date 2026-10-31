import { expect, it } from 'vitest';
import { assignFavorite, favoriteIDs } from './favorites';
import type { FavoriteProfile } from './types';

it('assigns one service to several collections while preserving defaults and other services', () => {
 const profile: FavoriteProfile = { collections: [{ id:'daily',name:'일상',cards:['photos'] }, { id:'ops',name:'관리',cards:['rancher'] }], startCollection:'daily', defaultTarget:'node3', defaultEndpoints:{ cloud:'personal-cloud' } };
 const next = assignFavorite(profile,'cloud',['daily','ops']);
 expect(next.collections.map(c => c.cards)).toEqual([['photos','cloud'],['rancher','cloud']]);
 expect(favoriteIDs(next)).toEqual(['photos','cloud','rancher']);
 expect(assignFavorite(next,'cloud',['ops']).collections[0].cards).toEqual(['photos']);
 expect(profile.collections[0].cards).toEqual(['photos']);
 expect(next.defaultEndpoints).toEqual(profile.defaultEndpoints);
 expect(next.defaultTarget).toBe('node3');
 expect(next.startCollection).toBe('daily');
});
