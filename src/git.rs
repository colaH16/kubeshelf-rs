//! Git persistence (Git parts of Go `store.go` and `favorite_store.go`).
//!
//! KubeShelf owns a private, ephemeral clone under `$KUBESHELF_WORK_DIR/repository`
//! and talks to the remote only through the `git` CLI over SSH:
//!
//! * hooks are disabled, the `file://` protocol is refused, and only the
//!   configured SSH key (copied to a private 0600 file) and known_hosts file
//!   are used (`StrictHostKeyChecking=yes`);
//! * reads use `git show <commit>:<path>`, which reads blobs from the object
//!   database and never follows a symlink in a working tree;
//! * writes stage blobs with `hash-object` + `update-index --cacheinfo`
//!   (again no working-tree files), commit, and push without force. A
//!   rejected push means another writer won; the caller reports a conflict.
//!
//! Every path written is server-derived: the configured settings path or the
//! per-user ConfigMap path computed from the OIDC subject hash. No path from
//! an HTTP request ever reaches the Git index.

use crate::{config::Config, model::*};
use anyhow::{Context, Result, anyhow, ensure};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    os::unix::fs::PermissionsExt,
    path::{Component, Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{io::AsyncWriteExt, process::Command};

#[derive(Clone)]
pub struct Repository {
    pub cfg: Config,
    dir: PathBuf,
    /// Tests use local bare repositories instead of SSH.
    local: bool,
}

/// POSIX shell single-quoting for `core.sshCommand`.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\"'\"'"))
}

/// Go `favoriteConfigMapName`: subject → stable, DNS-safe ConfigMap name.
pub fn favorite_name(subject: &str) -> String {
    format!(
        "kubeshelf-user-{}",
        hex::encode(&Sha256::digest(subject.as_bytes())[..20])
    )
}

/// Go `favoritePath`: `<dir of settings path>/users/<name>.yaml`
/// (Go `path.Join(path.Dir(p), …)`, so a bare file name gives `users/…`).
pub fn favorite_path(git_path: &str, subject: &str) -> String {
    let dir = match git_path.rfind('/') {
        Some(i) => &git_path[..i],
        None => "",
    };
    let file = format!("users/{}.yaml", favorite_name(subject));
    if dir.is_empty() {
        file
    } else {
        format!("{dir}/{file}")
    }
}

/// Startup validation of the Git settings (Go `OpenStore`).
///
/// The manifest path check is stricter than Go's `filepath.Clean` based
/// check: it must already be a normalised relative path with no `.`/`..`/
/// empty components and no `.git` component. Go accepted e.g. `./a.yaml`,
/// but Git's index rejects such paths, so saves could never have worked.
pub fn validate_store_config(cfg: &Config) -> Result<()> {
    let ssh = regex::Regex::new(r"^(ssh://[^\s]+|[A-Za-z0-9._-]+@[A-Za-z0-9._-]+:[^\s]+)$")?;
    ensure!(ssh.is_match(&cfg.git_url), "Git URL must use SSH");
    let b = &cfg.git_branch;
    ensure!(
        !b.contains([' ', '\n', '\r', '~', '^', ':', '?', '*', '[', '\\'])
            && !b.starts_with('-')
            && !b.contains(".."),
        "invalid Git branch"
    );
    let p = &cfg.git_path;
    let normal = !p.is_empty()
        && !p.contains(|c: char| c.is_control())
        && p.split('/').all(|part| {
            !part.is_empty() && part != "." && part != ".." && !part.eq_ignore_ascii_case(".git")
        })
        && Path::new(p)
            .components()
            .all(|c| matches!(c, Component::Normal(_)));
    ensure!(normal, "invalid Git manifest path");
    Ok(())
}

impl Repository {
    pub fn new(cfg: Config) -> Result<Self> {
        validate_store_config(&cfg)?;
        Ok(Self {
            dir: Path::new(&cfg.work_dir).join("repository"),
            cfg,
            local: false,
        })
    }

    /// A repository that may use local `file://` remotes (tests only).
    pub fn for_local_tests(cfg: Config) -> Self {
        Self {
            dir: Path::new(&cfg.work_dir).join("repository"),
            cfg,
            local: true,
        }
    }

