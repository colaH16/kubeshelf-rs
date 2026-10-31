//! Data model shared by the HTTP API, the Git store and the catalog.
//!
//! Go → Rust notes:
//! * Each Go struct with `json:"…"` tags becomes a `serde` struct. Field order
//!   is kept identical because the SHA-256 "version" of settings and
//!   favourites is computed over the serialised JSON, exactly like Go.
//! * Go `omitempty` maps to `skip_serializing_if`.
//! * Go maps are serialised with sorted keys, so Rust uses `BTreeMap`.
//! * Go's decoder treats an explicit JSON `null` as "leave the zero value"
//!   (`""`, `false`, empty slice/map). Serde would reject `null` for a
//!   `String`, so every non-`Option` field uses one of the `null_*` helpers.
//! * `deny_unknown_fields` mirrors Go's `DisallowUnknownFields()`; see
//!   [`decode_json`] for the remaining decoder differences that are handled.

use crate::gourl::GoUrl;
use anyhow::{Result, anyhow, ensure};
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

fn is_false(v: &bool) -> bool {
    !*v
}
fn is_zero(v: &i32) -> bool {
    *v == 0
}

/// `null` → `T::default()` (Go leaves the zero value in place).
pub fn null_default<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

/// Slices: a `null` slice is empty and a `null` element is the zero value.
pub fn null_vec<'de, D, T>(d: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    let items = Option::<Vec<Option<T>>>::deserialize(d)?.unwrap_or_default();
    Ok(items.into_iter().map(Option::unwrap_or_default).collect())
}

/// Maps: a `null` map is empty and `{"k": null}` stores the zero value at `k`.
pub fn null_map<'de, D, T>(d: D) -> Result<BTreeMap<String, T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    let items = Option::<BTreeMap<String, Option<T>>>::deserialize(d)?.unwrap_or_default();
    Ok(items
        .into_iter()
        .map(|(k, v)| (k, v.unwrap_or_default()))
        .collect())
}

