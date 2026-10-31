import type { Connection } from './types';
// Every Local endpoint remembers its own last usable target. The selector remains section-wide.
export function chooseTarget(selected: string, previous: string | undefined, available: Connection[]): Connection | undefined {
  return available.find(t => t.targetId === selected)
    ?? available.find(t => t.targetId === previous)
    ?? [...available].sort((a, b) => a.name.localeCompare(b.name, undefined, { numeric: true }))[0];
}
export function policyLabel(p: { mode: string; groups?: string[]; users?: string[] } | undefined) {
  if (!p) return '네임스페이스 설정 따름';
  if (p.mode === 'public') return '누구나 · 로그인 없이';
  if (p.mode === 'admin') return '관리자만';
  return [...(p.groups || []), ...(p.users?.length ? [`사용자 ${p.users.length}명`] : [])].join(', ') || '허용 대상 없음';
}
