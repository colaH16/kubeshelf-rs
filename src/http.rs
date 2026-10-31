//! HTTP API and static frontend (Go `server.go`, `routes.go`, `favorites.go`).
//!
//! Routes, methods, status codes and bodies follow the Go server:
//! JSON API responses are Go `json.Encoder` output (HTML-escaped, trailing
//! newline); the auth endpoints answer errors with Go `http.Error`-style
//! plain text. Every response carries the same no-store and security headers.
//!
//! Authorisation order per endpoint (Go `account` / `admin` wrappers):
//! identity → role (401 for anonymous personal routes, 403 for non-admins) →
//! for writes: same `Origin` and the session CSRF header (403) → body.

// Handlers return `Result<Response, Response>`: both arms are complete HTTP
// responses, so the large `Err` variant is intended.
#![allow(clippy::result_large_err)]

use crate::{
    auth::{Auth, AuthError, Directory},
    catalog,
    config::Config,
    model::*,
    source::Source,
    store::Store,
};
use anyhow::Result;
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::json;
use std::{future::Future, sync::Arc, time::Duration};
use tower_http::services::ServeDir;

/// Shared application state handed to every handler.
#[derive(Clone)]
pub struct App {
    pub cfg: Config,
    pub store: Arc<Store>,
    pub source: Arc<Source>,
    pub auth: Arc<Auth>,
    pub directory: Arc<Directory>,
}

/// Version string shown in the UI (Go `-ldflags -X main.version`).
pub const VERSION: &str = match option_env!("KUBESHELF_VERSION") {
    Some(v) => v,
    None => "dev",
};

const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' https: data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'";

type ApiResult = std::result::Result<Response, Response>;

/// Go `writeJSON`.
fn json_response<T: Serialize>(status: u16, value: &T) -> Response {
    let mut body = go_json(value).expect("serializable response");
    body.push(b'\n');
    let mut r = Response::new(Body::from(body));
    *r.status_mut() = StatusCode::from_u16(status).expect("valid status");
    r.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    r
}

/// Go `writeError`: `{"error": "..."}`.
fn api_error(status: u16, message: impl std::fmt::Display) -> Response {
    json_response(status, &json!({"error": message.to_string()}))
}

/// Go `http.Error`: plain text with a trailing newline.
fn plain_error(status: u16, message: &str) -> Response {
    let mut r = Response::new(Body::from(format!("{message}\n")));
    *r.status_mut() = StatusCode::from_u16(status).expect("valid status");
    r.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    r
}

impl From<AuthError> for Response {
    fn from(e: AuthError) -> Self {
        plain_error(e.status, &e.message)
    }
}

/// Go `decodeBody`: JSON content type, at most 800 000 bytes, unknown fields
/// rejected, exactly one JSON value. A literal `null` decodes to the zero value.
async fn decode_body<T: DeserializeOwned + Default>(
    request: Request,
    shape: &Shape,
) -> std::result::Result<T, &'static str> {
    let content_type = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default();
    if !content_type.starts_with("application/json") {
        return Err("JSON request required");
    }
    let bytes = to_bytes(request.into_body(), 800_000)
        .await
        .map_err(|_| "요청 형식이 올바르지 않습니다")?;
    decode_json(&bytes, shape).map_err(|_| "요청 형식이 올바르지 않습니다")
}

/// Same-origin + CSRF header check for state-changing requests.
fn csrf_ok(app: &App, h: &HeaderMap, csrf: &str) -> bool {
    app.auth.same_origin(h)
        && !csrf.is_empty()
        && h.get("X-CSRF-Token").and_then(|v| v.to_str().ok()) == Some(csrf)
}

/// Go `Server.account`: any logged-in account.
async fn account(app: &App, h: &HeaderMap, write: bool) -> std::result::Result<Identity, Response> {
    let (user, csrf) = app.auth.identity(h).await;
    if user.subject.is_empty() {
        return Err(api_error(401, "로그인이 필요합니다"));
    }
    if write && !csrf_ok(app, h, &csrf) {
        return Err(api_error(403, "요청 검증에 실패했습니다"));
    }
    if write && app.cfg.read_only {
        return Err(api_error(403, "read-only validation mode"));
    }
    Ok(user)
}