/// Where nested structs, maps and lists sit inside a JSON document.
///
/// Serde's derived `Deserialize` also accepts a struct written as a JSON
/// *array* of its fields in order (`["public"]` for a `Policy`). Go's
/// decoder rejects that, so documents are checked against this shape first.
/// Only struct-valued positions need listing: arrays anywhere else either
/// fill a real list or fail normal type checking.
pub enum Shape {
    Struct(&'static [(&'static str, Shape)]),
    Map(&'static Shape),
    List(&'static Shape),
}

const POLICY: Shape = Shape::Struct(&[]);
const ENDPOINT_SETTINGS: Shape = Shape::Struct(&[("visibility", POLICY)]);
const APP: Shape = Shape::Struct(&[
    ("visibility", POLICY),
    ("endpoints", Shape::Map(&ENDPOINT_SETTINGS)),
]);
const MANUAL_APP: Shape = Shape::Struct(&[("urls", Shape::List(&Shape::Struct(&[])))]);
const TARGET: Shape = Shape::Struct(&[("visibility", POLICY)]);
pub const SETTINGS_SHAPE: Shape = Shape::Struct(&[
    ("namespaces", Shape::Map(&POLICY)),
    ("nodePortNamespaces", Shape::Map(&POLICY)),
    ("apps", Shape::Map(&APP)),
    ("manual", Shape::List(&MANUAL_APP)),
    ("targets", Shape::List(&TARGET)),
]);
pub const PROFILE_SHAPE: Shape =
    Shape::Struct(&[("collections", Shape::List(&Shape::Struct(&[])))]);
pub const FAVORITE_STATE_SHAPE: Shape = Shape::Struct(&[("profile", PROFILE_SHAPE)]);
pub const FLAT_SHAPE: Shape = Shape::Struct(&[]);

fn matches_shape(v: &serde_json::Value, shape: &Shape) -> bool {
    use serde_json::Value;
    match (v, shape) {
        (Value::Null, _) => true,
        (Value::Object(m), Shape::Struct(fields)) => fields
            .iter()
            .all(|(k, s)| m.get(*k).is_none_or(|v| matches_shape(v, s))),
        (Value::Object(m), Shape::Map(s)) => m.values().all(|v| matches_shape(v, s)),
        (Value::Array(a), Shape::List(s)) => a.iter().all(|v| matches_shape(v, s)),
        // A struct written as an array (or a scalar) is invalid; wrong types
        // for maps/lists are left for serde to reject.
        (_, Shape::Struct(_)) => false,
        _ => true,
    }
}

/// Decode JSON the way Go's `json.Decoder` with `DisallowUnknownFields`
/// does: unknown fields and positional structs are errors, a top-level
/// `null` is the zero value, and duplicate keys keep the last value.
pub fn decode_json<T>(data: &[u8], shape: &Shape) -> Result<T>
where
    T: serde::de::DeserializeOwned + Default,
{
    let value: serde_json::Value = serde_json::from_slice(data)?;
    ensure!(matches_shape(&value, shape), "struct encoded as array");
    Ok(Option::<T>::deserialize(value)?.unwrap_or_default())
}

/// A namespace / card / address visibility rule.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Policy {
    /// Dashboard default for namespace policies; access remains separate.
    #[serde(skip_serializing_if = "is_false", deserialize_with = "null_default")]
    pub hidden: bool,
    #[serde(deserialize_with = "null_default")]
    pub mode: String,
    #[serde(skip_serializing_if = "Vec::is_empty", deserialize_with = "null_vec")]
    pub groups: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", deserialize_with = "null_vec")]
    pub users: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Identity {
    #[serde(deserialize_with = "null_default")]
    pub subject: String,
    #[serde(deserialize_with = "null_default")]
    pub name: String,
    #[serde(deserialize_with = "null_default")]
    pub username: String,
    #[serde(deserialize_with = "null_vec")]
    pub groups: Vec<String>,
    #[serde(deserialize_with = "null_default")]
    pub admin: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EndpointSettings {
    #[serde(
        skip_serializing_if = "String::is_empty",
        deserialize_with = "null_default"
    )]
    pub label: String,
    #[serde(
        skip_serializing_if = "String::is_empty",
        deserialize_with = "null_default"
    )]
    pub url: String,
    #[serde(
        skip_serializing_if = "String::is_empty",
        deserialize_with = "null_default"
    )]
    pub scheme: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<Policy>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct AppSettings {
    #[serde(
        skip_serializing_if = "String::is_empty",
        deserialize_with = "null_default"
    )]
    pub name: String,
    #[serde(
        skip_serializing_if = "String::is_empty",
        deserialize_with = "null_default"
    )]
    pub icon: String,
    #[serde(
        skip_serializing_if = "String::is_empty",
        deserialize_with = "null_default"
    )]
    pub description: String,
    /// Legacy `true` remains an explicit hide (see `display`).
    #[serde(deserialize_with = "null_default")]
    pub hidden: bool,
    #[serde(
        skip_serializing_if = "String::is_empty",
        deserialize_with = "null_default"
    )]
    pub display: String,
    #[serde(
        skip_serializing_if = "String::is_empty",
        deserialize_with = "null_default"
    )]
    pub default_endpoint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<Policy>,
    #[serde(
        skip_serializing_if = "BTreeMap::is_empty",
        deserialize_with = "null_map"
    )]
    pub reviewed: BTreeMap<String, String>,
    #[serde(
        skip_serializing_if = "BTreeMap::is_empty",
        deserialize_with = "null_map"
    )]
    pub endpoints: BTreeMap<String, EndpointSettings>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ManualApp {
    #[serde(deserialize_with = "null_default")]
    pub id: String,
    #[serde(deserialize_with = "null_default")]
    pub name: String,
    #[serde(deserialize_with = "null_vec")]
    pub urls: Vec<ManualURL>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ManualURL {
    #[serde(deserialize_with = "null_default")]
    pub id: String,
    #[serde(deserialize_with = "null_default")]
    pub label: String,
    #[serde(deserialize_with = "null_default")]
    pub url: String,
}

/// A NodePort connection target: a ready node or an admin-defined domain.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct Target {
    #[serde(deserialize_with = "null_default")]
    pub id: String,
    #[serde(deserialize_with = "null_default")]
    pub name: String,
    #[serde(deserialize_with = "null_default")]
    pub host: String,
    #[serde(
        skip_serializing_if = "String::is_empty",
        deserialize_with = "null_default"
    )]
    pub node_name: String,
    #[serde(deserialize_with = "null_default")]
    pub manual: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<Policy>,
    #[serde(deserialize_with = "null_default")]
    pub tcp_enabled: bool,
}

