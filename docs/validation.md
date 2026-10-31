# 검증 기록과 재개 메모

마지막 실행: 2026-10-31, 로컬 Linux x86_64, Rust 1.99.1, Go 1.27.1(원본 오라클 빌드), Node 24.22.2.
아래 최초 검증은 Claude가 로컬 모의 환경에서 수행했습니다. 이후 실클러스터 읽기 전용 검증 결과는 문서 마지막에 기록합니다.

## 실행한 명령과 결과

| 명령 | 결과 |
| --- | --- |
| `cargo fmt --check` | 통과 |
| `cargo clippy --locked --all-targets -- -D warnings` | 경고 0 |
| `cargo test --locked` | 12개 통과 (아래 목록) |
| `python3 scripts/build_reference.py --source <원본 clone> --output <임시 경로>` | 고정 커밋 `957bdd0`에서 Go 오라클 빌드, 원본 저장소 `git status` 깨끗함 유지 |
| `KUBESHELF_GO_REFERENCE=… python3 tests/compare.py` | `tests/demo.json` 바이트 일치 + 397개 차등 사례 일치 (catalog 96, url 52, favorites 56, route 25, settings 22, decode-settings 18, host 18, favorite-path 19, config 17, store-validation 17, read-settings 16, read-favorites 8, encode-favorites 5, encode-settings 2, change-summary 3, legacy-defaults 3, personal-summary 2, default-favorites 4, cross-read 14). 문서화된 Rust 쪽 엄격 사례 8개는 방향까지 확인 |
| `cd web && npm ci --ignore-scripts && npm test && npm run build` | 4개 파일 14개 테스트 통과, `web/dist` 빌드 |
| `python3 tests/e2e.py target/debug/kubeshelf-rs` | 145개 검사 통과 |
| `KUBESHELF_VERSION=… cargo build --locked --release` 후 `python3 tests/e2e.py target/release/kubeshelf-rs` | 145개 검사 통과, 바이너리 약 14.8 MB(strip), 동적 의존성 glibc/libgcc/libm만 |

단위 테스트 12개: Git 개인 파일 격리·공유 CAS·applied 경계, 기존 공유 기본값 이전, 오래된 push 거부(타인 커밋 보존),
subject 불일치 매니페스트 거부, 엄격 ConfigMap(중첩 알 수 없는 필드·중복 키·잘못된 타입), YAML 1.1 안전 스칼라,
개인 파일 경로, 매니페스트 경로 검증, Go URL 렌더링, HTTP 역할·CSRF·엄격 JSON·읽기 전용·평문 오류, TCP 확인 주소 차단,
로컬 OIDC 제공자 대상 PKCE·state 쿠키·nonce·audience·issuer·만료·잘못된 서명·HS256 거부·키 교체·비활성 계정 세션 제거.

종단 간 145개 검사 요약:

- 데모: healthz/readyz, 보안 헤더·CSP, SPA 경로 5개, 해시 asset, 404(경로 이탈 포함), 405, 익명 401/403, 익명 카탈로그에
  노드 주소 없음, 데모 로그인 Origin 검사, 그룹 정책, 즐겨찾기 CSRF·Origin·알 수 없는 필드·배열 JSON·Content-Type·저장·
  유지·409·보이지 않는 기본 대상 거부, 안전한 returnTo, 관리자 저장 pending → 1.5초 후 적용, 409, 기존 기본값 읽기 전용,
  TCP 확인 거부, 로그아웃 Origin/CSRF, SIGTERM 종료 코드 0과 시간 제한.
- 운영 모드(로컬 모의 서버): 인자 거부, 접근 불가 Git이면 시작 실패(65초 이내, 비밀 값 미노출), SSH 옵션
  (`-F /dev/null`, `IdentitiesOnly`, `StrictHostKeyChecking=yes`, known_hosts)과 키 사본 0600, 활성 계정만 개인 파일 생성,
  `resourceVersion=0` 목록, 라벨 범위 ConfigMap watch, 콜백 state 불일치 400, 비활성 계정 로그인 403과 로그인 쿠키 삭제,
  OIDC 로그인(PKCE·스코프·콜백 URL), 디렉터리 기반 신원, 사용자 파일에만 즐겨찾기 커밋과 커밋 메시지, 관리자 저장이
  Git에 push되지만 카탈로그는 그대로(pending), 409, 투영 파일 변경 후 적용, 잘못된 투영 파일은 오류 표시 + 마지막 정상
  설정 유지, 외부 커밋 + watch 이벤트로 Git 스냅숏 갱신, 관련 없는 커밋은 CAS 토큰 불변, 로그아웃, SIGTERM.

## 검증하지 못한 것 (남은 과제)

1. **GitHub Actions 미실행**: `.github/workflows/ci.yml`은 작성만 했습니다. 첫 push 후 amd64·arm64 결과를 확인해야 합니다.
   특히 arm64 러너에서의 Node tarball·rolldown/TypeScript 네이티브 바인딩, amd64 Chrome 브라우저 스모크(필수로 설정됨).
2. **컨테이너 이미지 미빌드**: 이 환경의 rootless podman은 subuid 매핑이 없어 Ubuntu 이미지를 풀 수 없었습니다.
   Dockerfile·`.dockerignore`는 CI에서 처음 검증됩니다.
