# 설정 (환경 변수)

모든 런타임 설정은 환경 변수로 합니다. 바이너리는 인자를 받지 않으며, 인자를 주면 시작하지 않습니다.
이름·기본값·검증 순서는 원본 Go의 `ConfigFromEnv`와 같습니다. 빈 문자열은 “설정하지 않음”으로 취급되어 기본값이 쓰입니다.

## 원본과 같은 변수

| 변수 | 기본값 | 설명 |
| --- | --- | --- |
| `KUBESHELF_LISTEN` | `:8080` | 수신 주소. `:포트`는 모든 인터페이스(IPv6 우선, 실패 시 IPv4) |
| `KUBESHELF_PUBLIC_URL` | (데모: `http://localhost:8080`) | 공개 origin. 경로·쿼리·프래그먼트 금지, 끝의 `/`는 제거. 운영에서는 `https` 필수. `Origin` 검사와 OIDC 콜백 주소에 사용 |
| `KUBESHELF_CLUSTER_NAME` | `cluster` | 자동 발견 카드 ID 계산에 들어갑니다. **바꾸면 카드 ID가 바뀝니다** |
| `KUBESHELF_KUBECONFIG` | (비어 있음) | 비어 있으면 in-cluster 설정 사용 |
| `KUBESHELF_SETTINGS_FILE` | `/config/runtime/settings.json` | Fleet가 투영한 ConfigMap의 `settings.json` (적용된 설정) |
| `KUBESHELF_WORK_DIR` | `/work` | 앱 전용 Git 클론과 SSH 키 사본(0600) 위치. 쓰기 가능해야 함 |
| `KUBESHELF_GIT_URL` | (운영 필수) | SSH 형식만 허용: `ssh://…` 또는 `user@host:path` |
| `KUBESHELF_GIT_BRANCH` | `main` | |
| `KUBESHELF_GIT_PATH` | `runtime/settings.yaml` | 공유 설정 매니페스트 경로. 개인 파일은 같은 디렉터리의 `users/` 아래 |
| `KUBESHELF_SSH_KEY_FILE` | `/credentials/git/ssh-privatekey` | |
| `KUBESHELF_KNOWN_HOSTS_FILE` | `/credentials/git-host/known_hosts` | `StrictHostKeyChecking=yes`로 사용 |
| `KUBESHELF_OIDC_ISSUER` | (운영 필수) | OIDC issuer URL (discovery 문서의 `issuer`와 정확히 같아야 함) |
| `KUBESHELF_OIDC_CLIENT_ID` | (운영 필수) | |
| `KUBESHELF_OIDC_SECRET_FILE` | `/credentials/auth/client-secret` | 요청마다 다시 읽음 (Secret 교체 시 재시작 불필요) |
| `KUBESHELF_DIRECTORY_URL` | (비어 있음) | authentik 호환 API 기준 URL (`/api/v3/core/users/`, `/groups/`) |
| `KUBESHELF_DIRECTORY_TOKEN_FILE` | `/credentials/auth/directory-token` | |
| `KUBESHELF_ADMIN_SUBJECTS` | (운영 필수) | 쉼표로 구분한 관리자 OIDC subject. **첫 번째**가 기존 공유 기본 주소를 넘겨받는 기본 관리자 |
| `KUBESHELF_INGRESS_SCHEMES` | `{}` | IngressClass 이름 → `http`/`https` JSON 객체. 예: `{"traefik":"https"}` |
| `KUBESHELF_DEMO` | (거짓) | 정확히 `true`일 때만 데모. 그 밖의 값(`TRUE` 등)은 거짓 (원본과 같음) |
| `KUBESHELF_CONFIGMAP_NAME` | `kubeshelf-settings` | 공유 ConfigMap 이름 (Git 매니페스트 검증) |
| `POD_NAMESPACE` | `public-services` | ConfigMap 네임스페이스. 차트가 downward API로 설정 |

운영 모드(데모가 아닐 때) 시작 조건: `https` 공개 URL, OIDC issuer·client ID, 관리자 subject 1개 이상, Git URL.

## Rust 포팅에서 추가된 변수

| 변수 | 기본값 | 설명 |
| --- | --- | --- |
| `KUBESHELF_STATIC_DIR` | `web/dist` (이미지: `/usr/share/kubeshelf/web`) | 빌드된 프런트엔드 디렉터리. 원본은 바이너리에 포함했음 |
| `KUBESHELF_READ_ONLY` | (거짓) | `true`이면 UI·API는 그대로 제공하되 모든 쓰기(설정 저장, 즐겨찾기, 계정 파일 생성, TCP 확인)를 거부. 교체 전 검증용 |
| `RUST_LOG` | `info` | 로그 수준 (`tracing` 필터 문법). 예: `info,kubeshelf_rs=debug` |

빌드 시점 변수 `KUBESHELF_VERSION`은 UI에 표시되는 버전 문자열입니다(없으면 `dev`, 원본의 `-X main.version`과 같은 역할).
CI는 `sha-<커밋>`을 넣습니다.

## 차트와의 대응

원본 차트 0.1.12는 위 변수 대부분을 values에서 채웁니다. `KUBESHELF_STATIC_DIR`은 이미지의 `ENV`로 설정되어 있으므로
차트를 수정할 필요가 없습니다. `KUBESHELF_READ_ONLY`를 쓰려면 차트에 env를 추가해야 하는데, 원본 차트는 임의 env를 받지 않습니다.
읽기 전용 검증은 차트 밖(예: 로컬 kubeconfig로 실행)에서 하거나 별도 오버레이가 필요합니다.
