import { useId } from 'react';
import type { DisplayMode, Endpoint, Settings } from './types';
import { namespaceDisplay } from './display';

export default function DisplayEditor({ value, settings, endpoints, onChange }: {
  value: DisplayMode | 'mixed'; settings: Settings; endpoints: Endpoint[]; onChange: (mode: DisplayMode) => void;
}) {
  const name = useId();
  const parents = (endpoints.length ? endpoints.map(e => namespaceDisplay(settings, e)) : [namespaceDisplay(settings)])
    .filter((p, i, all) => all.findIndex(other => other.source === p.source) === i);
  return <fieldset className="display-editor"><legend>대시보드 표시</legend>
    <div className="display-options">{([['inherit', '상속 받기'], ['show', '표시하기'], ['hide', '숨기기']] as const).map(([mode, label]) => <label key={mode} className={value === mode ? 'selected' : ''}><input type="radio" name={name} value={mode} checked={value === mode} onChange={() => onChange(mode)}/>{label}</label>)}</div>
    {value === 'inherit' && <div className="display-inheritance">{parents.map(p => <p key={p.source}>{p.source} · 기본 <strong>{p.hidden ? '숨김' : '표시'}</strong></p>)}</div>}
    {value === 'mixed' && <p className="muted">주소마다 표시 설정이 다릅니다. 선택하면 이 서비스에 함께 적용됩니다.</p>}
    <p className="muted display-access-note">표시하기를 선택해도 아래 공개 범위는 적용됩니다.</p>
  </fieldset>;
}