3. **브라우저 스모크 추가 확인**: Chromium 종료 원인은 로컬 폰트 설정이었습니다. `FONTCONFIG_FILE`을 유효한 Fontconfig 설정으로 지정하고 `KUBESHELF_E2E_BROWSER_REQUIRED=1`로 실행해 화면 표시·비공개 정보 숨김을 포함한 146개 검사를 통과했습니다. 위 145개 기록은 최초 브라우저 생략 실행입니다.
4. **실제 서비스와의 통합**: authentik, 실제 Kubernetes API, Fleet, SSH Git 서버와는 아직 연결해 보지 않았습니다.
   (모의 서버는 원본 코드가 쓰는 API 형식에 맞췄습니다.)
5. **운영 교체**: [operations.md](operations.md)의 확인 목록으로 교체 후 점검이 필요합니다.

## 재개 방법

```sh
# 도구 (예시: 이 작업 환경에서 쓴 고정 버전)
rustup toolchain install 1.99.1 --profile minimal --component rustfmt,clippy
# Go 1.27.1, Node 24.22.2, git, openssl, python3 필요

git clone https://github.com/colaH16/KubeShelf /tmp/kubeshelf-original   # 고정 커밋 포함 clone
python3 scripts/build_reference.py --source /tmp/kubeshelf-original --output /tmp/go-reference
(cd web && npm ci --ignore-scripts && npm test && npm run build)
cargo fmt --check && cargo clippy --locked --all-targets -- -D warnings && cargo test --locked
cargo build --locked --bin reference && KUBESHELF_GO_REFERENCE=/tmp/go-reference python3 tests/compare.py
cargo build --locked && python3 tests/e2e.py
```

차등 테스트가 실패하면 상세 내용이 `KUBESHELF_MISMATCH_FILE`(기본 `/tmp/kubeshelf-mismatch.json`)에 기록됩니다.
새로운 의도적 차이를 추가할 때는 `tests/compare.py`의 `RUST_STRICTER`와 [compatibility.md](compatibility.md)를 함께 고치세요.

## CI 및 읽기 전용 실클러스터 검증

- 소스 `79ceafefcc2b239715a59c2ee5fc0cb5a745d89a`, [CI 41172859971](https://github.com/colaH16/kubeshelf-rs/actions/runs/41172859971): amd64·arm64 모두 성공, 통합 이미지 게시 및 익명 pull 확인.
- 이미지: `ghcr.io/colah16/kubeshelf-rs@sha256:91b8198500845ac2f2a89062dec6d789dbcc0ded93e4a1bc2b9f137439424692`.
- 릴리스 바이너리의 로컬 브라우저 포함 종단 간 검사 146개 통과.
- 실제 기존 Secret 참조·환경 변수·서비스 계정·UID/GID 10001·읽기 전용 루트 파일시스템을 사용하되 서비스 선택자와 분리하고 `KUBESHELF_READ_ONLY=true`로 시험했습니다. Secret 값은 공개하지 않습니다.
- readiness 통과. 익명 카탈로그는 버전·갱신 시각을 제외하면 Go와 동일(공개 카드 5개, 노드 대상 0개).
- 상태 경로 2개, 화면 경로 5개, 카탈로그, 관리자 403, 로그인 필요 API 401, 실제 OIDC PKCE/state 로그인 리다이렉트 302 확인.
- 실제 사용자 인증 완료·운영 설정 저장은 하지 않았습니다. 이 흐름의 저장·충돌·적용 경계는 로컬 모의 환경 검증입니다.
- 운영 GitOps는 이미지 필드만 변경했습니다. 기존 sealed-secrets 컨트롤러가 준비되지 않아 Fleet가 의존성 검사에서 보류 중입니다. 기존 Go 서비스는 유지되며, **운영 전환 완료가 아닙니다**. 의존성 복구 후 Fleet 적용과 운영 확인이 남았습니다.

위 “검증하지 못한 것” 목록은 최초 로컬 검증 시점의 기록이며, CI·이미지·브라우저·읽기 전용 실환경 항목은 이 추가 검증으로 해소됐습니다.

## 운영 전환 완료 (2026-10-31 KST)

앞서 기록한 인프라 의존성이 정상화되어 Fleet가 승인된 이미지 변경을 적용했습니다.

- Rust 운영 Pod 1/1, 재시작 0. 기존 활성 계정 3개를 확인했고 개인 설정 파일을 새로 만들거나 기존 파일을 덮어쓰지 않았습니다(`initialized=0`).
- 전환 전후 Pod 사양을 비교해 이미지 외에 환경 변수·인자·볼륨·서비스 계정·보안 설정이 동일함을 확인했습니다.
- 설정 저장소 커밋은 전환 전후 동일합니다.
- 실제 운영 Pod에서 화면 5개·상태 경로 2개·익명 카탈로그·권한 401/403·OIDC PKCE 로그인 리다이렉트 검증 통과.
- 운영 카탈로그는 버전·갱신 시각을 제외하면 Go 기준 응답과 동일합니다.
- 운영 계정으로 실제 로그인 완료·설정 저장을 실행하지 않았습니다. 전체 인증·저장 흐름의 검증은 위 로컬 모의 환경 결과를 참고하세요.
- 단일 replica와 Recreate 방식은 유지했습니다. 이 결과를 무중단 전환으로 표현하지 않습니다.