/// Go `Server.admin`: configured admin subjects only (403 even when anonymous).
async fn admin(app: &App, h: &HeaderMap, write: bool) -> std::result::Result<Identity, Response> {
    let (user, csrf) = app.auth.identity(h).await;
    if !user.admin {
        return Err(api_error(403, "관리자만 사용할 수 있습니다"));
    }
    if write && !csrf_ok(app, h, &csrf) {
        return Err(api_error(403, "요청 검증에 실패했습니다"));
    }
    if write && app.cfg.read_only {
        return Err(api_error(403, "read-only validation mode"));
    }
    Ok(user)
}

/// Run `op` with a deadline (Go `context.WithTimeout`).
async fn bounded<T>(seconds: u64, op: impl Future<Output = Result<T>>) -> Result<T> {
    tokio::time::timeout(Duration::from_secs(seconds), op)
        .await
        .map_err(|_| anyhow::anyhow!("작업 시간이 초과됐습니다"))?
}

/// 409 for compare-and-swap conflicts, else `fallback`.
fn conflict_or(e: &anyhow::Error, fallback: u16) -> u16 {
    if e.downcast_ref::<Conflict>().is_some() {
        409
    } else {
        fallback
    }
}

async fn security_headers(request: Request, next: Next) -> Response {
    // This one static, public link manifest can be read by the other cola16 sites.
    // API/auth responses retain their original same-origin boundary and headers.
    let public_navigation = request.uri().path() == "/branding/navigation.json"
        && matches!(
            *request.method(),
            axum::http::Method::GET | axum::http::Method::HEAD
        );
    let mut response = next.run(request).await;
    let h = response.headers_mut();
    for (k, v) in [
        ("cache-control", "private, no-store, max-age=0"),
        ("cdn-cache-control", "no-store"),
        ("cloudflare-cdn-cache-control", "no-store"),
        ("pragma", "no-cache"),
        ("vary", "Cookie"),
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "no-referrer"),
        ("x-frame-options", "DENY"),
        ("content-security-policy", CSP),
    ] {
        h.insert(k, HeaderValue::from_static(v));
    }
    if public_navigation {
        h.insert(
            header::ACCESS_CONTROL_ALLOW_ORIGIN,
            HeaderValue::from_static("*"),
        );
    }
    response
}

pub fn router(app: App) -> Router {
    let mut r = Router::new()
        .route("/healthz", get(|| async { "ok\n" }))
        .route("/readyz", get(ready))
        .route("/auth/login", get(login))
        .route("/auth/callback", get(callback))
        .route("/auth/logout", post(logout))
        .route("/api/catalog", get(get_catalog))
        .route("/api/me/favorites", get(get_favorites).put(put_favorites))
        .route("/api/admin/state", get(admin_state).put(save_state))
        .route("/api/admin/status", get(admin_status))
        .route("/api/admin/tcp", post(check_tcp));
    if app.cfg.demo {
        r = r.route("/auth/demo", post(demo_login));
    }
    for view in ["/", "/favorites", "/discovery", "/namespaces", "/hidden"] {
        r = r.route(view, get(index));
    }
    // Everything else: hashed frontend assets (404 if absent).
    r.fallback_service(ServeDir::new(&app.cfg.static_dir))
        .layer(middleware::from_fn(security_headers))
        .layer(tower_http::timeout::RequestBodyTimeoutLayer::new(
            Duration::from_secs(15),
        ))
        .layer(tower_http::timeout::TimeoutLayer::with_status_code(
            StatusCode::SERVICE_UNAVAILABLE,
            Duration::from_secs(60),
        ))
        .with_state(app)
}

async fn ready(State(app): State<App>) -> Response {
    if app.store.applied().await.revision.is_empty() {
        plain_error(503, "settings unavailable")
    } else {
        "ready\n".into_response()
    }
}

