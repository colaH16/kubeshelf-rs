# Go → Rust 대응표와 학습 노트

원본 Go 코드를 읽어 본 사람이 Rust 코드를 따라갈 수 있도록 정리했습니다. 각 Rust 함수의 doc 주석에도 대응하는
Go 함수 이름(예: “Go `BuildCatalog`”)을 적어 두었습니다.

## 파일 대응

| Go (`internal/shelf/…`) | Rust (`src/…`) | 핵심 |
| --- | --- | --- |
| `model.go` 타입·검증, `favorites.go` 타입·검증 | `model.rs` | serde 구조체, `Settings::validate`, `FavoriteProfile::{validate,state}` |
| (Go 표준 라이브러리 `net/url`) | `gourl.rs` | Go 1.27.1 URL 파싱·출력 규칙을 그대로 옮김 |
| `ConfigFromEnv` | `config.rs` | `Config::from_lookup` (테스트에서 환경 변수 대신 클로저 사용) |
| `catalog.go` | `catalog.rs` | `build` = `BuildCatalog`, `service_health` = `serviceHealth` |
| `store.go`, `favorite_store.go`의 Git 부분 | `git.rs` | `Repository::{read,write,read_favorites}`, 매니페스트 파싱·인코딩 |
| `store.go`, `favorite_store.go`의 `Store` | `store.rs` | applied/desired 분리, CAS, 계정 파일 준비 |
| `auth.go` | `auth.rs` | OIDC, 세션, 디렉터리 |
| `server.go`, `routes.go`, `favorites.go`의 핸들러 | `http.rs` | axum 라우터, 응답 형식, `serve` |
| `source.go`, `user_settings_watch.go` | `source.rs` | 6종 목록 조회, ConfigMap watch |
| `cmd/kubeshelf/main.go` | `main.rs` | 시작 순서, 종료 처리 |

## 개념 대응

| Go | Rust | 이 코드에서의 예 |
| --- | --- | --- |
| 구조체 태그 `json:"name,omitempty"` | `#[serde(rename…, skip_serializing_if = …)]` | `model.rs` 전체. 필드 순서가 곧 JSON 순서라 해시가 같아짐 |
| `map[string]T` (JSON은 키 정렬) | `BTreeMap<String, T>` | 정렬된 맵이라 Go와 같은 JSON |
| nil 슬라이스/맵과 `null` | `Vec`/`BTreeMap`은 nil이 없음 → `null_vec`, `null_map`, `null_default` | `null`을 “기본값”으로 읽는 Go 동작 재현 |
| `DisallowUnknownFields()` | `#[serde(deny_unknown_fields)]` + `decode_json`의 모양 검사 | serde는 구조체를 배열로도 받기 때문에 추가 검사 필요 |
| `error`, `fmt.Errorf` | `anyhow::Result`, `ensure!`, `bail!` | 메시지 문자열은 원본과 같게 유지 |
| `errors.Is(err, ErrConflict)` | `err.downcast_ref::<Conflict>()` | `http.rs`의 `conflict_or` |
| goroutine + `context.Context` | `tokio::spawn`, `JoinSet`, `tokio::time::timeout` | `main.rs`, `store.rs` |
| `sync.Mutex` / `sync.RWMutex` | `tokio::sync::{Mutex, RwLock}` | `.await` 중에도 잡고 있을 수 있는 비동기 잠금 |
| 값 복사로 스냅숏 반환 | `Arc<Snapshot>` 교체 | `source.rs`: 요청마다 전체 데이터 복사를 피함 |
| `http.ServeMux` + 미들웨어 | `axum::Router` + `middleware::from_fn` | `http.rs`의 `router`, `security_headers` |
| `http.Server{ReadHeaderTimeout…}` + `Shutdown` | hyper `http1::Builder` + `GracefulShutdown` | `http.rs`의 `serve` |
| `http.Error` (평문) / `json.Encoder` | `plain_error` / `json_response` | 응답 본문·Content-Type까지 원본과 같게 |
| `exec.CommandContext("git", …)` | `tokio::process::Command` + `kill_on_drop` | 시간 초과 시 자식 프로세스도 정리 |
| client-go informer | `kube::runtime::watcher` + resourceVersion 비교 | `source.rs::watch_users` |
| go-oidc + oauth2 | `openidconnect` 크레이트 | `auth.rs` (알고리즘 정책을 go-oidc에 맞춤) |
| `//go:embed dist/*` | `KUBESHELF_STATIC_DIR` + `tower_http::ServeDir` | 이미지에 `web/dist`를 복사 |
| `strings.ToLower` | `model::go_lower` | Rust `to_lowercase`는 그리스어 끝 시그마 등 문맥 규칙이 있어 따로 구현 |

## 읽을 때 도움이 되는 점

1. **같은 결과를 위한 장치**: 설정 버전은 JSON 바이트의 SHA-256입니다. 그래서 `go_json`이 Go처럼 `<`, `>`, `&`,
   U+2028/2029를 이스케이프하고, 필드 순서·생략 규칙을 Go와 똑같이 맞춥니다. 한 글자라도 다르면 롤백·혼용 시 CAS가 깨집니다.
2. **URL을 직접 구현한 이유**: `url` 크레이트는 WHATWG 표준이라 `http:/host`, 앞뒤 공백, 큰 포트 번호 등을 Go와 다르게
   처리합니다. 저장되는 URL 문자열이 달라지면 지문(fingerprint)과 화면이 달라지므로 Go 규칙을 그대로 옮겼습니다.
3. **권한 → 그룹 순서**: `catalog.rs`의 큰 루프에서 `endpoint_policy(...).allows(user)` 검사가 그룹(`assignments`) 계산보다
   먼저 옵니다. 순서를 바꾸면 그룹이 권한을 넓힐 수 있으므로 주석으로 표시해 두었습니다.
4. **applied와 desired**: `store.rs` 맨 위 설명을 먼저 읽으세요. 저장은 Git(desired)만 바꾸고, 권한은 투영된 파일(applied)에서만 옵니다.
5. **테스트 읽는 순서**: 단위 테스트(`cargo test`) → `tests/compare.py`(원본과 같은지) → `tests/e2e.py`(실제 바이너리).
