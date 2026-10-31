import { describe, expect, it } from 'vitest';
import { chooseTarget } from './selection';
const nodes = (ids: number[]) => ids.map(n => ({ targetId: `node${n}`, name: `node${n}`, url: `http://192.0.2.${n}:30000`, address: `192.0.2.${n}:30000` }));
describe('Local NodePort section selection', () => {
  it('remembers the last eligible node for the exact requested sequence', () => {
    const available = nodes([2, 3, 5]);
    let previous: string | undefined;
    const actual = ['node2', 'node1', 'node3', 'node4'].map(selected => { previous = chooseTarget(selected, previous, available)?.targetId; return previous; });
    expect(actual).toEqual(['node2', 'node2', 'node3', 'node3']);
  });
  it('falls back if the previous node loses its Ready endpoint and disables when none remain', () => {
    expect(chooseTarget('node4', 'node3', nodes([2, 5]))?.targetId).toBe('node2');
    expect(chooseTarget('node4', 'node3', [])).toBeUndefined();
  });
  it('follows the section again when its selected node becomes eligible', () => {
    expect(chooseTarget('node4', 'node3', nodes([3, 4, 5]))?.targetId).toBe('node4');
  });
  it('uses natural node-name order for initial fallback', () => {
    expect(chooseTarget('missing', undefined, nodes([10, 2, 5]))?.targetId).toBe('node2');
  });
});
