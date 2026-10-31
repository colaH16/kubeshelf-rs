# cola16 공통 헤더와 메뉴

## 원본과 배포

| 원본 (이 저장소) | 공개 주소 (`https://apps.cola16.app`) | 내용 |
|---|---|---|
| `web/src/cola16-navigation.json` | `/branding/navigation.json` | 메뉴 데이터: `version:1`, `items[{id,label,href}]` |
| `web/src/cola-header.js` | `/branding/v1/cola-header.js` | `<cola-header>` 요소. CSS와 메뉴 기본값을 빌드 때 포함 |
| `web/src/cola-header.css` | `/branding/v1/cola-header.css` | 같은 CSS를 `cola-header:not(:defined)`로 범위 지정한 대체 표시용 사본 |

`npm --prefix web run build`의 prebuild(`web/scripts/build-plum.cjs`)가 원본에서 공개 파일을 만듭니다. 생성 파일은 편집하거나 Git에 추가하지 않습니다. 식별자는 `blog`, `portfolio`, `tools`, `notes`, `graph`이고 순서는 배열 순서입니다.

배포는 기존 절차 그대로입니다. 원본 수정 → KubeShelf CI 이미지 → GitOps의 이미지 갱신. 메뉴 이름·순서·주소와 v1 안의 호환 UI 변경은 이것만으로 끝납니다. JavaScript가 동작하는 소비 사이트는 스크립트에 함께 들어 있는 CSS·기본 메뉴와 최신 JSON을 쓰므로, 다시 배포하지 않아도 공개 캐시(최대 5분)가 지난 뒤 다음 페이지 로드에서 바뀝니다. 즉시 반영되지 않으며, 소비 사이트의 이미지를 자동으로 바꾸지도 않습니다.

소비 사이트가 가진 것은 JavaScript가 없거나 공급자가 응답하지 않을 때만 보이는 대체 링크와 대체 CSS의 **마지막 빌드 시점 사본**입니다. 이 사본은 바로 따라오지 않으며, 맞추는 일은 선택입니다(아래 "대체 표시"). v1 안에서는 속성·slot·대체 마크업 클래스를 호환되게 유지합니다. 깨지는 변경은 `/branding/v2/`에 새로 두고, 각 소비 사이트를 옮기는 작업이 따로 필요합니다.

## 사이트에 연결하기

```html
<style>/* /branding/v1/cola-header.css의 사본을 로컬 CSS나 인라인 <style>로 둡니다. */</style>
<script src="https://apps.cola16.app/branding/v1/cola-header.js" async></script>

<cola-header current="blog" subtitle="기록" home="https://cola16.app/">
  <!-- JavaScript 없이도 보이는 기본 링크. 요소가 정의되면 렌더링되지 않습니다. -->
  <a class="ch-skip" href="#main">본문 바로가기</a>
  <header class="ch-bar">
    <a class="ch-brand" href="https://cola16.app/" aria-label="cola16 기록 홈">cola16 <small>기록</small></a>
    <nav class="ch-links" aria-label="cola16의 공간"><a href="https://cola16.app/" aria-current="page">기록</a>…</nav>
  </header>
  <!-- 선택: 사이트가 소유하는 로그인/로그아웃. 래퍼 없이 요소의 직접 자식으로 둡니다. -->
  <a id="login" slot="account" href="/auth/login" hidden>로그인</a>
  <button id="logout" slot="account" type="button" hidden>로그아웃</button>
</cola-header>
```