/// The shared settings document stored in Git and applied by Fleet.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct Settings {
    #[serde(deserialize_with = "null_default")]
    pub schema_version: i32,
    #[serde(deserialize_with = "null_default")]
    pub revision: String,
    #[serde(deserialize_with = "null_map")]
    pub namespaces: BTreeMap<String, Policy>,
    #[serde(deserialize_with = "null_map")]
    pub node_port_namespaces: BTreeMap<String, Policy>,
    #[serde(deserialize_with = "null_map")]
    pub apps: BTreeMap<String, AppSettings>,
    #[serde(deserialize_with = "null_vec")]
    pub manual: Vec<ManualApp>,
    #[serde(deserialize_with = "null_vec")]
    pub targets: Vec<Target>,
    #[serde(
        skip_serializing_if = "String::is_empty",
        deserialize_with = "null_default"
    )]
    pub default_target: String,
    #[serde(deserialize_with = "null_map")]
    pub assignments: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct Health {
    #[serde(deserialize_with = "null_default")]
    pub state: String,
    #[serde(deserialize_with = "null_default")]
    pub ready: i32,
    #[serde(deserialize_with = "null_default")]
    pub total: i32,
    #[serde(deserialize_with = "null_default")]
    pub reason: String,
    #[serde(skip_serializing_if = "Vec::is_empty", deserialize_with = "null_vec")]
    pub ready_nodes: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct Connection {
    #[serde(deserialize_with = "null_default")]
    pub target_id: String,
    #[serde(deserialize_with = "null_default")]
    pub name: String,
    #[serde(deserialize_with = "null_default")]
    pub url: String,
    #[serde(deserialize_with = "null_default")]
    pub address: String,
    #[serde(
        skip_serializing_if = "String::is_empty",
        deserialize_with = "null_default"
    )]
    pub node_name: String,
}

/// One reachable address of a service (Ingress rule, NodePort or manual URL).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct Endpoint {
    #[serde(deserialize_with = "null_default")]
    pub id: String,
    #[serde(deserialize_with = "null_default")]
    pub app_id: String,
    #[serde(deserialize_with = "null_default")]
    pub label: String,
    #[serde(deserialize_with = "null_default")]
    pub url: String,
    #[serde(deserialize_with = "null_default")]
    pub kind: String,
    #[serde(
        skip_serializing_if = "String::is_empty",
        deserialize_with = "null_default"
    )]
    pub namespace: String,
    #[serde(skip_serializing_if = "Vec::is_empty", deserialize_with = "null_vec")]
    pub ingresses: Vec<String>,
    #[serde(
        skip_serializing_if = "String::is_empty",
        deserialize_with = "null_default"
    )]
    pub service: String,
    #[serde(
        skip_serializing_if = "String::is_empty",
        deserialize_with = "null_default"
    )]
    pub port: String,
    #[serde(skip_serializing_if = "is_zero", deserialize_with = "null_default")]
    pub node_port: i32,
    #[serde(deserialize_with = "null_default")]
    pub scheme: String,
    #[serde(
        skip_serializing_if = "String::is_empty",
        deserialize_with = "null_default"
    )]
    pub protocol: String,
    #[serde(deserialize_with = "null_default")]
    pub local: bool,
    #[serde(rename = "needsURL", deserialize_with = "null_default")]
    pub needs_url: bool,
    #[serde(deserialize_with = "null_default")]
    pub health: Health,
    #[serde(deserialize_with = "null_vec")]
    pub targets: Vec<Connection>,
    #[serde(
        skip_serializing_if = "String::is_empty",
        deserialize_with = "null_default"
    )]
    pub fingerprint: String,
    #[serde(skip_serializing_if = "is_false", deserialize_with = "null_default")]
    pub hidden: bool,
}

