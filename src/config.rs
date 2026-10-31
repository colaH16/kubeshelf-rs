//! Environment configuration (Go `ConfigFromEnv` plus `cmd/kubeshelf/main.go`).
//!
//! All runtime configuration comes from `KUBESHELF_*` environment variables
//! (and `POD_NAMESPACE`); the binary accepts no command-line arguments.
//! Variable names, defaults and validation order match the Go original.
//! Two variables are Rust-only additions: `KUBESHELF_STATIC_DIR` (the Go
//! binary embedded the frontend) and `KUBESHELF_READ_ONLY`.

use crate::model::validate_url;
use anyhow::{Result, anyhow, bail, ensure};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct Config {
    pub listen: String,
    pub public_url: String,
    pub cluster: String,
    pub kubeconfig: String,
    pub settings_file: String,
    pub work_dir: String,
    pub git_url: String,
    pub git_branch: String,
    pub git_path: String,
    pub ssh_key_file: String,
    pub known_hosts_file: String,
    pub oidc_issuer: String,
    pub oidc_client_id: String,
    pub oidc_secret_file: String,
    pub directory_url: String,
    pub directory_token_file: String,
    pub admin_subjects: Vec<String>,
    pub ingress_schemes: BTreeMap<String, String>,
    pub demo: bool,
    /// Rust-only: serve the UI and API but refuse every write (validation runs).
    pub read_only: bool,
    /// Go reads these two with `env(...)` at the point of use.
    pub configmap_name: String,
    pub namespace: String,
    /// Rust-only: directory containing the built frontend (`web/dist`).
    pub static_dir: String,
}

/// Go `env(k, d)`: the variable if set and non-empty, else the default.
pub fn env(k: &str, d: &str) -> String {
    std::env::var(k)
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| d.into())
}

impl Config {
    pub fn from_env() -> Result<Self> {
        Self::from_lookup(env)
    }

    /// Build a config from any `env`-like lookup (tests pass a closure).
    pub fn from_lookup(get: impl Fn(&str, &str) -> String) -> Result<Self> {
        let demo = get("KUBESHELF_DEMO", "") == "true";
        let mut cfg = Self {
            listen: get("KUBESHELF_LISTEN", ":8080"),
            public_url: get("KUBESHELF_PUBLIC_URL", ""),
            cluster: get("KUBESHELF_CLUSTER_NAME", "cluster"),
            kubeconfig: get("KUBESHELF_KUBECONFIG", ""),
            settings_file: get("KUBESHELF_SETTINGS_FILE", "/config/runtime/settings.json"),
            work_dir: get("KUBESHELF_WORK_DIR", "/work"),
            git_url: get("KUBESHELF_GIT_URL", ""),
            git_branch: get("KUBESHELF_GIT_BRANCH", "main"),
            git_path: get("KUBESHELF_GIT_PATH", "runtime/settings.yaml"),
            ssh_key_file: get("KUBESHELF_SSH_KEY_FILE", "/credentials/git/ssh-privatekey"),
            known_hosts_file: get(
                "KUBESHELF_KNOWN_HOSTS_FILE",
                "/credentials/git-host/known_hosts",
            ),
            oidc_issuer: get("KUBESHELF_OIDC_ISSUER", ""),
            oidc_client_id: get("KUBESHELF_OIDC_CLIENT_ID", ""),
            oidc_secret_file: get(
                "KUBESHELF_OIDC_SECRET_FILE",
                "/credentials/auth/client-secret",
            ),
            directory_url: get("KUBESHELF_DIRECTORY_URL", ""),
            directory_token_file: get(
                "KUBESHELF_DIRECTORY_TOKEN_FILE",
                "/credentials/auth/directory-token",
            ),
            admin_subjects: get("KUBESHELF_ADMIN_SUBJECTS", "")
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect(),
            ingress_schemes: BTreeMap::new(),
            demo,
            read_only: get("KUBESHELF_READ_ONLY", "false") == "true",
            configmap_name: get("KUBESHELF_CONFIGMAP_NAME", "kubeshelf-settings"),
            namespace: get("POD_NAMESPACE", "public-services"),
            static_dir: get("KUBESHELF_STATIC_DIR", "web/dist"),
        };
        if cfg.public_url.is_empty() && demo {
            cfg.public_url = "http://localhost:8080".into();
        }
        let u = validate_url(&cfg.public_url).map_err(|_| {
            anyhow!("KUBESHELF_PUBLIC_URL is required and must be an HTTP(S) origin")
        })?;
        let path = String::from_utf8_lossy(&u.path);
        if (!path.is_empty() && path != "/") || !u.raw_query.is_empty() || !u.fragment.is_empty() {
            bail!("public URL must be an origin");
        }
        cfg.public_url = cfg.public_url.trim_end_matches('/').into();
        ensure!(
            demo || (u.scheme == "https"
                && !cfg.oidc_issuer.is_empty()
                && !cfg.oidc_client_id.is_empty()
                && !cfg.admin_subjects.is_empty()
                && !cfg.git_url.is_empty()),
            "production requires HTTPS, OIDC, admin subjects, and Git URL"
        );
        let schemes = get("KUBESHELF_INGRESS_SCHEMES", "");
        if !schemes.is_empty() {
            // Go: json.Unmarshal into the map; `null` leaves it empty and a
            // `null` value becomes "" (rejected just below).
            let parsed: Option<BTreeMap<String, Option<String>>> = serde_json::from_str(&schemes)?;
            for (class, scheme) in parsed.unwrap_or_default() {
                cfg.ingress_schemes
                    .insert(class, scheme.unwrap_or_default());
            }
        }
        ensure!(
            cfg.ingress_schemes
                .values()
                .all(|v| v == "https" || v == "http"),
            "invalid ingress scheme"
        );
        if demo {
            // cmd/kubeshelf/main.go: the demo Ingresses use this class.
            cfg.ingress_schemes
                .insert("demo-https".into(), "https".into());
        }
        Ok(cfg)
    }
}