    /// Run one `git` command in the private clone (Go `gitRepository.run`).
    pub async fn run(&self, args: &[&str], input: Option<&[u8]>) -> Result<Vec<u8>> {
        let mut cmd = Command::new("git");
        cmd.current_dir(&self.dir)
            .kill_on_drop(true)
            .args(["-c", "core.hooksPath=/dev/null"])
            .args([
                "-c",
                if self.local {
                    "protocol.file.allow=always"
                } else {
                    "protocol.file.allow=never"
                },
            ])
            .args(["-c", "commit.gpgsign=false"])
            .args(["-c", "user.name=KubeShelf"])
            .args(["-c", "user.email=kubeshelf@users.noreply.local"])
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null");
        if !self.local {
            // Secret volumes can be group-readable; OpenSSH requires a private
            // copy with mode 0600.
            let key = tokio::fs::read(&self.cfg.ssh_key_file)
                .await
                .map_err(|_| anyhow!("Git SSH 키를 읽을 수 없습니다"))?;
            let private = Path::new(&self.cfg.work_dir).join("git-identity");
            let mut file = tokio::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&private)
                .await?;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))
                .await?;
            file.write_all(&key).await?;
            file.flush().await?;
            drop(file);
            let ssh = format!(
                "ssh -F /dev/null -o BatchMode=yes -o IdentitiesOnly=yes -o StrictHostKeyChecking=yes -o ConnectTimeout=10 -o UserKnownHostsFile={} -i {}",
                shell_quote(&self.cfg.known_hosts_file),
                shell_quote(&private.to_string_lossy())
            );
            cmd.args(["-c", &format!("core.sshCommand={ssh}")]);
        }
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let operation = async {
            let mut child = cmd.spawn()?;
            let mut stdin = child.stdin.take().context("git stdin")?;
            if let Some(data) = input {
                stdin.write_all(data).await?;
            }
            drop(stdin);
            let out = child.wait_with_output().await?;
            if !out.status.success() {
                // stderr may contain remote URLs or host names, so it is only
                // logged at debug level and never returned to HTTP clients.
                tracing::debug!(
                    "git {} failed: {}",
                    args[0],
                    String::from_utf8_lossy(&out.stderr).trim()
                );
            }
            ensure!(out.status.success(), "Git {} 작업에 실패했습니다", args[0]);
            Ok(out.stdout)
        };
        tokio::time::timeout(Duration::from_secs(20), operation)
            .await
            .map_err(|_| anyhow!("Git {} 작업 시간이 초과됐습니다", args[0]))?
    }

    /// Create the private clone on first use and (re)point `origin`.
    pub async fn init(&self) -> Result<()> {
        tokio::fs::create_dir_all(&self.dir).await?;
        tokio::fs::set_permissions(&self.dir, std::fs::Permissions::from_mode(0o700)).await?;
        if !self.dir.join(".git").exists() {
            let branch = format!("--initial-branch={}", self.cfg.git_branch);
            self.run(&["init", &branch], None).await?;
            self.run(&["remote", "add", "origin", &self.cfg.git_url], None)
                .await?;
        }
        self.run(&["remote", "set-url", "origin", &self.cfg.git_url], None)
            .await?;
        Ok(())
    }

    /// Fetch the branch and return the shared settings plus the commit id.
    pub async fn read(&self) -> Result<(Settings, String)> {
        self.run(
            &["fetch", "--depth=20", "origin", &self.cfg.git_branch],
            None,
        )
        .await?;
        // Only this application's private clone is reset, never an operator checkout.
        self.run(&["reset", "--hard", "FETCH_HEAD"], None).await?;
        let head = self.rev_parse_head().await?;
        let bytes = self
            .run(&["show", &format!("HEAD:{}", self.cfg.git_path)], None)
            .await?;
        Ok((parse_settings_manifest(&self.cfg, &bytes)?, head))
    }

    async fn rev_parse_head(&self) -> Result<String> {
        let out = self.run(&["rev-parse", "HEAD"], None).await?;
        Ok(String::from_utf8(out)?.trim().to_owned())
    }

    pub fn favorite_path(&self, user: &Identity) -> String {
        favorite_path(&self.cfg.git_path, &user.subject)
    }

    /// Read one account's favourites at `commit`. Returns the defaults and
    /// `false` when the account has no file yet.
    pub async fn read_favorites(
        &self,
        user: &Identity,
        commit: &str,
    ) -> Result<(FavoriteProfile, bool)> {
        ensure!(
            !commit.is_empty() && commit.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid Git commit"
        );
        let path = self.favorite_path(user);
        let entry = self
            .run(&["ls-tree", "--name-only", commit, "--", &path], None)
            .await?;
        if entry.iter().all(u8::is_ascii_whitespace) {
            return Ok((FavoriteProfile::for_user(user.admin), false));
        }
        let bytes = self
            .run(&["show", &format!("{commit}:{path}")], None)
            .await?;
        Ok((parse_favorites_manifest(&self.cfg, user, &bytes)?, true))
    }

    pub fn encode_settings(&self, settings: &Settings) -> Result<Vec<u8>> {
        encode_settings(&self.cfg, settings)
    }

    pub fn encode_favorites(&self, user: &Identity, profile: &FavoriteProfile) -> Result<Vec<u8>> {
        encode_favorites(&self.cfg, user, profile)
    }

    /// Stage `files`, commit and push (Go `writeFiles`). Returns the new commit.
    pub async fn write(&self, files: &BTreeMap<String, Vec<u8>>, message: &str) -> Result<String> {
        ensure!(!self.cfg.read_only, "read-only validation mode");
        for (path, data) in files {
            let blob = String::from_utf8(
                self.run(&["hash-object", "-w", "--stdin"], Some(data))
                    .await?,
            )?;
            let info = format!("100644,{},{}", blob.trim(), path);
            self.run(&["update-index", "--add", "--cacheinfo", &info], None)
                .await?;
        }
        self.run(&["commit", "--file=-"], Some(message.as_bytes()))
            .await?;
        let head = self.rev_parse_head().await?;
        let target = format!("HEAD:refs/heads/{}", self.cfg.git_branch);
        if self.run(&["push", "origin", &target], None).await.is_err() {
            // A lost acknowledgement can follow a successful push; confirm
            // the remote branch before reporting failure.
            let branch = format!("refs/heads/{}", self.cfg.git_branch);
            let remote = self.run(&["ls-remote", "origin", &branch], None).await;
            let pushed =
                remote.is_ok_and(|r| String::from_utf8_lossy(&r).starts_with(&format!("{head}\t")));
            ensure!(
                pushed,
                "Git push에 실패했습니다. 원격 변경 또는 연결 상태를 확인해 주세요"
            );
        }
        Ok(head)
    }
}