async fn index(State(app): State<App>) -> Response {
    match tokio::fs::read(std::path::Path::new(&app.cfg.static_dir).join("index.html")).await {
        Ok(b) => ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], b).into_response(),
        Err(_) => plain_error(503, "frontend not built"),
    }
}

async fn get_catalog(State(app): State<App>, h: HeaderMap) -> Response {
    let (user, csrf) = app.auth.identity(&h).await;
    let snapshot = app.source.snapshot().await;
    let mut cat = catalog::build(&snapshot, &app.store.applied().await, &app.cfg, &user);
    cat.csrf = csrf;
    cat.version = VERSION.into();
    json_response(200, &cat)
}

async fn get_favorites(State(app): State<App>, h: HeaderMap) -> ApiResult {
    let user = account(&app, &h, false).await?;
    let state = bounded(45, app.store.favorites(&user))
        .await
        .map_err(|e| api_error(503, e))?;
    Ok(json_response(200, &state))
}

async fn put_favorites(State(app): State<App>, r: Request) -> ApiResult {
    let user = account(&app, r.headers(), true).await?;
    let request: FavoriteState = decode_body(r, &FAVORITE_STATE_SHAPE)
        .await
        .map_err(|e| api_error(400, e))?;
    // New defaults are checked against what this user can see *now*.
    let snapshot = app.source.snapshot().await;
    let accessible = catalog::build(&snapshot, &app.store.applied().await, &app.cfg, &user);
    let saved = bounded(
        45,
        app.store
            .save_favorites(&user, &request.version, request.profile, &accessible),
    )
    .await
    .map_err(|e| api_error(conflict_or(&e, 400), e))?;
    Ok(json_response(200, &saved))
}

async fn admin_status(State(app): State<App>, h: HeaderMap) -> ApiResult {
    admin(&app, &h, false).await?;
    Ok(json_response(200, &app.store.status().await))
}

async fn admin_state(State(app): State<App>, h: HeaderMap) -> ApiResult {
    let user = admin(&app, &h, false).await?;
    bounded(30, app.store.refresh())
        .await
        .map_err(|e| api_error(503, e))?;
    let (settings, base_commit) = app.store.desired().await;
    let directory = bounded(30, app.directory.load())
        .await
        .map_err(|e| api_error(503, e))?;
    // The admin preview uses the *desired* settings.
    let snapshot = app.source.snapshot().await;
    let cat = catalog::build(&snapshot, &settings, &app.cfg, &user);
    Ok(json_response(
        200,
        &json!({"settings": settings, "baseCommit": base_commit, "status": app.store.status().await, "catalog": cat, "directory": directory}),
    ))
}

const SAVE_REQUEST_SHAPE: Shape = Shape::Struct(&[("settings", SETTINGS_SHAPE)]);

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
struct SaveRequest {
    #[serde(deserialize_with = "null_default")]
    base_commit: String,
    #[serde(deserialize_with = "null_vec")]
    review_endpoints: Vec<String>,
    #[serde(deserialize_with = "null_default")]
    settings: Settings,
}

/// Go `sameLegacyDefaults`: legacy shared default addresses are read-only
/// here (they moved to personal settings).
pub fn same_legacy_defaults(a: &Settings, b: &Settings) -> bool {
    let default_of = |s: &Settings, id: &str| {
        s.apps
            .get(id)
            .map(|x| x.default_endpoint.clone())
            .unwrap_or_default()
    };
    a.default_target == b.default_target
        && a.apps
            .iter()
            .all(|(id, x)| x.default_endpoint == default_of(b, id))
        && b.apps
            .iter()
            .all(|(id, x)| x.default_endpoint == default_of(a, id))
}

