# cola16 공통 메뉴

## 원본과 배포

편집할 원본은 이 저장소의 `web/src/cola16-navigation.json`입니다. `version:1`과 `items`를 제공하며 각 항목은 고정 식별자 `id`, 표시 이름 `label`, HTTPS 목적지 `href`입니다. 식별자는 `blog`, `portfolio`, `tools`, `notes`, `graph`이고 순서는 배열 순서입니다. 현재 페이지 표시는 각 앱이 자신의 식별자로 결정합니다.

`npm --prefix web run build`가 같은 원본에서 번들 기본값과 `web/public/branding/navigation.json`을 만듭니다. 생성된 공개 복사본은 편집하거나 Git에 추가하지 않습니다. 원본 수정 후 KubeShelf CI로 이미지를 만들고 제공 앱의 GitOps image tag/digest를 갱신하면, 연결된 소비 앱은 다음 페이지 로드에서 새 설정을 받습니다. **이미 연결된 앱은 메뉴 데이터 변경 때문에 다시 빌드·배포할 필요가 없습니다.** 이 배포 자동 갱신 자체를 새로 도입한 것은 아니며 기존 이미지 검수·GitOps 절차를 사용합니다.

## 공개 읽기 주소

`GET https://apps.cola16.app/branding/navigation.json`

이 파일에 한해서 GET/HEAD에 `Access-Control-Allow-Origin: *`를 제공합니다. 공개 링크만 담고 계정·서비스 목록·세션 정보는 없습니다. 로그인이나 쿠키 없이 읽습니다. 모든 기존 API/auth 응답의 CORS·CSRF·보안 헤더는 유지합니다. 기존 no-store 정책을 유지하므로 다음 페이지를 열 때 새 정의를 조회합니다.

```js
const response = await fetch('https://apps.cola16.app/branding/navigation.json', {
  credentials: 'omit',
  signal: controller.signal,
});
```

소비 앱은 3초 이내에 읽기를 끝내고 버전·배열·HTTPS 링크를 검증합니다. 예제 구현은 `web/src/navigation.ts`, `BrandNavigation.tsx`입니다. 실패·잘못된 응답은 기본 링크를 유지합니다. 즉시 보이는 기본 링크를 함께 제공해 KubeShelf 접속 장애가 다른 사이트의 기본 이동 메뉴를 없애지 않게 합니다. DOM에서는 링크 텍스트를 textContent 또는 프레임워크의 기본 escaping으로 렌더링합니다. 공통 메뉴와 앱 내부 메뉴의 aria-label을 구별합니다.

iframe이나 별도 메뉴 마이크로서비스는 쓰지 않습니다. 헤더 마크업·반응형 배치·현재 페이지·테마·로그인은 각 앱이 담당하고, 이 설정은 메뉴 이름·순서·목적지만 공유합니다. 로그인 상태나 테마 저장소를 출처 간 자동 공유하는 기능은 아닙니다.

## 후속 연결 범위

현재 이 계약의 소비자는 KubeShelf입니다. WordPress·포트폴리오의 기존 링크 정의는 이번 작업에서 바꾸지 않았고, RAG 소스·색인·배포도 건드리지 않았습니다. 사용자가 다른 세션에서 RAG를 작업 중이므로 나중에 노트 검색·지도 모두 이 주소에 연결합니다. 기존 앱을 최초 한 번 연결해야 메뉴 데이터가 중앙 관리됩니다. 헤더 HTML을 각각 복사해 수정하는 것으로 연결을 대신하지 않습니다.

RAG 후속 화면: 맨 위 왼쪽 로고, 가운데 이 공통 메뉴, 오른쪽 테마(자동/라이트/다크)와 로그인. 중복인 지식 지도 링크를 제거하고 PUBLIC LIBRARY는 헤더 아래 본문 쪽으로 옮깁니다. CSP의 connect-src에 `https://apps.cola16.app`만 추가하고 credentials:omit으로 읽습니다. 기존 검색·지도 기능과 로그인 경계는 유지합니다. WordPress·포트폴리오도 이 주소를 최초 연결하면 이후 메뉴 내용 변경을 앱별로 반복할 필요가 없습니다.
