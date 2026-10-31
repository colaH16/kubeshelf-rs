//! Authentication (Go `auth.go`).
//!
//! * OIDC authorization-code flow with PKCE (S256), state bound to a
//!   short-lived cookie, nonce, and full ID-token verification (signature
//!   against the provider JWKS, issuer, audience, expiry). Accepted signing
//!   algorithms follow go-oidc: asymmetric algorithms advertised by the
//!   provider, defaulting to RS256; HMAC and `none` are never accepted.
//! * Sessions are in memory (one replica), one hour, with a per-session CSRF
//!   token. Every request re-checks the user against the directory (cached
//!   30 s): disabled or removed accounts lose their session immediately.
//! * The directory is the Authentik-compatible `/api/v3/core/users/` and
//!   `/groups/` API, read with a bearer token from a mounted file.

use crate::{config::Config, model::*, store::random_id};
use anyhow::{Context, Result, anyhow, ensure};
use axum::http::{HeaderMap, header};
use openidconnect::{
    AccessTokenHash, AuthorizationCode, ClientId, ClientSecret, CsrfToken, IssuerUrl, Nonce,
    OAuth2TokenResponse, PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, Scope, TokenResponse,
    core::{CoreAuthenticationFlow, CoreClient, CoreJwsSigningAlgorithm, CoreProviderMetadata},
    reqwest,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

/// An error with the HTTP status the Go handler used (`http.Error`).
#[derive(Debug)]
pub struct AuthError {
    pub status: u16,
    pub message: String,
}
impl AuthError {
    fn new(status: u16, message: impl std::fmt::Display) -> Self {
        Self {
            status,
            message: message.to_string(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DirectoryUser {
    pub subject: String,
    pub username: String,
    pub name: String,
    pub groups: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct DirectoryData {
    pub users: Vec<DirectoryUser>,
    pub groups: Vec<String>,
}

// Typed views of the directory API responses. Like Go's json.Unmarshal into
// a struct, unknown fields are ignored but wrong types are errors.
#[derive(Default, Deserialize)]
#[serde(default)]
struct Pagination {
    #[serde(deserialize_with = "null_default")]
    next: i64,
}
#[derive(Default, Deserialize)]
#[serde(default, bound(deserialize = "T: Deserialize<'de> + Default"))]
struct Page<T: Default> {
    #[serde(deserialize_with = "null_default")]
    pagination: Pagination,
    #[serde(deserialize_with = "null_vec")]
    results: Vec<T>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct UserRow {
    #[serde(deserialize_with = "null_default")]
    uuid: String,
    #[serde(deserialize_with = "null_default")]
    username: String,
    #[serde(deserialize_with = "null_default")]
    name: String,
    #[serde(rename = "is_active", deserialize_with = "null_default")]
    active: bool,
    #[serde(rename = "type", deserialize_with = "null_default")]
    kind: String,
    #[serde(rename = "groups_obj", deserialize_with = "null_vec")]
    groups: Vec<GroupRow>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct GroupRow {
    #[serde(deserialize_with = "null_default")]
    name: String,
}

pub struct Directory {
    cfg: Config,
    client: reqwest::Client,
    /// Holding this lock while loading serialises refreshes, as in Go.
    cache: Mutex<Option<(Instant, DirectoryData)>>,
}

impl Directory {
    pub fn new(cfg: Config) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            cfg,
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            cache: Mutex::new(None),
        }))
    }

    /// GET one page; `failure` is the message for transport/status errors.
    async fn page<T: Default + serde::de::DeserializeOwned>(
        &self,
        kind: &str,
        page: u32,
        token: &str,
        failure: &'static str,
        connect: &'static str,
    ) -> Result<Page<T>> {
        let url = format!(
            "{}/api/v3/core/{kind}/?page_size=100&page={page}",
            self.cfg.directory_url.trim_end_matches('/')
        );
        let mut response = self
            .client
            .get(url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|_| anyhow!(connect))?;
        ensure!(response.status() == reqwest::StatusCode::OK, failure);
        // Read at most 4 MiB, like io.LimitReader in Go.
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| anyhow!(failure))? {
            ensure!(
                body.len() + chunk.len() <= 4 << 20,
                "사용자 디렉터리 응답 오류"
            );
            body.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&body).map_err(|_| anyhow!("사용자 디렉터리 응답 오류"))
    }

    /// Go `Directory.Load`: active, non-service accounts plus all groups.
    pub async fn load(&self) -> Result<DirectoryData> {
        if self.cfg.demo {
            let user = |subject: &str, username: &str, name: &str, group: &str| DirectoryUser {
                subject: subject.into(),
                username: username.into(),
                name: name.into(),
                groups: vec![group.into()],
            };
            return Ok(DirectoryData {
                users: vec![
                    user("demo-admin", "admin", "Administrator", "operators"),
                    user("demo-family", "family", "Family", "family"),
                ],
                groups: vec!["operators".into(), "family".into()],
            });
        }
        let mut cache = self.cache.lock().await;
        if let Some((expires, data)) = &*cache
            && Instant::now() < *expires
        {
            return Ok(data.clone());
        }
        ensure!(
            !self.cfg.directory_url.is_empty(),
            "사용자 디렉터리가 구성되지 않았습니다"
        );
        let token = tokio::fs::read_to_string(&self.cfg.directory_token_file)
            .await
            .map_err(|_| anyhow!("사용자 디렉터리 인증 정보를 읽을 수 없습니다"))?;
        let token = token.trim();
        let mut data = DirectoryData::default();
        let mut groups = BTreeSet::new();
        for page in 1..=100 {
            let list: Page<UserRow> = self
                .page(
                    "users",
                    page,
                    token,
                    "사용자 디렉터리 조회에 실패했습니다",
                    "사용자 디렉터리에 연결할 수 없습니다",
                )
                .await?;
            for u in list.results {
                if !u.active || u.uuid.is_empty() || u.kind.contains("service_account") {
                    continue;
                }
                let names: Vec<String> = u.groups.into_iter().map(|g| g.name).collect();
                groups.extend(names.iter().cloned());
                data.users.push(DirectoryUser {
                    subject: u.uuid,
                    username: u.username,
                    name: u.name,
                    groups: names,
                });
            }
            if list.pagination.next == 0 {
                break;
            }
            ensure!(page < 100, "사용자 디렉터리가 너무 큽니다");
        }
        // Include empty groups too, so permissions can be assigned before membership.
        for page in 1..=100 {
            let list: Page<GroupRow> = self
                .page(
                    "groups",
                    page,
                    token,
                    "그룹 디렉터리 조회 실패",
                    "그룹 디렉터리 조회 실패",
                )
                .await?;
            groups.extend(list.results.into_iter().map(|g| g.name));
            if list.pagination.next == 0 {
                break;
            }
            ensure!(page < 100, "그룹 디렉터리가 너무 큽니다");
        }
        data.groups = groups.into_iter().collect();
        data.users.sort_by(|a, b| a.username.cmp(&b.username));
        *cache = Some((Instant::now() + Duration::from_secs(30), data.clone()));
        Ok(data)
    }

    pub fn identity(&self, u: &DirectoryUser) -> Identity {
        Identity {
            subject: u.subject.clone(),
            username: u.username.clone(),
            name: u.name.clone(),
            groups: u.groups.clone(),
            admin: self.cfg.admin_subjects.contains(&u.subject),
        }
    }
}

struct Session {
    identity: Identity,
    csrf: String,
    expires: Instant,
}

struct Attempt {
    nonce: String,
    verifier: String,
    return_to: String,
    expires: Instant,
}

pub struct Auth {
    pub cfg: Config,
    pub directory: Arc<Directory>,
    client: reqwest::Client,
    sessions: Mutex<BTreeMap<String, Session>>,
    attempts: Mutex<BTreeMap<String, Attempt>>,
    provider: Mutex<Option<(Instant, CoreProviderMetadata)>>,
}

/// Algorithms go-oidc accepts when the provider advertises them.
const ASYMMETRIC_ALGS: &[CoreJwsSigningAlgorithm] = &[
    CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256,
    CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha384,
    CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha512,
    CoreJwsSigningAlgorithm::EcdsaP256Sha256,
    CoreJwsSigningAlgorithm::EcdsaP384Sha384,
    CoreJwsSigningAlgorithm::EcdsaP521Sha512,
    CoreJwsSigningAlgorithm::RsaSsaPssSha256,
    CoreJwsSigningAlgorithm::RsaSsaPssSha384,
    CoreJwsSigningAlgorithm::RsaSsaPssSha512,
    CoreJwsSigningAlgorithm::EdDsa,
];

impl Auth {
    pub fn new(cfg: Config, directory: Arc<Directory>) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            cfg,
            directory,
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(12))
                .build()?,
            sessions: Mutex::new(BTreeMap::new()),
            attempts: Mutex::new(BTreeMap::new()),
            provider: Mutex::new(None),
        }))
    }

    /// `__Host-` cookies require Secure + Path=/ (not possible over demo HTTP).
    pub fn cookie_name(&self) -> &'static str {
        if self.cfg.demo {
            "kubeshelf-demo"
        } else {
            "__Host-kubeshelf"
        }
    }

    pub fn login_cookie_name(&self) -> String {
        format!("{}-login", self.cookie_name())
    }

    /// A `Set-Cookie` value. `age == 0` deletes the cookie (Go `MaxAge: -1`).
    pub fn cookie(&self, name: &str, value: &str, age: u32) -> String {
        let secure = if self.cfg.demo { "" } else { "; Secure" };
        format!("{name}={value}; Path=/; Max-Age={age}; HttpOnly{secure}; SameSite=Lax")
    }

    /// First cookie named `name` (Go `r.Cookie`).
    fn cookie_value<'a>(&self, headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
        headers
            .get_all(header::COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .flat_map(|v| v.split(';'))
            .find_map(|pair| {
                let (k, v) = pair.trim().split_once('=')?;
                (k == name).then(|| v.trim_matches('"'))
            })
    }

    /// Go `SameOrigin`: the `Origin` header must equal the public origin.
    pub fn same_origin(&self, headers: &HeaderMap) -> bool {
        headers.get(header::ORIGIN).and_then(|v| v.to_str().ok())
            == Some(self.cfg.public_url.as_str())
    }

    /// Go `Auth.Identity`: the session user, re-validated against the
    /// directory, plus the session CSRF token. Anonymous if anything fails.
    pub async fn identity(&self, headers: &HeaderMap) -> (Identity, String) {
        let anonymous = (Identity::default(), String::new());
        let Some(id) = self.cookie_value(headers, self.cookie_name()) else {
            return anonymous;
        };
        let (identity, csrf) = {
            let mut sessions = self.sessions.lock().await;
            if sessions.get(id).is_some_and(|s| Instant::now() > s.expires) {
                sessions.remove(id);
            }
            let Some(s) = sessions.get(id) else {
                return anonymous;
            };
            (s.identity.clone(), s.csrf.clone())
        };
        if self.cfg.demo {
            return (identity, csrf);
        }
        let Ok(data) = self.directory.load().await else {
            return anonymous; // directory down: fail closed, keep the session
        };
        match data.users.iter().find(|u| u.subject == identity.subject) {
            Some(u) => (self.directory.identity(u), csrf),
            None => {
                self.sessions.lock().await.remove(id);
                anonymous
            }
        }
    }

    /// Go `newSession`: replace any current session, return (redirect target,
    /// Set-Cookie value).
    pub async fn new_session(
        &self,
        headers: &HeaderMap,
        user: Identity,
        return_to: &str,
    ) -> Result<(String, String), AuthError> {
        let mut sessions = self.sessions.lock().await;
        if let Some(old) = self.cookie_value(headers, self.cookie_name()) {
            sessions.remove(old);
        }
        let now = Instant::now();
        sessions.retain(|_, s| now <= s.expires);
        if sessions.len() >= 10000 {
            return Err(AuthError::new(503, "로그인 세션이 너무 많습니다"));
        }
        let id = random_id();
        sessions.insert(
            id.clone(),
            Session {
                identity: user,
                csrf: random_id(),
                expires: now + Duration::from_secs(3600),
            },
        );
        Ok((
            safe_return_to(return_to),
            self.cookie(self.cookie_name(), &id, 3600),
        ))
    }

    /// Provider discovery (including JWKS), cached for five minutes.
    async fn metadata(&self, force: bool) -> Result<CoreProviderMetadata> {
        let mut cache = self.provider.lock().await;
        if !force
            && let Some((expires, p)) = &*cache
            && Instant::now() < *expires
        {
            return Ok(p.clone());
        }
        let issuer = IssuerUrl::new(self.cfg.oidc_issuer.clone())
            .context("SSO 서버에 연결할 수 없습니다")?;
        let p = CoreProviderMetadata::discover_async(issuer, &self.client)
            .await
            .map_err(|_| anyhow!("SSO 서버에 연결할 수 없습니다"))?;
        *cache = Some((Instant::now() + Duration::from_secs(300), p.clone()));
        Ok(p)
    }

    async fn secret(&self) -> Result<ClientSecret> {
        let secret = tokio::fs::read_to_string(&self.cfg.oidc_secret_file)
            .await
            .map_err(|_| anyhow!("SSO 인증 정보를 읽을 수 없습니다"))?;
        Ok(ClientSecret::new(secret.trim().into()))
    }

    fn redirect_url(&self) -> Result<RedirectUrl> {
        Ok(RedirectUrl::new(format!(
            "{}/auth/callback",
            self.cfg.public_url
        ))?)
    }

    /// Go `Auth.Login`: returns (authorization URL, login-state cookie).
    pub async fn login(&self, return_to: &str) -> Result<(String, String), AuthError> {
        let setup = async {
            let client = CoreClient::from_provider_metadata(
                self.metadata(false).await?,
                ClientId::new(self.cfg.oidc_client_id.clone()),
                Some(self.secret().await?),
            )
            .set_redirect_uri(self.redirect_url()?);
            Ok::<_, anyhow::Error>(client)
        };
        let client = setup.await.map_err(|e| AuthError::new(503, e))?;
        let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
        let (state, nonce) = (random_id(), random_id());
        {
            let mut attempts = self.attempts.lock().await;
            let now = Instant::now();
            attempts.retain(|_, a| now <= a.expires);
            if attempts.len() >= 2000 {
                return Err(AuthError::new(429, "잠시 후 다시 시도해 주세요"));
            }
            attempts.insert(
                state.clone(),
                Attempt {
                    nonce: nonce.clone(),
                    verifier: verifier.secret().clone(),
                    return_to: safe_return_to(return_to),
                    expires: now + Duration::from_secs(300),
                },
            );
        }
        let state_for_url = state.clone();
        let (url, _, _) = client
            .authorize_url(
                CoreAuthenticationFlow::AuthorizationCode,
                move || CsrfToken::new(state_for_url),
                move || Nonce::new(nonce),
            )
            .add_scope(Scope::new("profile".into()))
            .add_scope(Scope::new("email".into()))
            .set_pkce_challenge(challenge)
            .url();
        Ok((
            url.into(),
            self.cookie(&self.login_cookie_name(), &state, 300),
        ))
    }

    /// The callback `state` must match the login cookie set by [`Auth::login`].
    pub fn state_matches(&self, headers: &HeaderMap, state: &str) -> bool {
        !state.is_empty() && self.cookie_value(headers, &self.login_cookie_name()) == Some(state)
    }

    /// Go `Auth.Callback` after the state check: consume the attempt, redeem
    /// the code with the PKCE verifier, verify the ID token, and require an
    /// active directory account. Returns (identity, return path).
    pub async fn callback(&self, state: &str, code: &str) -> Result<(Identity, String), AuthError> {
        let pending = self.attempts.lock().await.remove(state);
        let pending = match pending {
            Some(p) if Instant::now() <= p.expires => p,
            _ => return Err(AuthError::new(400, "로그인 요청이 만료됐습니다")),
        };
        let setup = async {
            let metadata = self.metadata(false).await?;
            let secret = self.secret().await?;
            Ok::<_, anyhow::Error>((metadata, secret, self.redirect_url()?))
        };
        let (metadata, secret, redirect) = setup.await.map_err(|e| AuthError::new(503, e))?;
        let client = CoreClient::from_provider_metadata(
            metadata,
            ClientId::new(self.cfg.oidc_client_id.clone()),
            Some(secret),
        )
        .set_redirect_uri(redirect);
        let exchange_failed = || AuthError::new(401, "SSO 코드 확인에 실패했습니다");
        let token = client
            .exchange_code(AuthorizationCode::new(code.into()))
            .map_err(|_| exchange_failed())?
            .set_pkce_verifier(PkceCodeVerifier::new(pending.verifier))
            .request_async(&self.client)
            .await
            .map_err(|_| exchange_failed())?;
        let id_token = token
            .id_token()
            .ok_or_else(|| AuthError::new(401, "SSO 토큰이 없습니다"))?;
        let nonce = Nonce::new(pending.nonce);
        let mut subject = None;
        // On failure, re-discover once: the provider may have rotated keys.
        for attempt in 0..2 {
            let Ok(metadata) = self.metadata(attempt == 1).await else {
                break;
            };
            let mut algs: Vec<CoreJwsSigningAlgorithm> = metadata
                .id_token_signing_alg_values_supported()
                .iter()
                .filter(|a| ASYMMETRIC_ALGS.contains(a))
                .cloned()
                .collect();
            if algs.is_empty() {
                algs.push(CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256);
            }
            // A public-client verifier never uses the client secret as an HMAC key.
            let client = CoreClient::from_provider_metadata(
                metadata,
                ClientId::new(self.cfg.oidc_client_id.clone()),
                None,
            );
            let verifier = client
                .id_token_verifier()
                .set_allowed_algs(algs)
                // go-oidc requires our client id in `aud` but allows others.
                .set_other_audience_verifier_fn(|_| true);
            let Ok(claims) = id_token.claims(&verifier, &nonce) else {
                continue;
            };
            if let Some(expected) = claims.access_token_hash() {
                // Optional `at_hash` binding (stricter than the Go original).
                let matches = id_token
                    .signing_alg()
                    .ok()
                    .zip(id_token.signing_key(&verifier).ok())
                    .and_then(|(alg, key)| {
                        AccessTokenHash::from_token(token.access_token(), alg, key).ok()
                    })
                    .is_some_and(|actual| actual == *expected);
                if !matches {
                    break;
                }
            }
            subject = Some(claims.subject().as_str().to_owned());
            break;
        }
        let subject = subject.ok_or_else(|| AuthError::new(401, "SSO 토큰 검증에 실패했습니다"))?;
        let data = self
            .directory
            .load()
            .await
            .map_err(|e| AuthError::new(503, e))?;
        match data.users.iter().find(|u| u.subject == subject) {
            Some(u) => Ok((self.directory.identity(u), pending.return_to)),
            None => Err(AuthError::new(
                403,
                "활성화된 사용자만 로그인할 수 있습니다",
            )),
        }
    }

    /// Drop the current session (Go `Logout`, after its CSRF checks).
    pub async fn logout(&self, headers: &HeaderMap) {
        if let Some(id) = self.cookie_value(headers, self.cookie_name()) {
            self.sessions.lock().await.remove(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Form, Json, Router,
        extract::State,
        routing::{get, post},
    };
    use openidconnect::{
        AccessToken, Audience, EmptyAdditionalClaims, JsonWebKeyId, PrivateSigningKey,
        StandardClaims, SubjectIdentifier,
        core::{CoreHmacKey, CoreIdToken, CoreIdTokenClaims, CoreRsaPrivateSigningKey},
    };
    use serde_json::{Value, json};
    use std::sync::atomic::{AtomicBool, Ordering};

    /// A local OIDC provider + directory. `mode` selects one misbehaviour.
    struct Mock {
        issuer: String,
        key: CoreRsaPrivateSigningKey,
        other: CoreRsaPrivateSigningKey,
        mode: Mutex<String>,
        nonce: Mutex<String>,
        challenge: Mutex<String>,
        active: AtomicBool,
    }

    fn rsa_key(kid: &str) -> CoreRsaPrivateSigningKey {
        let output = std::process::Command::new("openssl")
            .args(["genrsa", "-traditional", "2048"])
            .output()
            .unwrap();
        assert!(output.status.success());
        CoreRsaPrivateSigningKey::from_pem(
            std::str::from_utf8(&output.stdout).unwrap(),
            Some(JsonWebKeyId::new(kid.into())),
        )
        .unwrap()
    }

    async fn discovery(State(m): State<Arc<Mock>>) -> Json<Value> {
        let algs = if *m.mode.lock().await == "hmac" {
            json!(["HS256"])
        } else {
            json!(["RS256"])
        };
        Json(json!({
            "issuer": m.issuer,
            "authorization_endpoint": format!("{}/authorize", m.issuer),
            "token_endpoint": format!("{}/token", m.issuer),
            "jwks_uri": format!("{}/jwks", m.issuer),
            "response_types_supported": ["code"],
            "subject_types_supported": ["public"],
            "id_token_signing_alg_values_supported": algs,
            "token_endpoint_auth_methods_supported": ["client_secret_basic"],
        }))
    }

    async fn jwks(State(m): State<Arc<Mock>>) -> Json<Value> {
        let key = if *m.mode.lock().await == "rotation" {
            &m.other
        } else {
            &m.key
        };
        Json(json!({"keys": [key.as_verification_key()]}))
    }

    async fn token(
        State(m): State<Arc<Mock>>,
        Form(f): Form<BTreeMap<String, String>>,
    ) -> Json<Value> {
        assert_eq!(f["grant_type"], "authorization_code");
        assert_eq!(f["code"], "test-code");
        let verifier = PkceCodeVerifier::new(f["code_verifier"].clone());
        let challenge = PkceCodeChallenge::from_code_verifier_sha256(&verifier);
        assert_eq!(
            challenge.as_str(),
            m.challenge.lock().await.as_str(),
            "PKCE verifier must match"
        );
        let mode = m.mode.lock().await.clone();
        let issuer = if mode == "issuer" {
            "https://wrong.example.test"
        } else {
            &m.issuer
        };
        let audience = if mode == "audience" {
            "other-client"
        } else {
            "client"
        };
        let nonce = if mode == "nonce" {
            "wrong".into()
        } else {
            m.nonce.lock().await.clone()
        };
        let lifetime = if mode == "expired" { -300 } else { 300 };
        let claims = CoreIdTokenClaims::new(
            IssuerUrl::new(issuer.into()).unwrap(),
            vec![Audience::new(audience.into())],
            chrono::Utc::now() + chrono::Duration::seconds(lifetime),
            chrono::Utc::now(),
            StandardClaims::new(SubjectIdentifier::new("active-user".into())),
            EmptyAdditionalClaims {},
        )
        .set_nonce(Some(Nonce::new(nonce)));
        let access = AccessToken::new("synthetic-access".into());
        let id = if mode == "hmac" {
            // An HS256 token keyed with the client secret must never be accepted.
            let key = CoreHmacKey::new("synthetic-secret");
            CoreIdToken::new(
                claims,
                &key,
                CoreJwsSigningAlgorithm::HmacSha256,
                Some(&access),
                None,
            )
            .unwrap()
        } else {
            let signing = if ["signature", "rotation"].contains(&mode.as_str()) {
                &m.other
            } else {
                &m.key
            };
            CoreIdToken::new(
                claims,
                signing,
                CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256,
                Some(&access),
                None,
            )
            .unwrap()
        };
        Json(
            json!({"access_token": access.secret(), "token_type": "Bearer", "id_token": id.to_string(), "expires_in": 300}),
        )
    }

    async fn users(State(m): State<Arc<Mock>>) -> Json<Value> {
        Json(json!({"pagination": {"next": 0}, "results": [
            {"uuid": "active-user", "username": "alice", "name": "Alice", "is_active": m.active.load(Ordering::Relaxed), "type": "internal", "groups_obj": [{"name": "family"}]},
            {"uuid": "service", "username": "robot", "is_active": true, "type": "service_account", "groups_obj": []},
        ]}))
    }

    async fn begin(auth: &Auth, m: &Mock) -> (HeaderMap, String) {
        let (url, cookie) = auth.login("/favorites?collection=daily").await.unwrap();
        let url = url::Url::parse(&url).unwrap();
        let q: BTreeMap<_, _> = url.query_pairs().collect();
        *m.nonce.lock().await = q["nonce"].to_string();
        *m.challenge.lock().await = q["code_challenge"].to_string();
        assert_eq!(q["code_challenge_method"], "S256");
        let mut h = HeaderMap::new();
        h.insert(
            header::COOKIE,
            cookie.split(';').next().unwrap().parse().unwrap(),
        );
        (h, q["state"].to_string())
    }

    #[tokio::test]
    async fn oidc_pkce_signature_nonce_audience_expiry_rotation_and_directory_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("secret"), "synthetic-secret").unwrap();
        std::fs::write(dir.path().join("directory-token"), "synthetic-token").unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let issuer = format!("http://{}", listener.local_addr().unwrap());
        let mock = Arc::new(Mock {
            issuer: issuer.clone(),
            key: rsa_key("one"),
            other: rsa_key("two"),
            mode: Mutex::new("valid".into()),
            nonce: Mutex::new(String::new()),
            challenge: Mutex::new(String::new()),
            active: AtomicBool::new(true),
        });
        let router = Router::new()
            .route("/.well-known/openid-configuration", get(discovery))
            .route("/jwks", get(jwks))
            .route("/token", post(token))
            .route("/api/v3/core/users/", get(users))
            .route(
                "/api/v3/core/groups/",
                get(|| async {
                    Json(json!({"pagination": {"next": 0}, "results": [{"name": "empty-group"}]}))
                }),
            )
            .with_state(mock.clone());
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

        let mut cfg = Config::from_lookup(|k, d| {
            if k == "KUBESHELF_DEMO" {
                "true".into()
            } else {
                d.into()
            }
        })
        .unwrap();
        cfg.demo = false;
        cfg.public_url = "https://shelf.example.test".into();
        cfg.oidc_issuer = issuer.clone();
        cfg.directory_url = issuer;
        cfg.oidc_client_id = "client".into();
        cfg.oidc_secret_file = dir.path().join("secret").to_string_lossy().into();
        cfg.directory_token_file = dir.path().join("directory-token").to_string_lossy().into();
        cfg.admin_subjects = vec!["active-user".into()];
        let directory = Directory::new(cfg.clone()).unwrap();
        let auth = Auth::new(cfg, directory.clone()).unwrap();

        // Happy path; the attempt is single-use and bound to the state cookie.
        let (h, state) = begin(&auth, &mock).await;
        assert!(
            !auth.state_matches(&HeaderMap::new(), &state),
            "cookie binding"
        );
        assert!(auth.state_matches(&h, &state));
        let (u, dest) = auth.callback(&state, "test-code").await.unwrap();
        assert!(u.admin);
        assert_eq!(dest, "/favorites?collection=daily");
        assert_eq!(
            auth.callback(&state, "test-code").await.unwrap_err().status,
            400,
            "attempt is single-use"
        );

        let data = directory.load().await.unwrap();
        assert_eq!(
            data.users.len(),
            1,
            "inactive and service accounts are excluded"
        );
        assert!(data.groups.contains(&"empty-group".into()));

        for mode in [
            "nonce",
            "audience",
            "issuer",
            "expired",
            "signature",
            "hmac",
        ] {
            *mock.mode.lock().await = mode.into();
            *auth.provider.lock().await = None;
            let (_, state) = begin(&auth, &mock).await;
            assert_eq!(
                auth.callback(&state, "test-code").await.unwrap_err().status,
                401,
                "must reject {mode}"
            );
        }

        // Signing key rotation: the cached JWKS is stale, one re-discovery fixes it.
        *mock.mode.lock().await = "valid".into();
        *auth.provider.lock().await = None;
        begin(&auth, &mock).await;
        *mock.mode.lock().await = "rotation".into();
        let (_, state) = begin(&auth, &mock).await;
        let (u, _) = auth.callback(&state, "test-code").await.unwrap();

        // Disabling the account in the directory ends the session.
        let (_, cookie) = auth.new_session(&HeaderMap::new(), u, "/").await.unwrap();
        let mut session = HeaderMap::new();
        session.insert(
            header::COOKIE,
            cookie.split(';').next().unwrap().parse().unwrap(),
        );
        assert!(!auth.identity(&session).await.0.subject.is_empty());
        mock.active.store(false, Ordering::Relaxed);
        *directory.cache.lock().await = None;
        assert!(
            auth.identity(&session).await.0.subject.is_empty(),
            "disabled user loses session"
        );
        mock.active.store(true, Ordering::Relaxed);
        *directory.cache.lock().await = None;
        assert!(
            auth.identity(&session).await.0.subject.is_empty(),
            "session was removed, not just hidden"
        );
        task.abort();
    }
}