async fn save_state(State(app): State<App>, r: Request) -> ApiResult {
    let user = admin(&app, r.headers(), true).await?;
    let mut req: SaveRequest = decode_body(r, &SAVE_REQUEST_SHAPE)
        .await
        .map_err(|e| api_error(400, e))?;
    req.settings.validate().map_err(|e| api_error(400, e))?;
    if !same_legacy_defaults(&app.store.desired().await.0, &req.settings) {
        return Err(api_error(
            400,
            "기본 접속 주소는 개인 설정에서 저장합니다. 페이지를 새로고침해 주세요",
        ));
    }
    if req.review_endpoints.len() > 1000 {
        return Err(api_error(400, "too many review entries"));
    }
    if !req.review_endpoints.is_empty() {
        // Record the current fingerprint of each reviewed discovered endpoint.
        let snapshot = app.source.snapshot().await;
        let cat = catalog::build(&snapshot, &req.settings, &app.cfg, &user);
        for e in cat.cards.into_iter().flat_map(|c| c.endpoints) {
            if e.kind != "custom" && req.review_endpoints.contains(&e.id) {
                let app_settings = req.settings.apps.entry(e.app_id).or_default();
                app_settings.reviewed.insert(e.id, e.fingerprint);
            }
        }
    }
    let status = bounded(
        45,
        app.store
            .save(&req.base_commit, req.settings, &user.username),
    )
    .await
    .map_err(|e| api_error(conflict_or(&e, 400), e))?;
    let (settings, base_commit) = app.store.desired().await;
    Ok(json_response(
        200,
        &json!({"status": status, "baseCommit": base_commit, "settings": settings}),
    ))
}

/// First value of a query parameter (Go `r.URL.Query().Get`).
fn query(r: &Request, key: &str) -> String {
    url::form_urlencoded::parse(r.uri().query().unwrap_or_default().as_bytes())
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
        .unwrap_or_default()
}

fn redirect(status: StatusCode, location: &str, cookies: &[String]) -> Response {
    let mut r = status.into_response();
    if let Ok(v) = HeaderValue::from_str(location) {
        r.headers_mut().insert(header::LOCATION, v);
    }
    for c in cookies {
        if let Ok(v) = HeaderValue::from_str(c) {
            r.headers_mut().append(header::SET_COOKIE, v);
        }
    }
    r
}

/// Demo administrator (Go `Auth.Login` in demo mode).
fn demo_identity(admin: bool) -> Identity {
    let (subject, username, name, group) = if admin {
        ("demo-admin", "admin", "Administrator", "operators")
    } else {
        ("demo-family", "family", "Family", "family")
    };
    Identity {
        subject: subject.into(),
        username: username.into(),
        name: name.into(),
        groups: vec![group.into()],
        admin,
    }
}

async fn login(State(app): State<App>, r: Request) -> ApiResult {
    let return_to = safe_return_to(&query(&r, "returnTo"));
    if app.cfg.demo {
        let (location, cookie) = app
            .auth
            .new_session(r.headers(), demo_identity(true), &return_to)
            .await?;
        return Ok(redirect(StatusCode::SEE_OTHER, &location, &[cookie]));
    }
    let (location, cookie) = app.auth.login(&return_to).await?;
    Ok(redirect(StatusCode::FOUND, &location, &[cookie]))
}

async fn callback(State(app): State<App>, r: Request) -> ApiResult {
    if app.cfg.demo {
        return Err(plain_error(404, "404 page not found"));
    }
    let (state, code, headers) = (query(&r, "state"), query(&r, "code"), r.headers().clone());
    if !app.auth.state_matches(&headers, &state) {
        return Err(plain_error(400, "로그인 요청이 일치하지 않습니다"));
    }
    // From here on the login cookie is always cleared (Go order).
    let clear = app.auth.cookie(&app.auth.login_cookie_name(), "", 0);
    let result = async {
        let (user, return_to) = app.auth.callback(&state, &code).await?;
        app.auth.new_session(&headers, user, &return_to).await
    }
    .await;
    match result {
        Ok((location, cookie)) => Ok(redirect(StatusCode::SEE_OTHER, &location, &[clear, cookie])),
        Err(e) => {
            let mut response = Response::from(e);
            if let Ok(v) = HeaderValue::from_str(&clear) {
                response.headers_mut().append(header::SET_COOKIE, v);
            }
            Err(response)
        }
    }
}

