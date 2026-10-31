import type { DetailedHTMLProps, HTMLAttributes, ReactNode, Ref } from 'react';
import navigation from './cola16-navigation.json';

export const colaBrand = document.documentElement.dataset.brand === 'cola16';

type ColaHeaderProps = DetailedHTMLProps<HTMLAttributes<HTMLElement>, HTMLElement> & { current?: string; subtitle?: string; home?: string };
declare module 'react' { namespace JSX { interface IntrinsicElements { 'cola-header': ColaHeaderProps } } }

// Shared cola16 header (/branding/v1/cola-header.js). The element renders its own
// shadow tree; React owns only the no-script fallback and the account slot.
export function BrandHeader({ ref, inert, children }: { ref: Ref<HTMLElement>; inert: boolean; children: ReactNode }) {
  return <cola-header ref={ref} current="tools" subtitle="도구" home="https://apps.cola16.app/" inert={inert}>
    <a className="ch-skip" href="#main">본문 바로가기</a>
    <header className="ch-bar"><a className="ch-brand" href="https://apps.cola16.app/" aria-label="cola16 도구 홈">cola16 <small>도구</small></a><nav className="ch-links" aria-label="cola16의 공간">{navigation.items.map(item => <a key={item.id} href={item.href} aria-current={item.id === 'tools' ? 'page' : undefined}>{item.label}</a>)}</nav></header>
    <div slot="account" className="cola-account">{children}</div>
  </cola-header>;
}
