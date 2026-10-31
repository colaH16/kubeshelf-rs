# 운영: 배포, 롤백, 보안, 알려진 제한

> 2026-10-31: CI·읽기 전용 비교를 거쳐 개인 Kubernetes 클러스터의 운영 이미지를 Rust로 전환했습니다. 기존 인프라 의존성 복구 후 Fleet가 승인된 이미지 변경을 적용했습니다.

## 이미지

CI(`.github/workflows/ci.yml`)는 `main` push마다 amd64(`ubuntu-24.04`)와 arm64(`ubuntu-24.04-arm`) 러너에서 각각
프런트엔드 테스트·빌드, Rust fmt/clippy/test, 원본 Go와의 차등 테스트, 릴리스 바이너리 종단 간 테스트를 통과한 뒤에만
이미지를 올립니다.

- 아키텍처별 태그: `ghcr.io/colah16/kubeshelf-rs:sha-<커밋>-amd64`, `…-arm64`
- 두 아키텍처가 모두 통과한 뒤 멀티 아키텍처 매니페스트: `ghcr.io/colah16/kubeshelf-rs:sha-<커밋>`
- 런타임: Ubuntu 24.04(다이제스트 고정), `git`, `openssh-client`, `ca-certificates`, UID/GID 10001(`kubeshelf`, passwd 항목 있음),
  `/work`는 10001 소유 0700, `ENTRYPOINT ["/usr/local/bin/kubeshelf"]`(인자 없음), `KUBESHELF_STATIC_DIR=/usr/share/kubeshelf/web`, `HOME=/tmp`.
- 프로세스가 쓰는 곳은 `/work`와 `/tmp`뿐이라 `readOnlyRootFilesystem: true`에서 동작합니다(차트가 둘 다 `emptyDir`로 마운트).

GHCR 패키지를 처음 만든 뒤 클러스터에서 익명으로 받으려면 패키지 공개 범위를 Public으로 바꿔야 합니다.

## 배포 (원본 차트 0.1.12 + 이미지 교체)