- 다른 출처의 CSS를 `<link rel="stylesheet">`로 걸지 않습니다. 렌더링을 막아 KubeShelf가 느리거나 멈추면 소비 사이트의 첫 화면도 멈춥니다. 대체 CSS는 로컬 사본으로 두고, 스크립트는 `async`로 읽어 첫 화면과 `DOMContentLoaded`를 기다리게 하지 않습니다. 대체 CSS는 `cola-header:not(:defined)`에만 적용되므로, 요소가 정의된 뒤에는 오래된 사본이 새 스크립트의 Shadow DOM 스타일을 덮지 않습니다. KubeShelf 자신은 공급자와 같은 출처라 그대로 `<link>`와 `defer`를 씁니다.
- 요소는 Shadow DOM 안에 직접 그리고 light DOM을 바꾸지 않습니다. React 같은 프레임워크가 기본 링크와 `slot="account"`를 계속 소유합니다. 같은 링크가 두 번 보이거나 키보드 순서에 두 번 들어가지 않습니다.
- 계정 영역 `slot="account"`은 Shadow DOM에서 테마 버튼 **앞**에 있어 화면 순서와 키보드 순서가 같습니다(로그인/로그아웃 → 테마). 테마 버튼은 줄바꿈 없이 항상 오른쪽 끝(모바일 16px, 데스크톱 1440px 안 32px)에 있어 로그인 여부와 사이트에 관계없이 같은 자리입니다.
  - 로그인/로그아웃은 `slot="account"`을 단 `<a>` 또는 `<button>` **직접 자식**으로 둡니다. 공통 CSS가 한 가지 모양(14px·600, 높이 44px, 좌우 12px, `--cola-border` 1px 테두리, 4px 모서리, `--cola-surface` 배경, 밑줄 없음, 공통 포커스·hover·disabled)을 `!important`로 정하므로 사이트 CSS의 버튼·링크 규칙이 덮지 못합니다. 사이트는 이 요소에 버튼 클래스·아이콘·스타일을 붙이지 않습니다. 문구는 `로그인`/`로그아웃`입니다.
  - `hidden`은 항상 숨김입니다. 세션 확인 전에는 둘 다 `hidden`으로 두고 사이트 JS가 하나만 보입니다. id·href(`next` 포함)·이벤트 리스너는 사이트 것이고 공통 코드는 light DOM을 옮기거나 바꾸지 않습니다.
  - 다른 자식(배지 묶음, v1의 `<div slot="account">` 래퍼)도 계속 받습니다. 줄로 배치만 하고 모양은 사이트가 정합니다. 래퍼 안의 링크·버튼에는 공통 버튼 모양이 적용되지 않습니다. KubeShelf는 DEMO·관리자 배지를 `.cola-badges` 묶음으로 따로 넣고(모바일에서는 숨김), 로그인/로그아웃은 직접 자식입니다.
  - 대체 표시(요소 정의 전·JS 없음)에서도 직접 자식 로그인/로그아웃은 같은 모양으로 헤더 첫 줄 오른쪽 끝에 놓여 헤더 높이가 바뀌지 않습니다. 테마 버튼은 JS가 필요해 대체 표시에는 없습니다.