/// A dashboard card: one presentation group of endpoints of the same kind.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct Card {
    #[serde(
        skip_serializing_if = "String::is_empty",
        deserialize_with = "null_default"
    )]
    pub default_endpoint: String,
    #[serde(deserialize_with = "null_default")]
    pub id: String,
    #[serde(deserialize_with = "null_default")]
    pub name: String,
    #[serde(deserialize_with = "null_default")]
    pub icon: String,
    #[serde(deserialize_with = "null_default")]
    pub description: String,
    #[serde(deserialize_with = "null_default")]
    pub source: String,
    #[serde(
        skip_serializing_if = "String::is_empty",
        deserialize_with = "null_default"
    )]
    pub namespace: String,
    #[serde(deserialize_with = "null_default")]
    pub hidden: bool,
    #[serde(deserialize_with = "null_default")]
    pub new: bool,
    #[serde(deserialize_with = "null_default")]
    pub changed: bool,
    #[serde(deserialize_with = "null_vec")]
    pub endpoints: Vec<Endpoint>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct NamespaceInfo {
    #[serde(deserialize_with = "null_default")]
    pub name: String,
    #[serde(deserialize_with = "null_default")]
    pub configured: bool,
    #[serde(deserialize_with = "null_default")]
    pub services: i32,
    #[serde(deserialize_with = "null_default")]
    pub policy: Policy,
    #[serde(deserialize_with = "null_default")]
    pub ingress_configured: bool,
    #[serde(deserialize_with = "null_default")]
    pub ingress_services: i32,
    #[serde(deserialize_with = "null_default")]
    pub node_port_configured: bool,
    #[serde(deserialize_with = "null_default")]
    pub node_port_services: i32,
    #[serde(deserialize_with = "null_default")]
    pub node_port_policy: Policy,
}

/// The `/api/catalog` response, already filtered for one identity.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct Catalog {
    #[serde(
        skip_serializing_if = "String::is_empty",
        deserialize_with = "null_default"
    )]
    pub default_target: String,
    #[serde(deserialize_with = "null_vec")]
    pub cards: Vec<Card>,
    #[serde(deserialize_with = "null_vec")]
    pub targets: Vec<Target>,
    #[serde(skip_serializing_if = "Vec::is_empty", deserialize_with = "null_vec")]
    pub namespaces: Vec<NamespaceInfo>,
    #[serde(deserialize_with = "null_default")]
    pub connected: bool,
    /// Go `time.Time` JSON (RFC 3339 with trimmed nanoseconds), see [`go_time`].
    #[serde(deserialize_with = "null_default")]
    pub updated_at: String,
    #[serde(
        skip_serializing_if = "String::is_empty",
        deserialize_with = "null_default"
    )]
    pub problem: String,
    #[serde(deserialize_with = "null_default")]
    pub identity: Identity,
    #[serde(
        skip_serializing_if = "String::is_empty",
        deserialize_with = "null_default"
    )]
    pub csrf: String,
    #[serde(deserialize_with = "null_default")]
    pub demo: bool,
    #[serde(deserialize_with = "null_default")]
    pub version: String,
    #[serde(deserialize_with = "null_default")]
    pub applied_revision: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FavoriteCollection {
    #[serde(deserialize_with = "null_default")]
    pub id: String,
    #[serde(deserialize_with = "null_default")]
    pub name: String,
    #[serde(deserialize_with = "null_vec")]
    pub cards: Vec<String>,
}