// ---------------------------------------------------------------------------
// ConfigMap manifests
// ---------------------------------------------------------------------------

/// Go `yaml.UnmarshalStrict(data, &core.ConfigMap{})` equivalent.
///
/// * duplicate keys are rejected (via `serde_yaml::Value`);
/// * unknown fields are rejected at *every* level: the document is decoded
///   into the typed `k8s_openapi` ConfigMap, re-encoded, and every non-null
///   input key must survive that round trip;
/// * wrong value types (for example a number in `data`) fail the typed decode.
fn decode_configmap(data: &[u8], message: &'static str) -> Result<Value> {
    let invalid = || anyhow!(message);
    let yaml: serde_yaml::Value = serde_yaml::from_slice(data).map_err(|_| invalid())?;
    let input: Value = serde_json::to_value(yaml).map_err(|_| invalid())?;
    ensure!(input.is_object(), invalid());
    // TypeMeta is checked by the callers (Go reports a wrong kind as a
    // target mismatch), so only its type is validated here.
    let mut body = input.clone();
    for field in ["apiVersion", "kind"] {
        let removed = body.as_object_mut().and_then(|m| m.remove(field));
        ensure!(
            removed.is_none_or(|v| v.is_string() || v.is_null()),
            invalid()
        );
    }
    let typed: k8s_openapi::api::core::v1::ConfigMap =
        serde_json::from_value(body.clone()).map_err(|_| invalid())?;
    let known = serde_json::to_value(typed)?;
    ensure!(only_known_keys(&body, &known), invalid());
    Ok(input)
}