async fn logout(State(app): State<App>, h: HeaderMap) -> ApiResult {
    let (_, csrf) = app.auth.identity(&h).await;
    // Without a live session there is no CSRF token; Origin is still required.
    let header_ok = csrf.is_empty()
        || h.get("X-CSRF-Token").and_then(|v| v.to_str().ok()) == Some(csrf.as_str());
    if !app.auth.same_origin(&h) || !header_ok {
        return Err(plain_error(403, "요청 검증에 실패했습니다"));
    }
    app.auth.logout(&h).await;
    let mut r = StatusCode::NO_CONTENT.into_response();
    let clear = app.auth.cookie(app.auth.cookie_name(), "", 0);
    if let Ok(v) = HeaderValue::from_str(&clear) {
        r.headers_mut().append(header::SET_COOKIE, v);
    }
    Ok(r)
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct DemoRequest {
    #[serde(deserialize_with = "null_default")]
    role: String,
}

async fn demo_login(State(app): State<App>, r: Request) -> ApiResult {
    if !app.auth.same_origin(r.headers()) {
        return Err(plain_error(403, "invalid origin"));
    }
    let headers = r.headers().clone();
    let request: DemoRequest = decode_body(r, &FLAT_SHAPE)
        .await
        .map_err(|_| plain_error(400, "invalid request"))?;
    let user = demo_identity(request.role == "admin");
    let (location, cookie) = app.auth.new_session(&headers, user, "/").await?;
    Ok(redirect(StatusCode::SEE_OTHER, &location, &[cookie]))
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
struct TcpRequest {
    #[serde(deserialize_with = "null_default")]
    endpoint_id: String,
    #[serde(deserialize_with = "null_default")]
    target_id: String,
}

/// Addresses the server-side TCP probe must never connect to (Go:
/// loopback, unspecified, link-local unicast/multicast, multicast).
fn forbidden(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v) => {
            v.is_loopback() || v.is_unspecified() || v.is_link_local() || v.is_multicast()
        }
        std::net::IpAddr::V6(v) => {
            v.to_ipv4_mapped().is_some_and(|v4| forbidden(v4.into()))
                || v.is_loopback()
                || v.is_unspecified()
                || v.is_unicast_link_local()
                || v.is_multicast()
        }
    }
}

/// Go `checkTCP`: an admin probe from the server to a manual, TCP-enabled
/// domain, only for a NodePort that the catalog actually offers there.
async fn check_tcp(State(app): State<App>, r: Request) -> ApiResult {
    let user = admin(&app, r.headers(), true).await?;
    let req: TcpRequest = decode_body(r, &FLAT_SHAPE)
        .await
        .map_err(|e| api_error(400, e))?;
    let settings = app.store.applied().await;
    let Some(target) = settings
        .targets
        .iter()
        .find(|t| t.id == req.target_id && t.tcp_enabled)
    else {
        return Err(api_error(
            400,
            "TCP 확인이 허용된 수동 도메인을 선택해 주세요",
        ));
    };
    let snapshot = app.source.snapshot().await;
    let cat = catalog::build(&snapshot, &settings, &app.cfg, &user);
    let port = cat
        .cards
        .iter()
        .flat_map(|c| &c.endpoints)
        .filter(|e| e.id == req.endpoint_id && (e.protocol == "TCP" || e.protocol.is_empty()))
        .filter(|e| e.targets.iter().any(|t| t.target_id == req.target_id))
        .map(|e| e.node_port)
        .next_back() // Go keeps the last match
        .filter(|p| (1..=65535).contains(p));
    let Some(port) = port else {
        return Err(api_error(400, "확인 가능한 NodePort가 없습니다"));
    };
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    let lookup = tokio::net::lookup_host((target.host.as_str(), port as u16));
    let addresses: Vec<_> = match tokio::time::timeout_at(deadline, lookup).await {
        Ok(Ok(found)) => found.collect(),
        _ => vec![],
    };
    if addresses.is_empty() {
        return Ok(json_response(
            200,
            &json!({"reachable": false, "message": "DNS 조회 실패", "checkedAt": now()}),
        ));
    }
    if addresses.iter().any(|a| forbidden(a.ip())) {
        return Err(api_error(400, "이 주소는 TCP 확인 대상에서 제외됩니다"));
    }
    let mut reachable = false;
    for address in addresses {
        let limit = deadline.min(tokio::time::Instant::now() + Duration::from_secs(1));
        if let Ok(Ok(_)) =
            tokio::time::timeout_at(limit, tokio::net::TcpStream::connect(address)).await
        {
            reachable = true;
            break;
        }
    }
    let message = if reachable {
        "서버에서 TCP 연결 성공"
    } else {
        "서버에서 TCP 연결 실패"
    };
    Ok(json_response(
        200,
        &json!({"reachable": reachable, "message": message, "checkedAt": now()}),
    ))
}

