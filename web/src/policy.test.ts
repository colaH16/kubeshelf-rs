import { describe, expect, it } from 'vitest';
import { inheritedOptionLabel, inheritedPolicy, policySubjects, servicePolicyParents, servicePolicyState } from './policy';
import type { Settings } from './types';

const settings = (): Settings => ({
  schemaVersion: 1, revision: '',
  namespaces: { public: { mode: 'public' }, private: { mode: 'restricted', groups: ['operators'] } },
  nodePortNamespaces: { public: { mode: 'admin' } },
  apps: {}, manual: [], targets: [], assignments: {},
});
const endpoint = { appId: 'original', kind: 'ingress', namespace: 'public', service: 'cloud' };

describe('inherited access shown in the editor', () => {
  it('keeps Ingress and NodePort defaults independent in the same namespace', () => {
    const s = settings();
    expect(inheritedPolicy(s, endpoint).policy.mode).toBe('public');
    expect(inheritedPolicy(s, { ...endpoint, kind: 'nodeport' })).toMatchObject({ source: 'public · NodePort', policy: { mode: 'admin' } });
    delete s.nodePortNamespaces.public;
    expect(inheritedPolicy(s, { ...endpoint, kind: 'nodeport' })).toMatchObject({ policy: { mode: 'admin' }, defaulted: true });
  });
  it('uses the original service draft as the address parent, never an endpoint or presentation override', () => {
    const s = settings();
    s.assignments.url = 'presentation';
    s.apps.presentation = { hidden: false, visibility: { mode: 'public' } };
    s.apps.original = { hidden: false, visibility: { mode: 'restricted', groups: ['operators'] }, endpoints: { url: { visibility: { mode: 'admin' } } } };
    expect(inheritedPolicy(s, endpoint, 'address')).toMatchObject({ source: '서비스 설정 · public · cloud', policy: { mode: 'restricted', groups: ['operators'] } });
    expect(inheritedPolicy(s, endpoint, 'service').policy.mode).toBe('public');
    s.apps.original.visibility = { mode: 'admin' };
    expect(inheritedPolicy(s, endpoint, 'address').policy.mode).toBe('admin');
    delete s.apps.original.visibility;
    expect(inheritedPolicy(s, endpoint, 'address').policy.mode).toBe('public');
  });
  it('explains namespace inheritance separately for merged addresses and deduplicates aliases', () => {
    const s = settings();
    const parents = servicePolicyParents(s, [endpoint, endpoint, { ...endpoint, appId: 'other', namespace: 'private' }]);
    expect(parents.map(p => p.source)).toEqual(['public · Ingress', 'private · Ingress']);
    expect(inheritedOptionLabel(parents)).toContain('주소마다 다름');
    expect(inheritedOptionLabel(servicePolicyParents(s, [endpoint, endpoint]))).toContain('누구나');
  });
  it('does not misrepresent mixed service overrides as one inherited policy', () => {
    const s = settings();
    s.apps.second = { hidden: false, visibility: { mode: 'public' } };
    expect(servicePolicyState(s, ['original', 'second']).mixed).toBe(true);
    s.apps.original = { hidden: false, visibility: { mode: 'public' } };
    expect(servicePolicyState(s, ['original', 'second']).mixed).toBe(false);
    s.apps.original.visibility = { mode: 'restricted', groups: ['a', 'b'], users: ['u'] };
    s.apps.second.visibility = { mode: 'restricted', groups: ['b', 'a'], users: ['u'] };
    expect(servicePolicyState(s, ['original', 'second']).mixed).toBe(false);
    s.apps.second.visibility.users = ['someone-else'];
    expect(servicePolicyState(s, ['original', 'second']).mixed).toBe(true);
  });
  it('shows the admin fallback for unconfigured namespaces and manual services', () => {
    expect(inheritedPolicy(settings(), { ...endpoint, namespace: 'new' })).toMatchObject({ policy: { mode: 'admin' }, defaulted: true });
    expect(servicePolicyParents(settings(), [])).toEqual([{ source: '기본 설정', policy: { mode: 'admin' } }]);
  });
  it('resolves user subjects to readable names without hiding unknown subjects or groups', () => {
    const policy = { mode: 'restricted' as const, groups: ['operators', 'removed-group'], users: ['uuid-1', 'unknown-uuid'] };
    expect(policySubjects(policy, { groups: ['operators'], users: [{ subject: 'uuid-1', username: 'alice', name: 'Alice', groups: [] }] })).toEqual([
      '그룹: operators, removed-group', '사용자: Alice (alice), unknown-uuid',
    ]);
    expect(inheritedOptionLabel([{ source: 'private', policy: { mode: 'restricted' } }])).toContain('관리자만 · 선택한 대상 없음');
  });
});
