# kubeshelf-rs

[KubeShelf](https://github.com/colaH16/KubeShelf)(Go)의 Rust 포팅입니다. Kubernetes Ingress·NodePort 서비스를 찾아
한 화면(“선반”)에 모으고, 네임스페이스 기본값과 사용자·그룹별 공개 범위를 적용하며, 설정은 GitOps(Git 커밋 → Fleet 적용)로 관리합니다.

> **상태 (2026-10-31)**: 구현·테스트·패키징 준비 단계입니다. **아직 어떤 클러스터에도 배포되지 않았고**, 운영 중인 것은
> 여전히 원본 Go 버전입니다. 컨테이너 이미지는 CI가 처음 성공적으로 실행된 뒤에 생성됩니다.

## 원본과의 관계

- 원본: `colaH16/KubeShelf` 커밋 `957bdd081db04ea2a205672e5f8e4d968c5ed193` (차트/앱 버전 0.1.12).
- HTTP 경로·메서드·상태 코드·JSON 형식, `KUBESHELF_*` 환경 변수, Git 매니페스트 형식, 설정·즐겨찾기 버전 해시(SHA-256)를
  원본과 같게 유지합니다. 같은 Git 저장소와 같은 Helm 차트(0.1.12, 이미지만 교체)로 교체 배포하는 것이 목표입니다.
- `web/`의 프런트엔드(React)는 원본 코드를 그대로 가져왔습니다. 바뀐 것은 `web/vite.config.ts`의 빌드 출력 경로
  (`../internal/webassets/dist` → `dist`) 하나뿐입니다. 원본은 프런트엔드를 Go 바이너리에 포함했지만, 이 포팅은
  `KUBESHELF_STATIC_DIR` 디렉터리에서 제공합니다.
- `charts/kubeshelf/`는 원본 0.1.12 차트를 바이트 단위로 그대로 복사한 **호환성 기준 차트**입니다(기본 이미지도 원본 그대로).
  Rust 이미지는 `image.repository`/`image.tag`(또는 `image.digest`)를 덮어써서 사용합니다. 자세한 내용은 [운영 문서](docs/operations.md)를 보세요.

라이선스: 원본 저장소에는 LICENSE 파일이 없고 GitHub에도 라이선스가 표시되어 있지 않습니다(커밋 `957bdd0` 기준 확인).
그래서 이 저장소도 별도의 오픈소스 라이선스를 표시하지 않습니다. 원본과 이 저장소는 같은 소유자(colaH16)의 것입니다.

## 빠른 시작 (데모 모드)

Rust 1.99.1, Node 24, Git이 필요합니다. 데모는 합성 데이터(문서용 주소 `192.0.2.*`, `*.example.com`)만 쓰며
Kubernetes·OIDC·Git 서버가 필요 없습니다.

```sh
(cd web && npm ci --ignore-scripts && npm run build)
cargo build --release
KUBESHELF_DEMO=true KUBESHELF_PUBLIC_URL=http://localhost:8080 ./target/release/kubeshelf-rs
```

`http://localhost:8080`에 접속합니다. 로그인 버튼은 데모 관리자 세션을 엽니다. 운영 배포에서 데모 모드를 켜지 마세요.
바이너리는 명령행 인자를 받지 않습니다. 모든 설정은 환경 변수로 합니다([설정 문서](docs/configuration.md)).

## 검증

```sh
# Rust
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked

# 원본 Go 구현과의 차등 테스트 (원본은 읽기만 하고 임시 디렉터리에서 빌드)
python3 scripts/build_reference.py --source /path/to/KubeShelf --output /tmp/go-reference
cargo build --locked --bin reference
KUBESHELF_GO_REFERENCE=/tmp/go-reference python3 tests/compare.py

# 실제 바이너리 종단 간 테스트 (OIDC·디렉터리·Kubernetes API·Git을 모두 로컬 모의 서버로)
cargo build --locked
python3 tests/e2e.py

# 프런트엔드
(cd web && npm test && npm run build)
```

마지막으로 확인한 결과와 아직 검증하지 못한 항목은 [검증 기록](docs/validation.md)에 있습니다.

## 문서

| 문서 | 내용 |
| --- | --- |
| [docs/configuration.md](docs/configuration.md) | 환경 변수 전체 목록과 기본값 |
| [docs/operations.md](docs/operations.md) | 배포(원본 차트 + 이미지 교체), 롤백, 보안 경계, 알려진 제한 |
| [docs/compatibility.md](docs/compatibility.md) | 원본과 같게 맞춘 부분과 의도적으로 다른 부분 |
| [docs/go-to-rust.md](docs/go-to-rust.md) | Go 코드 → Rust 모듈 대응표와 학습용 설명 |
| [docs/validation.md](docs/validation.md) | 실행한 검증 명령과 결과, 남은 과제, 재개 방법 |

## 저장소 구조

```text
src/            Rust 서버 (main.rs, lib.rs와 모듈들; src/bin/reference.rs는 차등 테스트용)
web/            원본 React 프런트엔드
tests/          compare.py(차등), e2e.py(종단 간), demo.json(원본이 내보낸 데모 데이터), go-reference/(Go 어댑터)
scripts/        build_reference.py (고정 커밋의 원본으로 Go 오라클 빌드)
charts/         원본 0.1.12 Helm 차트 (호환성 기준, 수정 없음)
Dockerfile      CI가 만든 네이티브 바이너리 + web/dist만 담는 Ubuntu 24.04 런타임 이미지
```