/// Serve HTTP/1.1 until `shutdown` resolves, then drain in-flight requests
/// for up to 10 s (Go `http.Server` with `Shutdown`). Like the Go server it
/// limits header read time (5 s) and header size (64 KiB).
pub async fn serve(
    listener: tokio::net::TcpListener,
    app: Router,
    shutdown: impl Future<Output = ()>,
) {
    use hyper_util::{
        rt::{TokioIo, TokioTimer},
        server::graceful::GracefulShutdown,
        service::TowerToHyperService,
    };
    let graceful = GracefulShutdown::new();
    tokio::pin!(shutdown);
    loop {
        let stream = tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => stream,
                Err(e) => {
                    // e.g. EMFILE: back off instead of spinning.
                    tracing::warn!("accept failed: {e}");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
            },
            () = &mut shutdown => break,
        };
        let connection = hyper::server::conn::http1::Builder::new()
            .timer(TokioTimer::new())
            .header_read_timeout(Duration::from_secs(5))
            .max_buf_size((1 << 16) + 4096)
            .serve_connection(TokioIo::new(stream), TowerToHyperService::new(app.clone()));
        let watched = graceful.watch(connection);
        tokio::spawn(async move {
            let _ = watched.await;
        });
    }
    drop(listener);
    if tokio::time::timeout(Duration::from_secs(10), graceful.shutdown())
        .await
        .is_err()
    {
        tracing::warn!("shutdown: in-flight requests did not finish within 10 s");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;

    #[tokio::test]
    async fn cross_origin_reads_are_limited_to_the_public_navigation_file() {
        let app = Router::new()
            .fallback(|| async { "public fixture" })
            .layer(middleware::from_fn(security_headers));
        for (method, path, allowed) in [
            ("GET", "/branding/navigation.json", true),
            ("HEAD", "/branding/navigation.json", true),
            ("GET", "/branding/navigation.json?v=1", true),
            ("POST", "/branding/navigation.json", false),
            ("GET", "/api/catalog", false),
            ("GET", "/api/me/favorites", false),
            ("GET", "/auth/login", false),
            ("GET", "/branding/navigation.json/other", false),
        ] {
            let request = Request::builder()
                .method(method)
                .uri(path)
                .header(header::ORIGIN, "https://rag.cola16.app")
                .body(Body::empty())
                .unwrap();
            let response = app.clone().oneshot(request).await.unwrap();
            assert_eq!(
                response
                    .headers()
                    .contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN),
                allowed
            );
            assert!(
                !response
                    .headers()
                    .contains_key(header::ACCESS_CONTROL_ALLOW_CREDENTIALS)
            );
            assert_eq!(
                response.headers()[header::CACHE_CONTROL],
                "private, no-store, max-age=0"
            );
        }
    }

    async fn fixture() -> (App, Router) {
        let cfg = Config::from_lookup(|k, d| {
            if k == "KUBESHELF_DEMO" {
                "true".into()
            } else {
                d.into()
            }
        })
        .unwrap();
        let store = Store::open(cfg.clone(), crate::source::demo().1)
            .await
            .unwrap();
        let source = Source::open(&cfg).await.unwrap();
        let directory = Directory::new(cfg.clone()).unwrap();
        let auth = Auth::new(cfg.clone(), directory.clone()).unwrap();
        let app = App {
            cfg,
            store,
            source,
            auth,
            directory,
        };
        let router = router(app.clone());
        (app, router)
    }

    async fn call(
        router: Router,
        method: &str,
        path: &str,
        cookie: &str,
        csrf: &str,
        body: &str,
    ) -> Response {
        let request = Request::builder()
            .method(method)
            .uri(path)
            .header("Cookie", cookie)
            .header("Origin", "http://localhost:8080")
            .header("X-CSRF-Token", csrf)
            .header("Content-Type", "application/json")
            .body(Body::from(body.to_owned()))
            .unwrap();
        router.oneshot(request).await.unwrap()
    }

    async fn text(response: Response) -> String {
        String::from_utf8(
            to_bytes(response.into_body(), 10_000_000)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn roles_csrf_strict_json_and_readonly() {
        let (app, router) = fixture().await;
        assert_eq!(
            call(router.clone(), "GET", "/api/admin/state", "", "", "")
                .await
                .status(),
            403
        );
        assert_eq!(
            call(router.clone(), "GET", "/api/me/favorites", "", "", "")
                .await
                .status(),
            401
        );
        let login = call(
            router.clone(),
            "POST",
            "/auth/demo",
            "",
            "",
            r#"{"role":"admin"}"#,
        )
        .await;
        assert_eq!(login.status(), 303);
        let cookie = login.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_owned();
        let mut h = HeaderMap::new();
        h.insert(header::COOKIE, cookie.parse().unwrap());
        let (_, csrf) = app.auth.identity(&h).await;
        assert_eq!(
            call(router.clone(), "PUT", "/api/admin/state", &cookie, "", "{}")
                .await
                .status(),
            403
        );
        assert_eq!(
            call(
                router.clone(),
                "PUT",
                "/api/admin/state",
                &cookie,
                &csrf,
                r#"{"unknown":true}"#
            )
            .await
            .status(),
            400
        );
        let tcp = call(
            router.clone(),
            "POST",
            "/api/admin/tcp",
            &cookie,
            &csrf,
            r#"{"endpointId":"x","targetId":"node1"}"#,
        )
        .await;
        assert_eq!(tcp.status(), 400);
        let mut ro = app.clone();
        ro.cfg.read_only = true;
        assert_eq!(
            call(
                super::router(ro),
                "PUT",
                "/api/me/favorites",
                &cookie,
                &csrf,
                "{}"
            )
            .await
            .status(),
            403
        );
        let response = call(router.clone(), "GET", "/api/catalog", "", "", "").await;
        assert_eq!(
            response.headers()["Cache-Control"],
            "private, no-store, max-age=0"
        );
        assert_eq!(
            response.headers()["Content-Type"],
            "application/json; charset=utf-8"
        );
        let body = text(response).await;
        assert!(body.ends_with('\n'));
        assert!(
            !body.contains("192.0.2."),
            "anonymous NodePort addresses must be filtered"
        );
        let wrong_origin = Request::builder()
            .method("POST")
            .uri("/auth/demo")
            .header("Origin", "https://evil.example")
            .header("Content-Type", "application/json")
            .body(Body::from("{}"))
            .unwrap();
        let denied = router.clone().oneshot(wrong_origin).await.unwrap();
        assert_eq!(denied.status(), 403);
        assert_eq!(text(denied).await, "invalid origin\n");
        assert_eq!(
            text(call(router, "GET", "/readyz", "", "", "").await).await,
            "ready\n"
        );
    }

    #[test]
    fn tcp_denies_local_addresses_including_mapped_ipv4() {
        for ip in [
            "127.0.0.1",
            "::1",
            "0.0.0.0",
            "169.254.169.254",
            "fe80::1",
            "224.0.0.1",
            "::ffff:127.0.0.1",
        ] {
            assert!(forbidden(ip.parse().unwrap()), "{ip}");
        }
        assert!(!forbidden("192.0.2.11".parse().unwrap()));
    }
}