/// One account's personal settings, stored in its own ConfigMap in Git.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct FavoriteProfile {
    #[serde(
        skip_serializing_if = "String::is_empty",
        deserialize_with = "null_default"
    )]
    pub default_target: String,
    #[serde(deserialize_with = "null_map")]
    pub default_endpoints: BTreeMap<String, String>,
    #[serde(deserialize_with = "null_vec")]
    pub collections: Vec<FavoriteCollection>,
    #[serde(deserialize_with = "null_default")]
    pub start_collection: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FavoriteState {
    #[serde(deserialize_with = "null_default")]
    pub profile: FavoriteProfile,
    #[serde(deserialize_with = "null_default")]
    pub version: String,
}

/// A compare-and-swap conflict. HTTP handlers map it to `409 Conflict`
/// (Go: `errors.Is(err, ErrConflict)`).
#[derive(Debug)]
pub struct Conflict(pub &'static str);
impl std::fmt::Display for Conflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for Conflict {}

pub const CONFLICT: &str = "설정이 다른 곳에서 변경됐습니다. 최신 설정을 다시 불러와 주세요";
pub const FAVORITE_CONFLICT: &str =
    "다른 기기에서 모음이 변경됐어요. 최신 모음을 불러온 뒤 다시 저장해 주세요";

/// Go `json.Marshal`: compact JSON with HTML-sensitive characters escaped.
/// serde_json already escapes control characters the same way Go 1.22+ does.
pub fn go_json<T: Serialize>(v: &T) -> Result<Vec<u8>> {
    Ok(go_escape(serde_json::to_string(v)?).into_bytes())
}

/// Go `json.MarshalIndent(v, "", "  ")`, used for files written to Git.
pub fn go_json_indent<T: Serialize>(v: &T) -> Result<String> {
    Ok(go_escape(serde_json::to_string_pretty(v)?))
}

// `<`, `>` and `&` only occur inside JSON strings, so replacing them in the
// encoded text is equivalent to Go's per-string HTML escaping.
fn go_escape(json: String) -> String {
    json.replace('&', "\\u0026")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

/// SHA-256 over the Go JSON encoding. Used as the compare-and-swap token for
/// the shared settings file and for each personal favourites file.
pub fn version<T: Serialize>(v: &T) -> String {
    hex::encode(Sha256::digest(go_json(v).expect("serializable model")))
}

/// Go `stableID`: first 12 bytes of SHA-256 over NUL-joined parts.
pub fn stable_id(parts: &[&str]) -> String {
    hex::encode(&Sha256::digest(parts.join("\0").as_bytes())[..12])
}

/// Go `time.Time` JSON form (RFC 3339, nanoseconds without trailing zeros).
pub fn go_time(t: chrono::DateTime<chrono::Utc>) -> String {
    use chrono::Timelike;
    let mut out = t.format("%Y-%m-%dT%H:%M:%S").to_string();
    let nanos = t.nanosecond() % 1_000_000_000;
    if nanos > 0 {
        out.push_str(format!(".{nanos:09}").trim_end_matches('0'));
    }
    out.push('Z');
    out
}

/// Current time in Go `time.Time` JSON form.
pub fn now() -> String {
    go_time(chrono::Utc::now())
}

/// Go's zero `time.Time`, shown before the first successful cluster sync.
pub const ZERO_TIME: &str = "0001-01-01T00:00:00Z";

/// Go `strings.ToLower` (simple per-rune case mapping, no context rules such
/// as Greek final sigma, and U+0130 maps to plain `i`).
pub fn go_lower(s: &str) -> String {
    s.chars()
        .flat_map(|c| {
            let lower: Vec<char> = if c == '\u{130}' {
                vec!['i']
            } else {
                c.to_lowercase().collect()
            };
            lower
        })
        .collect()
}

impl Policy {
    pub fn admin() -> Self {
        Self {
            mode: "admin".into(),
            ..Self::default()
        }
    }

    /// Go `Policy.Allows`: admins see everything, `public` is open, and
    /// `restricted` needs a logged-in subject listed directly or via a group.
    pub fn allows(&self, user: &Identity) -> bool {
        if user.admin || self.mode == "public" {
            return true;
        }
        if self.mode != "restricted" || user.subject.is_empty() {
            return false;
        }
        self.users.contains(&user.subject) || user.groups.iter().any(|g| self.groups.contains(g))
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            ["public", "restricted", "admin"].contains(&self.mode.as_str()),
            "invalid visibility policy"
        );
        ensure!(
            self.users.len() + self.groups.len() <= 1000,
            "too many permission entries"
        );
        ensure!(
            self.users
                .iter()
                .chain(&self.groups)
                .all(|x| !x.is_empty() && x.len() <= 256),
            "invalid permission identity"
        );
        Ok(())
    }
}

/// Go `validateURL`: an absolute HTTP(S) URL with a host and no userinfo.
pub fn validate_url(raw: &str) -> Result<GoUrl> {
    let invalid = || anyhow!("valid HTTP or HTTPS address required");
    let u = GoUrl::parse(raw).map_err(|_| invalid())?;
    ensure!(
        (u.scheme == "http" || u.scheme == "https")
            && !u.hostname().is_empty()
            && u.user.is_none()
            && !raw.contains(['\r', '\n', '\t', '\\']),
        invalid()
    );
    Ok(u)
}

/// Go `validateHost`: an IP address or a DNS hostname without scheme/port.
pub fn validate_host(h: &str) -> Result<()> {
    if h.parse::<std::net::IpAddr>().is_ok() {
        return Ok(());
    }
    ensure!(
        !h.is_empty()
            && h.len() <= 253
            && !h.contains(['/', ':', '@', '?', '#', '\\', ' ', '\r', '\n', '\t']),
        "hostname without scheme or port required"
    );
    for label in h.split('.') {
        ensure!(
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-'),
            "invalid hostname"
        );
    }
    Ok(())
}

fn validate_policy(p: Option<&Policy>) -> Result<()> {
    p.map_or(Ok(()), Policy::validate)
}

impl Settings {
    /// Go `EmptySettings`.
    pub fn empty() -> Self {
        Self {
            schema_version: 1,
            revision: "initial".into(),
            ..Self::default()
        }
    }

    /// Go `ValidateSettings`. (Go also replaces nil maps/slices with empty
    /// ones here; Rust collections are never nil, so nothing to normalise.)
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == 1 && !self.revision.is_empty() && self.revision.len() <= 100,
            "invalid settings version"
        );
        ensure!(self.default_target.len() <= 300, "invalid default target");
        ensure!(
            self.apps.len() <= 10000
                && self.manual.len() <= 1000
                && self.targets.len() <= 100
                && self.assignments.len() <= 10000,
            "too many settings entries"
        );
        for (ns, p) in &self.namespaces {
            ensure!(!ns.is_empty() && ns.len() <= 253, "invalid namespace");
            p.validate()?;
        }
        for (ns, p) in &self.node_port_namespaces {
            ensure!(
                !ns.is_empty() && ns.len() <= 253,
                "invalid NodePort namespace"
            );
            p.validate()?;
        }
        for (id, a) in &self.apps {
            ensure!(
                !id.is_empty()
                    && id.len() <= 100
                    && a.name.len() <= 200
                    && a.description.len() <= 2000
                    && a.icon.len() <= 2048,
                "invalid card settings"
            );
            ensure!(a.default_endpoint.len() <= 100, "invalid default address");
            ensure!(
                ["", "inherit", "show", "hide"].contains(&a.display.as_str()),
                "invalid dashboard display setting"
            );
            validate_policy(a.visibility.as_ref())?;
            if a.icon.contains("://") {
                validate_url(&a.icon)?;
            }
            for (eid, e) in &a.endpoints {
                ensure!(
                    !eid.is_empty() && eid.len() <= 100 && e.label.len() <= 200,
                    "invalid endpoint settings"
                );
                if !e.url.is_empty() {
                    validate_url(&e.url)?;
                }
                ensure!(
                    ["", "http", "https", "tcp"].contains(&e.scheme.as_str()),
                    "invalid protocol"
                );
                validate_policy(e.visibility.as_ref())?;
            }
        }
        let mut ids = BTreeSet::new();
        for m in &self.manual {
            ensure!(
                m.id.starts_with("manual-")
                    && ids.insert(&m.id)
                    && !m.name.is_empty()
                    && m.name.len() <= 200
                    && !m.urls.is_empty()
                    && m.urls.len() <= 50,
                "invalid manual service"
            );
            let mut seen = BTreeSet::new();
            for u in &m.urls {
                ensure!(
                    !u.id.is_empty()
                        && seen.insert(&u.id)
                        && u.id.len() <= 100
                        && u.label.len() <= 200,
                    "invalid manual address"
                );
                validate_url(&u.url)?;
            }
        }
        ids.clear();
        for t in &self.targets {
            ensure!(
                t.id.starts_with("domain-")
                    && ids.insert(&t.id)
                    && !t.name.is_empty()
                    && t.name.len() <= 200,
                "invalid node domain"
            );
            validate_host(&t.host)?;
            validate_policy(t.visibility.as_ref())?;
        }
        ensure!(
            self.assignments
                .iter()
                .all(|(e, g)| !e.is_empty() && e.len() <= 100 && !g.is_empty() && g.len() <= 100),
            "invalid card grouping"
        );
        ensure!(
            go_json(self)?.len() <= 650000,
            "settings exceed ConfigMap size budget"
        );
        Ok(())
    }

    /// Go `decodeSettings`: strict JSON (unknown fields rejected), then validation.
    pub fn decode(data: &[u8]) -> Result<Self> {
        let s: Self = decode_json(data, &SETTINGS_SHAPE)
            .map_err(|_| anyhow!("설정 파일 형식이 올바르지 않습니다"))?;
        s.validate()?;
        Ok(s)
    }
}