차트·Secret·ConfigMap·Fleet 구성은 원본과 같습니다. 원본 [operations.md](https://github.com/colaH16/KubeShelf/blob/957bdd081db04ea2a205672e5f8e4d968c5ed193/docs/operations.md)의
필수 리소스(`kubeshelf-settings`, `kubeshelf-config-git`, `kubeshelf-git-known-hosts`, `kubeshelf-auth`)를 그대로 씁니다.
private values에서 이미지만 덮어씁니다. 태그보다 다이제스트 고정을 권장합니다.

```yaml
image:
  repository: ghcr.io/colah16/kubeshelf-rs
  digest: sha256:<CI 요약에 기록된 멀티 아키텍처 다이제스트>
  # 또는 tag: sha-<커밋>
```

차트가 이미 설정하는 것: `runAsUser/runAsGroup/fsGroup: 10001`, `readOnlyRootFilesystem`, `/work`·`/tmp` emptyDir,
`Recreate` 전략(1 replica), `/readyz` startup/readiness probe, `/healthz` liveness probe. RBAC도 그대로 충분합니다
(6종 리소스 get/list, 릴리스 네임스페이스 ConfigMap get/list/watch). Rust 버전은 `resourceVersion=0` 목록 조회와
ConfigMap watch만 합니다.

교체 직후 확인할 것:

1. `/readyz`가 `ready`를 반환하고 로그에 `KubeShelf sha-… listening`이 찍힘.
2. 로그의 `Favorite accounts ready: N active, M initialized` — 기존 사용자 파일이 있으면 `M=0`이어야 합니다(기존 파일은 절대 덮어쓰지 않음).
3. 관리자 화면의 적용 상태(`/api/admin/status`)에서 `pending=false`, `appliedRevision`이 현재 ConfigMap의 revision과 같음.
4. 관리자 저장 1회 → Git에 커밋이 올라가고 `pending=true` → Fleet 반영 후 `pending=false`.
5. 일반 사용자로 로그인해 즐겨찾기 저장 → `runtime/users/kubeshelf-user-<hash>.yaml`만 바뀜.

## 롤백

Git 형식이 같으므로 롤백은 이미지 값을 원래대로(`ghcr.io/colah16/kubeshelf:0.1.12`) 되돌리는 것으로 끝납니다.

- 공유 설정 JSON은 원본과 같은 직렬화(필드 순서, HTML 이스케이프, 들여쓰기)이고 버전 해시도 같아서, 롤백 후에도
  편집 중이던 화면의 CAS 토큰이 그대로 유효합니다.
- YAML 매니페스트도 일반적인 값(영숫자 ID, UUID 형태 subject)에서는 원본과 바이트 단위로 같습니다. 특수한 subject는
  큰따옴표로 쓰지만 원본(Go)이 같은 값으로 읽는 것을 차등 테스트로 확인했습니다.
- 세션은 메모리에만 있으므로 교체·롤백 시 모두 다시 로그인해야 합니다(원본도 같음).

## 보안 경계 (원본과 같게 유지)

- **적용된 설정만 권한에 사용**: 관리자 저장은 Git push까지만 하고 `pending`이 됩니다. 실제 권한·카탈로그는 Fleet가 투영한
  ConfigMap(`KUBESHELF_SETTINGS_FILE`)이 바뀐 뒤에만 바뀝니다. 투영 파일이 잘못되면 마지막 정상 설정을 유지하고 오류를 표시합니다.
- **CAS 범위**: 공유 설정은 설정 JSON의 SHA-256, 개인 파일은 각자의 해시로 충돌을 검사합니다. 다른 사용자나 관련 없는 커밋은
  충돌을 일으키지 않습니다. force push는 하지 않습니다.
- **서버가 정하는 경로**: Git에 쓰는 경로는 설정 경로와 `users/kubeshelf-user-<sha256(subject) 앞 40자리>.yaml`뿐이며,
  요청에서 경로를 받지 않습니다. 읽을 때도 이름 해시와 subject 주석이 모두 일치해야 합니다.
- **활성 계정만**: 매 요청마다 디렉터리(30초 캐시)에서 사용자를 다시 확인합니다. 비활성·삭제된 계정은 세션을 잃고,
  디렉터리를 확인할 수 없으면 익명으로 처리합니다(fail closed).
- **엄격한 입력**: JSON 요청·설정 파일은 알 수 없는 필드, 배열로 쓴 구조체, 800 KB 초과를 거부합니다. Git 매니페스트는
  모든 깊이에서 알 수 없는 필드와 중복 키를 거부합니다.
- **권한 후 그룹화**: 각 주소는 자신의 정책(주소 → 카드 → 네임스페이스)으로 먼저 걸러진 뒤에 표시용 그룹으로 묶입니다.
  NodePort는 Ingress 네임스페이스 정책을 상속하지 않고 기본이 관리자 전용입니다. 노드 주소는 보이는 NodePort가
  쓰는 대상만 응답에 포함됩니다.
- **CSRF**: 쓰기 요청은 `Origin`이 공개 URL과 같고 세션 CSRF 헤더가 맞아야 합니다. 세션 쿠키는 `__Host-`, `Secure`,
  `HttpOnly`, `SameSite=Lax`입니다.
- **OIDC**: PKCE(S256), state 쿠키 결합, nonce, 서명(JWKS)·issuer·audience·만료 검증. 허용 알고리즘은 go-oidc와 같이
  제공자가 광고한 비대칭 알고리즘(없으면 RS256)이며 HMAC/none은 받지 않습니다. 검증 실패 시 discovery/JWKS를 한 번 다시 받아
  키 교체에 대응합니다. `at_hash`가 있으면 추가로 확인합니다(원본보다 엄격).
- **Git/SSH**: 앱 전용 클론만 `reset --hard`하고, hook 비활성, `file://` 프로토콜 금지, `-F /dev/null`, `IdentitiesOnly`,
  `StrictHostKeyChecking=yes`. Git 오류 원문(원격 주소 포함 가능)은 HTTP로 돌려주지 않고 debug 로그에만 남깁니다.
- **TCP 확인**: 관리자만, TCP 확인이 허용된 수동 도메인과 카탈로그에 실제로 있는 NodePort만, loopback·link-local·멀티캐스트·
  미지정 주소(IPv4-mapped 포함)는 거부합니다.
- **HTTP 서버**: 헤더 읽기 5초, 헤더 64 KiB, 요청 본문 15초, 핸들러 60초 제한. 모든 응답에 `no-store`·CSP 등 보안 헤더.
  종료 신호 시 새 연결을 받지 않고 진행 중인 요청을 최대 10초 기다립니다.

## 알려진 제한

- 실제 Git clone·Kubernetes 조회·OIDC discovery/PKCE 로그인 리다이렉트·익명 화면은 읽기 전용 시험 Pod에서 확인했습니다. 실제 사용자 로그인 완료와 운영 설정 저장은 수행하지 않았으며, 전체 흐름은 로컬 모의 서버로 검증했습니다.
- amd64·arm64 CI에서 Dockerfile 빌드와 이미지 게시가 성공했습니다. CI 이미지는 실제 클러스터의 UID 10001·읽기 전용 루트 파일시스템에서 시작과 readiness를 통과했습니다.
- 단일 replica, 메모리 세션(원본과 같음). 재시작하면 모두 다시 로그인합니다.
- 브라우저 렌더링 스모크 테스트는 로컬 폰트 설정을 제공한 뒤 통과했습니다. CI amd64에서도 필수로 설정했습니다.
- 원본과 의도적으로 다른 부분은 [compatibility.md](compatibility.md)에 정리했습니다.
