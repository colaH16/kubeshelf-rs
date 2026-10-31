import type { Directory, Endpoint, Policy, Settings } from './types';

export type PolicySource = { source: string; policy: Policy; defaulted?: boolean };
type PolicyEndpoint = Pick<Endpoint, 'appId' | 'namespace' | 'kind' | 'service'>;

// Match endpointPolicy in internal/shelf/catalog.go. Presentation cards never
// determine access: every endpoint keeps its original service and namespace.
export function inheritedPolicy(settings: Settings, endpoint?: PolicyEndpoint, level: 'service' | 'address' = 'service'): PolicySource {
  if (endpoint && level === 'address') {
    const app = settings.apps[endpoint.appId];
    if (app?.visibility) return {
      source: ['서비스 설정', endpoint.namespace, app.name || endpoint.service || endpoint.appId].filter(Boolean).join(' · '),
      policy: app.visibility,
    };
  }
  if (endpoint?.namespace) {
    const nodePort = endpoint.kind === 'nodeport';
    const policy = (nodePort ? settings.nodePortNamespaces : settings.namespaces)?.[endpoint.namespace];
    return {
      source: `${endpoint.namespace} · ${nodePort ? 'NodePort' : 'Ingress'}`,
      policy: policy || { mode: 'admin' },
      defaulted: !policy,
    };
  }
  return { source: '기본 설정', policy: { mode: 'admin' } };
}

function policyKey(policy?: Policy) {
  if (!policy || policy.mode !== 'restricted') return policy?.mode || 'inherit';
  return JSON.stringify([policy.mode, [...new Set(policy.groups || [])].sort(), [...new Set(policy.users || [])].sort()]);
}

export function servicePolicyState(settings: Settings, appIDs: string[]) {
  const value = settings.apps[appIDs[0]]?.visibility;
  return { value, mixed: appIDs.some(id => policyKey(settings.apps[id]?.visibility) !== policyKey(value)) };
}

export function servicePolicyParents(settings: Settings, endpoints: PolicyEndpoint[]): PolicySource[] {
  const parents = endpoints.length ? endpoints.map(e => inheritedPolicy(settings, e)) : [inheritedPolicy(settings)];
  return parents.filter((p, i) => parents.findIndex(other => other.source === p.source && policyKey(other.policy) === policyKey(p.policy)) === i);
}

export function policyModeLabel(policy: Policy) {
  if (policy.mode === 'public') return '누구나 · 로그인 없이';
  if (policy.mode === 'admin') return '관리자만';
  return (policy.groups?.length || policy.users?.length) ? '선택한 그룹 또는 사용자' : '관리자만 · 선택한 대상 없음';
}

export function inheritedOptionLabel(parents: PolicySource[]) {
  if (!parents.length) return '상위 설정 따름';
  const same = parents.every(p => policyKey(p.policy) === policyKey(parents[0].policy));
  return `상위 설정 따름 · ${same ? policyModeLabel(parents[0].policy) : '주소마다 다름'}`;
}

export function policySubjects(policy: Policy, directory: Directory): string[] {
  if (policy.mode !== 'restricted') return [];
  return [
    ...(policy.groups?.length ? [`그룹: ${policy.groups.join(', ')}`] : []),
    ...(policy.users?.length ? [`사용자: ${policy.users.map(subject => {
      const user = directory.users.find(u => u.subject === subject);
      if (!user) return subject;
      return user.name && user.username && user.name !== user.username ? `${user.name} (${user.username})` : user.name || user.username || subject;
    }).join(', ')}`] : []),
  ];
}