- 글꼴(시스템 글꼴 스택), 크기, 간격, 44px 조작 영역, 최대 폭 1440px, 모바일 배치는 공통 CSS가 정합니다. 사이트 CSS는 헤더 내부에 적용되지 않고 색 토큰 `--cola-*`만 이어받습니다. 고정(sticky) 여부 같은 배치는 사이트가 `cola-header`에 지정합니다.
- 메뉴는 내장 기본값으로 즉시 그린 뒤 `/branding/navigation.json`을 `credentials:'omit'`, 3초 제한으로 읽습니다. 버전·배열·HTTPS 링크를 모두 검증하고 실패하면 기본값을 유지합니다. 텍스트는 `textContent`로만 넣습니다.
- 본문 바로가기는 Shadow DOM 안에 있어 일반 `#main` 이동으로는 포커스가 사이트의 `<body>`에 남습니다. 그래서 요소가 사이트 문서의 `#main`에 직접 포커스를 주고, 고정(sticky/fixed) 헤더와 사이트의 `scroll-padding-top` 중 큰 값만큼 아래에 맞춰 스크롤합니다. 포커스할 수 없는 `<main>`에는 임시로 `tabindex="-1"`을 붙였다가 포커스가 떠나면 지웁니다. 사이트가 둔 `tabindex`는 건드리지 않습니다. 주소의 `#main`은 바꾸지 않습니다. 사이트는 `id="main"`을 하나 두고 별도 바로가기 링크를 두지 않습니다.
- 새 메뉴를 받아 링크를 다시 그릴 때 키보드 포커스가 메뉴 안에 있었다면 같은 `id`(없으면 같은 주소)의 링크로 돌려놓습니다. 그 링크가 빠졌으면 그 자리에 온 링크로 옮기고, 포커스가 메뉴 밖에 있었으면 옮기지 않습니다.
- 테마 버튼은 자동(A)→라이트(해)→다크(달)→자동을 순환합니다. 접근 이름에 현재와 다음 테마를 넣고 바꾸면 상태로 알립니다. 사이트의 초기화 스크립트가 쓰는 `localStorage['cola-theme']`, `data-cola-preference`, `data-cola-theme`을 그대로 씁니다. 자동 모드는 시스템 설정 변경을 따르고, 같은 출처의 다른 탭과 storage 이벤트로 동기화합니다. 저장이 막히면 현재 화면에만 적용하고 짧게 안내합니다. 출처가 다른 사이트끼리 테마나 로그인을 공유하지는 않습니다.
- 로그인·세션·CSRF는 각 사이트에 남습니다. 공통 코드는 쿠키나 토큰을 읽지 않습니다.
- 소비 사이트에 CSP가 있으면 `script-src`, `connect-src`에 `https://apps.cola16.app`이 필요합니다. 인라인 대체 CSS를 쓰는 사이트는 `style-src`에서 그 `<style>`을 허용해야 합니다.

## 응답 헤더

위 세 경로의 GET/HEAD에만 `Access-Control-Allow-Origin: *`를 붙입니다. 성공(또는 304) 응답에는 `Cache-Control: public, max-age=300`과 같은 값의 CDN 헤더를 붙이고 `Vary: Cookie`는 붙이지 않습니다. 그 밖의 모든 응답(API, 인증, 즐겨찾기, 앱 화면, 없는 파일)은 기존 `private, no-store`, CSRF, 쿠키, 보안 헤더를 그대로 유지합니다. 경계 테스트는 `src/http.rs`의 `cross_origin_reads_and_caching_are_limited_to_public_branding_files`입니다.

공통 코드는 연결한 모든 사이트의 권한으로 실행됩니다. 따라서 KubeShelf 이미지의 무결성이 연결된 웹사이트 모두에 영향을 줍니다. 자주 바꾸지 않고, 바꿀 때는 CI와 이미지 검수를 거칩니다.

## 현재 소비자와 운영 반영 (2026-10-31)

현재 소비자는 WordPress(`blog`), 포트폴리오(`portfolio`), KubeShelf(`tools`), RAG 검색·원문(`notes`), RAG 지도(`graph`)입니다. 사용자 승인 V7 플럼과 공통 헤더를 운영에 반영했습니다. KubeShelf 제공 이미지가 먼저 Ready·공개 자산200이 된 뒤 소비 사이트를 배포했습니다.

