# 원본(Go)과의 호환성

기준: `colaH16/KubeShelf` `957bdd081db04ea2a205672e5f8e4d968c5ed193`, Go 1.27.1 툴체인, `go.mod`의 `go 1.25.0`.

## 차등 테스트로 확인하는 것

`tests/compare.py`는 같은 입력을 원본 Go 함수와 Rust 함수에 넣고 결과 JSON이 같은지 봅니다. Go 쪽은
`scripts/build_reference.py`가 고정 커밋을 `git archive`로 임시 디렉터리에 풀고 `tests/go-reference/`의 어댑터 두 파일만
추가해 빌드합니다(원본 저장소는 읽기만 함). 어댑터는 원본의 비공개 함수를 감쌀 뿐 동작을 바꾸지 않습니다.

| 영역 | 내용 |
| --- | --- |
| 카탈로그 | 데모 데이터와 별도의 경계 사례 클러스터(EndpointSlice 포트/프로토콜 일치, terminating·not-ready·삭제 중 Pod, 다른 네임스페이스 Pod, ExternalName·Pod가 아닌 엔드포인트, `externalTrafficPolicy: Local`과 ready 노드, InternalIP 빈 값→ExternalIP, IPv6 노드, not-ready 노드, TLS·IngressClass 스킴, 이름/번호 포트, 없는 Service, 와일드카드·호스트 없음·정규식 경로, 공백·유니코드·`%`·`?`·`#` 경로, 유니코드 호스트, defaultBackend, 수동 URL 충돌·정규화) × 4개 신원 × 연결 상태 × 스킴 설정, 표시/그룹 숨김 행렬, 네임스페이스 정책 행렬 |
| URL | Go 1.27.1 `net/url` 파싱·`String()`·`validateURL` 52가지 |
| 설정 | 검증 오류, 해시(버전), HTML/제어 문자 이스케이프, 엄격 JSON 디코딩(명시적 `null` 포함) |
| Git 매니페스트 | 원본의 실제 Git 읽기 경로(`git show`/`ls-tree`, strict YAML, 대상 검사)로 읽은 결과, 인코딩 YAML 바이트, 특이 subject의 상호 읽기 |
| 기타 | 즐겨찾기 검증·상태 해시, 반환 경로, 커밋 메시지 요약, 기존 기본값 검사, 환경 변수 설정, Git 설정 검증 |

`tests/demo.json`은 원본 어댑터의 `demo` 출력과 바이트 단위로 같은지도 매번 확인합니다.

## 의도적으로 다른 부분

Rust가 **더 엄격한** 경우(원본은 받아들이고 Rust는 거부; `compare.py`의 `RUST_STRICTER`에서 방향까지 검사):

1. 설정 JSON 뒤에 다른 데이터가 붙은 경우(Go `Decoder.Decode`는 첫 값만 읽음).
2. 대소문자가 다른 필드 이름(Go `encoding/json`은 대소문자를 무시하고 매칭).
3. Git 매니페스트 경로가 정규화되지 않은 경우(`./x`, `a/../x`, `a//x`, `x/`) 또는 중간에 `.git` 디렉터리가 있는 경우.
   원본은 시작은 했지만 Git 인덱스가 이런 경로를 거부해 저장이 불가능했습니다. Rust는 시작 시 거부합니다.
4. ConfigMap `data`의 숫자 값(sigs.k8s.io/yaml은 대상 타입에 맞춰 문자열로 바꿔 줌). KubeShelf가 쓰는 파일에는 생기지 않습니다.

그 밖의 차이:

- **카드 정렬의 동점**: Go는 맵 순회 + 불안정 정렬이라 소문자 이름이 같은 카드끼리는 순서가 매번 달라집니다. Rust는 항상 같은
  순서입니다. (테스트는 동점 카드만 정규화해 비교합니다.)
- **커밋 메시지 1000바이트 자르기**: Go는 문자 중간에서 자를 수 있어 잘못된 UTF-8이 생깁니다. Rust는 직전 문자 경계에서 자릅니다.
- **특이한 YAML 값**: 평범한 값은 go-yaml과 같은 바이트를 씁니다. 그 외(예: `on`, `12345`, 공백 포함)는 항상 큰따옴표로 써서
  YAML 1.1/1.2 모두에서 문자열로 읽힙니다(go-yaml은 작은따옴표를 쓰는 경우가 있음). 원본이 같은 값으로 읽는 것을 테스트합니다.
- **`KUBESHELF_INGRESS_SCHEMES=null`**: Go는 nil 맵, Rust는 빈 맵(동작은 같음).
- **프런트엔드 제공**: 원본은 바이너리에 포함(`embed`), Rust는 `KUBESHELF_STATIC_DIR`에서 제공합니다. `/assets/` 같은 디렉터리
  목록은 보여 주지 않고 404를 반환합니다(원본 `http.FileServer`는 목록을 보여 줌). `/index.html` 직접 요청도 원본은 `/`로
  리다이렉트하지만 Rust는 파일을 그대로 돌려줍니다.
- **추가 기능**: `KUBESHELF_READ_ONLY`, ID 토큰의 `at_hash` 확인, 헤더 읽기 시간 제한 등은 원본에 없는 보호 장치입니다.
- **로그 문구**: 운영 로그 문장은 원본과 다를 수 있습니다(HTTP 응답 문구는 원본과 같음).
- **버전 문자열**: 빌드 변수 `KUBESHELF_VERSION`(없으면 `dev`).