/// True when every non-null key in `input` also exists in `known`
/// (recursively through objects and arrays).
fn only_known_keys(input: &Value, known: &Value) -> bool {
    match (input, known) {
        (Value::Object(i), Value::Object(k)) => i
            .iter()
            .all(|(key, v)| v.is_null() || k.get(key).is_some_and(|kv| only_known_keys(v, kv))),
        (Value::Object(i), _) => i.values().all(Value::is_null),
        (Value::Array(i), Value::Array(k)) => i.iter().zip(k).all(|(a, b)| only_known_keys(a, b)),
        _ => true,
    }
}

/// Parse and check the shared settings ConfigMap (Go `gitRepository.read`).
pub fn parse_settings_manifest(cfg: &Config, bytes: &[u8]) -> Result<Settings> {
    let cm = decode_configmap(bytes, "Git ConfigMap 형식 오류")?;
    ensure!(
        cm["kind"] == "ConfigMap"
            && cm["apiVersion"] == "v1"
            && cm["metadata"]["name"] == cfg.configmap_name.as_str()
            && cm["metadata"]["namespace"] == cfg.namespace.as_str(),
        "Git ConfigMap 대상이 일치하지 않습니다"
    );
    Settings::decode(
        cm["data"]["settings.json"]
            .as_str()
            .unwrap_or_default()
            .as_bytes(),
    )
}

/// Parse and check one personal ConfigMap (Go `readFavorites`). The file must
/// name this subject, both by name hash and by annotation.
pub fn parse_favorites_manifest(
    cfg: &Config,
    user: &Identity,
    bytes: &[u8],
) -> Result<FavoriteProfile> {
    let cm = decode_configmap(bytes, "사용자 ConfigMap 형식이 올바르지 않습니다")?;
    let count = |v: &Value| v.as_object().map_or(0, |m| m.len());
    ensure!(
        cm["kind"] == "ConfigMap"
            && cm["apiVersion"] == "v1"
            && cm["metadata"]["name"] == favorite_name(&user.subject).as_str()
            && cm["metadata"]["namespace"] == cfg.namespace.as_str()
            && cm["metadata"]["annotations"]["kubeshelf.io/subject"] == user.subject.as_str()
            && count(&cm["data"]) == 1
            && count(&cm["binaryData"]) == 0,
        "사용자 ConfigMap 대상이 일치하지 않습니다"
    );
    let raw = cm["data"]["favorites.json"].as_str().unwrap_or_default();
    let mut profile: FavoriteProfile = decode_json(raw.as_bytes(), &PROFILE_SHAPE)
        .map_err(|_| anyhow!("즐겨찾기 파일 형식이 올바르지 않습니다"))?;
    profile.validate()?;
    Ok(profile)
}

/// Go `encodeSettings`.
pub fn encode_settings(cfg: &Config, settings: &Settings) -> Result<Vec<u8>> {
    let json = go_json_indent(settings)? + "\n";
    Ok(format!(
        "apiVersion: v1\ndata:\n  settings.json: {}kind: ConfigMap\nmetadata:\n  name: {}\n  namespace: {}\n",
        yaml_block(&json, 4),
        yaml_scalar(&cfg.configmap_name),
        yaml_scalar(&cfg.namespace),
    )
    .into_bytes())
}

/// Go `encodeFavorites`.
pub fn encode_favorites(
    cfg: &Config,
    user: &Identity,
    profile: &FavoriteProfile,
) -> Result<Vec<u8>> {
    let json = go_json_indent(profile)? + "\n";
    Ok(format!(
        concat!(
            "apiVersion: v1\ndata:\n  favorites.json: {}kind: ConfigMap\nmetadata:\n",
            "  annotations:\n    kubeshelf.io/subject: {}\n",
            "  labels:\n    app.kubernetes.io/part-of: kubeshelf\n    kubeshelf.io/user-settings: \"true\"\n",
            "  name: {}\n  namespace: {}\n"
        ),
        yaml_block(&json, 4),
        yaml_scalar(&user.subject),
        yaml_scalar(&favorite_name(&user.subject)),
        yaml_scalar(&cfg.namespace),
    )
    .into_bytes())
}