/// Go `collectionIDPattern` = `^[a-zA-Z0-9_-]{1,80}$`.
pub fn valid_collection_id(id: &str) -> bool {
    (1..=80).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

impl FavoriteProfile {
    /// Go `defaultFavorites`: admins get three collections, others one.
    pub fn for_user(admin: bool) -> Self {
        let defaults: &[(&str, &str)] = if admin {
            &[
                ("daily", "일상"),
                ("management", "관리"),
                ("monitoring", "모니터링"),
            ]
        } else {
            &[("favorites", "즐겨찾기")]
        };
        Self {
            collections: defaults
                .iter()
                .map(|(id, name)| FavoriteCollection {
                    id: (*id).into(),
                    name: (*name).into(),
                    cards: vec![],
                })
                .collect(),
            ..Self::default()
        }
    }

    /// Go `favoriteState`: non-admin profiles collapse into one sorted
    /// "즐겨찾기" collection, then the version hash is taken.
    pub fn state(mut self, user: &Identity) -> FavoriteState {
        if !user.admin {
            let cards: BTreeSet<String> = self
                .collections
                .iter()
                .flat_map(|c| c.cards.iter().cloned())
                .collect();
            self.collections = vec![FavoriteCollection {
                id: "favorites".into(),
                name: "즐겨찾기".into(),
                cards: cards.into_iter().collect(),
            }];
            if !self.start_collection.is_empty() {
                self.start_collection = "favorites".into();
            }
        }
        FavoriteState {
            version: version(&self),
            profile: self,
        }
    }

    /// Go `validateFavoriteProfile` (also trims collection names in place).
    pub fn validate(&mut self) -> Result<()> {
        ensure!(
            self.default_target.len() <= 300 && self.default_endpoints.len() <= 2000,
            "기본 주소 설정이 너무 많습니다"
        );
        ensure!(
            self.default_endpoints
                .iter()
                .all(|(c, e)| !c.is_empty() && !e.is_empty() && c.len() <= 100 && e.len() <= 100),
            "기본 주소 설정이 올바르지 않습니다"
        );
        ensure!(
            self.collections.len() <= 20,
            "모음은 최대 20개까지 만들 수 있습니다"
        );
        let (mut ids, mut names) = (BTreeSet::new(), BTreeSet::new());
        let mut references = 0;
        for c in &mut self.collections {
            c.name = c.name.trim().into();
            ensure!(
                valid_collection_id(&c.id)
                    && ids.insert(c.id.clone())
                    && !c.name.is_empty()
                    && c.name.chars().count() <= 40
                    && !c.name.chars().any(char::is_control)
                    && names.insert(go_lower(&c.name)),
                "모음 이름은 중복 없이 1~40자로 입력해 주세요"
            );
            let mut seen = BTreeSet::new();
            ensure!(
                c.cards
                    .iter()
                    .all(|id| !id.is_empty() && id.len() <= 100 && seen.insert(id)),
                "즐겨찾기 서비스 목록이 올바르지 않습니다"
            );
            references += c.cards.len();
        }
        ensure!(references <= 2000, "즐겨찾기 항목이 너무 많습니다");
        ensure!(
            self.start_collection.is_empty() || ids.contains(&self.start_collection),
            "시작 모음을 선택해 주세요"
        );
        Ok(())
    }
}

/// Only application views may be served as SPA pages or used after login.
pub fn is_view(p: &str) -> bool {
    ["/", "/favorites", "/discovery", "/namespaces", "/hidden"].contains(&p)
}

/// Go `safeReturnTo`: an allow-listed same-origin view path, else `/`.
/// Like Go, the allow-list check uses the percent-decoded path.
pub fn safe_return_to(raw: &str) -> String {
    let Ok(u) = GoUrl::parse(raw) else {
        return "/".into();
    };
    let path = String::from_utf8_lossy(&u.path);
    if !u.scheme.is_empty()
        || !u.opaque.is_empty()
        || !u.host.is_empty()
        || u.user.is_some()
        || !u.fragment.is_empty()
        || !is_view(&path)
    {
        return "/".into();
    }
    if u.raw_query.is_empty() {
        return path.into_owned();
    }
    // Go `url.ParseQuery` rejects `;` and malformed escapes.
    if u.raw_query.contains(';') {
        return "/".into();
    }
    let mut pairs = Vec::new();
    for part in u.raw_query.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = part.split_once('=').unwrap_or((part, ""));
        match (query_unescape(k), query_unescape(v)) {
            (Some(k), Some(v)) => pairs.push((k, v)),
            _ => return "/".into(),
        }
    }
    let keys: BTreeSet<&str> = pairs.iter().map(|(k, _)| k.as_str()).collect();
    if path == "/favorites"
        && keys.len() == 1
        && pairs.len() == 1
        && pairs[0].0 == "collection"
        && valid_collection_id(&pairs[0].1)
    {
        // The id pattern is URL-safe, so Go's QueryEscape leaves it unchanged.
        return format!("/favorites?collection={}", pairs[0].1);
    }
    "/".into()
}

/// Go `url.QueryUnescape` (`+` is a space; malformed `%` escapes fail).
fn query_unescape(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' => {
                let hex = s.get(i + 1..i + 3)?;
                if !hex.bytes().all(|h| h.is_ascii_hexdigit()) {
                    return None;
                }
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    Some(String::from_utf8_lossy(&out).into_owned())
}