제공 앱 소스는 `08252a326980cd54944dde2b740fd5e42b739f94`, 운영 이미지는 `sha256:24aaa6af684e877970c00bc83e3b52d453a1bfea4fc8d2d3d120e9dd55584c4e`이며 [두 아키텍처 CI](https://github.com/colaH16/kubeshelf-rs/actions/runs/41188901164)를 통과했습니다. 소비 사이트 포함 운영36화면에서 글꼴·위치·헤더 높이·테마 순환·모바일 넘침을 비교했고 공개 원문6조합도 확인했습니다. 미리보기80조합은 공개/소유자·CSP·키보드·저장 실패를 포함합니다. GitOps `scripts/branding/shared-header-v7-verification.json`이 실제 배포·보존·제약 기록입니다. Safari/iOS 실기와 최신 릴리스의 실제 사용자 CAPTCHA 성공은 별도입니다. KubeShelf 공개 카탈로그·Git 설정·개인 설정 파일 수는 전후 동일합니다.

공통 헤더 JS/CSS가 공유하는 것은 UI와 메뉴 JSON입니다. 색 토큰 `--cola-*`는 각 사이트가 자기 CSS로 정의하고 헤더는 이를 이어받기만 합니다. 그래서 색을 바꾸면 사이트마다 토큰을 갱신해 배포해야 합니다. 로그인·계정도 각 사이트가 따로 소유합니다.

### 대체 표시

JavaScript가 꺼져 있거나 공급자에 닿지 못할 때만 각 사이트의 대체 링크와 대체 CSS가 보입니다. 둘 다 마지막으로 빌드했을 때의 사본이므로 최신 메뉴·디자인보다 늦을 수 있고, 링크는 계속 동작합니다.

- KubeShelf: 같은 빌드에서 만들어지므로 항상 같습니다.
- WordPress·포트폴리오: GitOps 저장소의 `scripts/branding/cola-header.vendored.css`(원본 해시 기록)를 `build.py`가 테마 `style.css`와 포트폴리오의 `COLA_HEADER` 영역에 넣습니다. 맞추고 싶을 때만 `npm --prefix web run build` 뒤 `python3 -I scripts/branding/vendor-header-css.py <kubeshelf-rs>/web/dist/branding/v1/cola-header.css`, `python3 -I scripts/branding/build.py`를 실행해 배포합니다. 평소 `build.py`는 네트워크나 작업 경로 없이 같은 결과를 냅니다. 대체 링크는 `functions.php`의 `[cola_header]`와 포트폴리오 HTML에 있는 메뉴 사본이며, 같은 방식으로 필요할 때 맞춥니다.

RAG(노트 검색·원문 `notes`, 노트 지도 `graph`)도 같은 방식으로 연결했습니다. 대체 CSS 사본(`<style>` 또는 같은 출처 파일)과 `async` `<script>`, `current="notes"`/`"graph"` 요소를 넣고, 로그인은 `slot="account"`에 둡니다. CSP의 `script-src`·`connect-src`에 `https://apps.cola16.app`을 추가합니다. 엄격한 `style-src`에서는 대체 CSS를 같은 출처 파일로 두면 `'self'`만으로 됩니다. Shadow DOM 스타일은 `adoptedStyleSheets`로 넣어 `style-src`의 영향을 받지 않습니다. 이를 지원하지 않는 브라우저에서만 Shadow DOM 안에 `<style>`을 만듭니다. 중복 지식 지도 링크는 제거하고, PUBLIC LIBRARY는 헤더 아래 본문으로 옮깁니다. iframe이나 별도 메뉴 서비스는 쓰지 않습니다.


### 계정 버튼 보정 운영 반영

2026-10-31 사용자 요청에 따라 로그인/로그아웃→테마 아이콘 순서와 공통 버튼 스타일을 운영에 반영했다. 런타임 소스 `4b569b1b6a1a39e2a313b23bbd3928adffbd22e2`, 이미지 `sha256:b5990accfe8cdc9b872ce81c8677308c005f90ff0189640ea3dd97b6fe658f12` ([CI](https://github.com/colaH16/kubeshelf-rs/actions/runs/41206483339)). 앞선 소스08252a3/이미지24aaa6a 기록은 최초 V7 배포 이력이다. 한글 폰트 미리보기100조합·제공 실패40조합·실운영30조합에서 검수했고 기존 카탈로그·Git 설정·개인 파일 수는 동일하다. GitOps `scripts/branding/shared-header-account-verification.json`을 현재 보정 기록으로 따른다.