// The Go side writes YAML with sigs.k8s.io/yaml (go-yaml v2, YAML 1.1). That
// output is reproduced byte-for-byte for ordinary values. For unusual values
// this emitter double-quotes instead of guessing go-yaml's exact style; the
// result always parses to the same strings under YAML 1.1 and 1.2. (serde_yaml
// would emit e.g. `on` unquoted, which a YAML 1.1 reader treats as a boolean.)

/// Words that YAML 1.1 (go-yaml v2) resolves to booleans or null.
const YAML11_WORDS: &[&str] = &[
    "y", "Y", "yes", "Yes", "YES", "n", "N", "no", "No", "NO", "true", "True", "TRUE", "false",
    "False", "FALSE", "on", "On", "ON", "off", "Off", "OFF", "null", "Null", "NULL", "~",
];

/// go-yaml v2 `resolve`: does a digit/sign-first plain scalar parse as an
/// int (`strconv.ParseInt(s, 0, 64)` after removing `_`), a YAML float, or a
/// timestamp (four digits then `-`, checked conservatively)?
fn yaml11_number_or_time(s: &str) -> bool {
    let plain = s.replace('_', "");
    // One optional sign, as in strconv.ParseInt and the float pattern.
    let unsigned = plain.strip_prefix(['+', '-']).unwrap_or(&plain);
    let all = |t: &str, radix: u32| !t.is_empty() && t.chars().all(|c| c.is_digit(radix));
    let int = match unsigned.get(..2) {
        Some("0x" | "0X") => all(&unsigned[2..], 16),
        Some("0o" | "0O") => all(&unsigned[2..], 8),
        Some("0b" | "0B") => all(&unsigned[2..], 2),
        _ => all(unsigned, 10),
    };
    // yamlStyleFloat = ^[-+]?(\.[0-9]+|[0-9]+(\.[0-9]*)?)([eE][-+]?[0-9]+)?$
    let float = {
        let (mantissa, exponent) = match unsigned.find(['e', 'E']) {
            Some(i) => (&unsigned[..i], Some(&unsigned[i + 1..])),
            None => (unsigned, None),
        };
        let mantissa_ok = match mantissa.split_once('.') {
            Some(("", frac)) => all(frac, 10),
            Some((int, frac)) => all(int, 10) && (frac.is_empty() || all(frac, 10)),
            None => all(mantissa, 10),
        };
        let exponent_ok = exponent.is_none_or(|e| all(e.strip_prefix(['+', '-']).unwrap_or(e), 10));
        mantissa_ok && exponent_ok
    };
    let b = s.as_bytes();
    let timestamp = b.len() > 4 && b[..4].iter().all(u8::is_ascii_digit) && b[4] == b'-';
    int || float || timestamp
}

/// Whether go-yaml v2 emits `s` as a plain scalar. Conservative: only simple
/// tokens qualify; anything else is double-quoted.
fn yaml_plain(s: &str) -> bool {
    let Some(first) = s.chars().next() else {
        return false;
    };
    s.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
        && first.is_ascii_alphanumeric()
        && !YAML11_WORDS.contains(&s)
        && !(first.is_ascii_digit() && yaml11_number_or_time(s))
}

/// YAML-printable per go-yaml (`is_printable`), excluding line breaks.
fn yaml_printable(c: char) -> bool {
    matches!(c as u32, 0x20..=0x7E | 0xA0..=0xD7FF | 0xE000..=0xFFFD | 0x10000..=0x10FFFF)
        && !matches!(c, '\u{FEFF}' | '\u{2028}' | '\u{2029}')
}

/// A YAML double-quoted scalar.
fn yaml_quoted(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if yaml_printable(c) => out.push(c),
            c if (c as u32) <= 0xFFFF => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push_str(&format!("\\U{:08X}", c as u32)),
        }
    }
    out.push('"');
    out
}

