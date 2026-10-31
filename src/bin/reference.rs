//! Differential-test counterpart of `tests/go-reference/main.go`.
//!
//! Reads one JSON test case from stdin and prints the Rust implementation's
//! answer in the same JSON shape as the Go adapter. `tests/compare.py` runs
//! both and requires identical output. Not used at runtime.

use kubeshelf_rs::{
    catalog::{Snapshot, build},
    config::Config,
    git,
    gourl::GoUrl,
    http::same_legacy_defaults,
    model::*,
    store::{change_summary, personal_summary},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::BTreeMap, io::Read};

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
struct Input {
    op: String,
    snapshot: Snapshot,
    settings: Settings,
    old: Settings,
    identity: Identity,
    profile: FavoriteProfile,
    before: FavoriteProfile,
    route: String,
    raw: String,
    #[serde(rename = "GitURL")]
    git_url: String,
    branch: String,
    git_path: String,
    env: BTreeMap<String, String>,
    schemes: Option<BTreeMap<String, String>>,
}

fn demo_config() -> Config {
    Config::from_lookup(|k, d| {
        if k == "KUBESHELF_DEMO" {
            "true".into()
        } else {
            d.into()
        }
    })
    .expect("demo config")
}

fn error_or<T>(result: anyhow::Result<T>, ok: impl FnOnce(T) -> Value) -> Value {
    match result {
        Ok(v) => ok(v),
        Err(e) => json!({"error": e.to_string()}),
    }
}

/// Go `Config` has no JSON tags, so it encodes with Go field names.
fn config_json(c: &Config) -> Value {
    json!({
        "Listen": c.listen, "PublicURL": c.public_url, "Cluster": c.cluster, "Kubeconfig": c.kubeconfig,
        "SettingsFile": c.settings_file, "WorkDir": c.work_dir, "GitURL": c.git_url, "GitBranch": c.git_branch,
        "GitPath": c.git_path, "SSHKeyFile": c.ssh_key_file, "KnownHostsFile": c.known_hosts_file,
        "OIDCIssuer": c.oidc_issuer, "OIDCClientID": c.oidc_client_id, "OIDCSecretFile": c.oidc_secret_file,
        "DirectoryURL": c.directory_url, "DirectoryTokenFile": c.directory_token_file,
        "AdminSubjects": if c.admin_subjects.is_empty() { Value::Null } else { json!(c.admin_subjects) },
        "IngressSchemes": c.ingress_schemes, "Demo": c.demo,
    })
}

fn main() {
    let mut raw = String::new();
    std::io::stdin().read_to_string(&mut raw).expect("stdin");
    let mut i: Input = serde_json::from_str(&raw).expect("test case JSON");
    let mut c = demo_config();
    c.ingress_schemes = i
        .schemes
        .take()
        .unwrap_or_else(|| BTreeMap::from([("demo-https".into(), "https".into())]));
    let value = match i.op.as_str() {
        "catalog" => {
            serde_json::to_value(build(&i.snapshot, &i.settings, &c, &i.identity)).unwrap()
        }
        "settings" => error_or(
            i.settings.validate(),
            |()| json!({"settings": i.settings, "version": version(&i.settings)}),
        ),
        "decode-settings" => error_or(
            Settings::decode(i.raw.as_bytes()),
            |s| json!({"settings": s, "version": version(&s)}),
        ),
        "favorites" => error_or(i.profile.validate(), |()| {
            serde_json::to_value(i.profile.clone().state(&i.identity)).unwrap()
        }),
        "default-favorites" => {
            serde_json::to_value(FavoriteProfile::for_user(i.identity.admin).state(&i.identity))
                .unwrap()
        }
        "route" => json!(safe_return_to(&i.route)),
        "url" => {
            let mut out = json!({"valid": validate_url(&i.raw).is_ok()});
            if let Ok(mut u) = GoUrl::parse(&i.raw) {
                out["string"] = json!(u.to_string());
                out["scheme"] = json!(u.scheme);
                out["hostname"] = json!(String::from_utf8_lossy(u.hostname()));
                u.scheme = "https".into();
                out["https"] = json!(u.to_string());
            }
            out
        }
        "host" => json!({"error": validate_host(&i.raw).err().map(|e| e.to_string())}),
        "change-summary" => json!(change_summary(&i.old, &i.settings)),
        "personal-summary" => json!(personal_summary(&i.before, &i.profile)),
        "legacy-defaults" => json!(same_legacy_defaults(&i.old, &i.settings)),
        "encode-settings" => {
            json!({"yaml": String::from_utf8(git::encode_settings(&c, &i.settings).unwrap()).unwrap(), "error": null})
        }
        "encode-favorites" => {
            let yaml = git::encode_favorites(&c, &i.identity, &i.profile).unwrap();
            json!({"yaml": String::from_utf8(yaml).unwrap(), "error": null})
        }
        "favorite-path" => json!(git::favorite_path(&i.git_path, &i.identity.subject)),
        "read-settings" => error_or(
            git::parse_settings_manifest(&c, i.raw.as_bytes()),
            |s| json!({"settings": s, "version": version(&s)}),
        ),
        "read-favorites" => error_or(
            git::parse_favorites_manifest(&c, &i.identity, i.raw.as_bytes()),
            |p| json!({"profile": p, "exists": true}),
        ),
        "store-validation" => {
            let mut cfg = c.clone();
            cfg.git_url = i.git_url;
            cfg.git_branch = i.branch;
            cfg.git_path = i.git_path;
            json!(
                git::validate_store_config(&cfg)
                    .err()
                    .map(|e| e.to_string())
                    .unwrap_or_default()
            )
        }
        "config" => {
            let env = i.env;
            let lookup = |k: &str, d: &str| {
                env.get(k)
                    .filter(|v| !v.is_empty())
                    .cloned()
                    .unwrap_or_else(|| d.into())
            };
            error_or(Config::from_lookup(lookup), |cfg| config_json(&cfg))
        }
        other => panic!("unknown op {other}"),
    };
    println!("{value}");
}