/// A single-line scalar value.
fn yaml_scalar(s: &str) -> String {
    if yaml_plain(s) {
        s.to_owned()
    } else {
        yaml_quoted(s)
    }
}

/// A multi-line value ending in exactly one newline, as a literal block
/// (`|`) indented by `indent` spaces; falls back to a quoted scalar if the
/// text cannot be represented literally.
fn yaml_block(text: &str, indent: usize) -> String {
    let body = text.strip_suffix('\n').unwrap_or(text);
    let literal = text.ends_with('\n')
        && !body.ends_with('\n')
        && !body.starts_with(' ')
        && body.split('\n').all(|line| {
            !line.is_empty() && !line.ends_with(' ') && line.chars().all(yaml_printable)
        });
    if !literal {
        return yaml_quoted(text) + "\n";
    }
    let pad = " ".repeat(indent);
    let mut out = String::from("|\n");
    for line in body.split('\n') {
        out.push_str(&pad);
        out.push_str(line);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Config {
        Config::from_lookup(|k, d| {
            if k == "KUBESHELF_DEMO" {
                "true".into()
            } else {
                d.into()
            }
        })
        .unwrap()
    }

    #[test]
    fn strict_configmap_rejects_nested_unknown_and_duplicates() {
        let c = cfg();
        let good = encode_settings(&c, &Settings::empty()).unwrap();
        assert_eq!(
            parse_settings_manifest(&c, &good).unwrap(),
            Settings::empty()
        );
        let text = String::from_utf8(good).unwrap();
        let nested = text.replace("metadata:\n", "metadata:\n  bogus: 1\n");
        assert!(
            parse_settings_manifest(&c, nested.as_bytes()).is_err(),
            "nested unknown field"
        );
        let duplicate = text.replace(
            "  namespace: public-services\n",
            "  namespace: public-services\n  namespace: other\n",
        );
        assert!(
            parse_settings_manifest(&c, duplicate.as_bytes()).is_err(),
            "duplicate key"
        );
        let wrong_type = text.replace("kind: ConfigMap\n", "kind: ConfigMap\nimmutable: \"yes\"\n");
        assert!(
            parse_settings_manifest(&c, wrong_type.as_bytes()).is_err(),
            "wrong type"
        );
        let tolerated = text.replace(
            "metadata:\n",
            "metadata:\n  creationTimestamp: null\n  labels: {a: b}\n",
        );
        assert!(
            parse_settings_manifest(&c, tolerated.as_bytes()).is_ok(),
            "known ObjectMeta fields"
        );
    }

    #[test]
    fn yaml_scalars_stay_strings_under_yaml_1_1() {
        for (value, rendered) in [
            (
                "2f1c9a0e-5b7d-4c11-9a8e-0123456789ab",
                "2f1c9a0e-5b7d-4c11-9a8e-0123456789ab",
            ),
            ("on", "\"on\""),
            ("12345", "\"12345\""),
            ("1e10", "\"1e10\""),
            ("2024-01-02", "\"2024-01-02\""),
            ("a b", "\"a b\""),
            ("", "\"\""),
        ] {
            assert_eq!(yaml_scalar(value), rendered, "{value}");
        }
    }

    #[test]
    fn favorite_paths_follow_go_path_join() {
        assert_eq!(
            favorite_path("settings.yaml", "x"),
            format!("users/{}.yaml", favorite_name("x"))
        );
        assert_eq!(
            favorite_path("a/b/s.yaml", "x"),
            format!("a/b/users/{}.yaml", favorite_name("x"))
        );
    }

    #[test]
    fn manifest_path_validation() {
        let mut c = cfg();
        c.git_url = "git@example.invalid:o/r.git".into();
        for ok in ["runtime/settings.yaml", "settings.yaml", "a/b/c.yaml"] {
            c.git_path = ok.into();
            assert!(validate_store_config(&c).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "/abs.yaml",
            "../x.yaml",
            "a/../../x",
            ".git/config",
            "a/.GIT/x",
            "./x.yaml",
            "a//b.yaml",
            "a/",
        ] {
            c.git_path = bad.into();
            assert!(validate_store_config(&c).is_err(), "{bad:?}");
        }
    }
}
