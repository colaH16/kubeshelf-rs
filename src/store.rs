//! Settings and favourites store (Go `Store` in `store.go`/`favorite_store.go`).
//!
//! Two copies of the shared settings exist on purpose:
//!
//! * **applied** — read every second from the ConfigMap that Fleet projects
//!   into the pod (`KUBESHELF_SETTINGS_FILE`). Only this copy is used for
//!   authorisation and the public catalog.
//! * **desired** — the latest commit in Git, edited by admins. A successful
//!   save pushes to Git and marks the status as *pending*; it never changes
//!   `applied`. Production settings therefore only take effect after the
//!   GitOps pipeline has applied them. (Demo mode simulates that delay.)
//!
//! Compare-and-swap tokens are scoped to one file: the shared settings use
//! the SHA-256 of the settings JSON, each personal file its own hash, so a
//! personal commit never conflicts with an admin edit and vice versa.

use crate::{config::Config, git::Repository, model::*};
use anyhow::{Result, anyhow, ensure};
use rand::RngCore;
use serde::Serialize;
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio::sync::{Mutex, RwLock};

/// Go `randomID`: 24 random bytes, hex encoded.
pub fn random_id() -> String {
    let mut b = [0u8; 24];
    rand::rng().fill_bytes(&mut b);
    hex::encode(b)
}

/// Replace control characters (commit messages must stay single-purpose).
fn sanitize(v: &str) -> String {
    v.chars()
        .map(|c| if c < ' ' || c == '\u{7f}' { ' ' } else { c })
        .collect()
}

/// Go `ApplyStatus`.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyStatus {
    pub desired_revision: String,
    pub applied_revision: String,
    pub commit: String,
    pub pending: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub error: String,
    pub applied_at: String,
}

struct Inner {
    applied: Settings,
    desired: Settings,
    commit: String,
    /// Raw bytes of the last successfully applied file (skip re-parsing).
    last_file: Vec<u8>,
    apply_error: String,
    applied_at: String,
    /// Demo mode keeps favourites in memory.
    profiles: BTreeMap<String, FavoriteProfile>,
}

pub struct Store {
    pub cfg: Config,
    repo: Option<Repository>,
    inner: RwLock<Inner>,
    /// Serialises every Git read-modify-write (Go `saveMu`).
    save: Mutex<()>,
}

impl Store {
    /// Go `OpenStore`. Production reads the mounted settings first, then
    /// validates the Git configuration and fetches the desired settings.
    pub async fn open(cfg: Config, demo: Settings) -> Result<Arc<Self>> {
        let (applied, last_file, repo) = if cfg.demo {
            (demo, vec![], None)
        } else {
            let bytes = tokio::fs::read(&cfg.settings_file)
                .await
                .map_err(|_| anyhow!("mounted runtime settings unavailable"))?;
            let settings = Settings::decode(&bytes)?;
            let repo = Repository::new(cfg.clone())?;
            repo.init().await?;
            (settings, bytes, Some(repo))
        };
        let store = Arc::new(Self::with_repository(cfg, repo, applied, last_file));
        store.refresh().await?;
        Ok(store)
    }

    fn with_repository(
        cfg: Config,
        repo: Option<Repository>,
        applied: Settings,
        last_file: Vec<u8>,
    ) -> Self {
        let commit = if repo.is_none() { "demo-initial" } else { "" };
        Self {
            cfg,
            repo,
            inner: RwLock::new(Inner {
                desired: applied.clone(),
                applied,
                commit: commit.into(),
                last_file,
                apply_error: String::new(),
                applied_at: now(),
                profiles: BTreeMap::new(),
            }),
            save: Mutex::new(()),
        }
    }

    /// Settings in force (from the projected ConfigMap).
    pub async fn applied(&self) -> Settings {
        self.inner.read().await.applied.clone()
    }

    /// Latest Git settings and their CAS version.
    pub async fn desired(&self) -> (Settings, String) {
        let settings = self.inner.read().await.desired.clone();
        let v = version(&settings);
        (settings, v)
    }

    pub async fn status(&self) -> ApplyStatus {
        let s = self.inner.read().await;
        ApplyStatus {
            desired_revision: s.desired.revision.clone(),
            applied_revision: s.applied.revision.clone(),
            commit: s.commit.clone(),
            pending: s.desired.revision != s.applied.revision,
            error: s.apply_error.clone(),
            applied_at: s.applied_at.clone(),
        }
    }

    async fn refresh_locked(&self) -> Result<()> {
        if let Some(repo) = &self.repo {
            let (next, commit) = repo.read().await?;
            let mut s = self.inner.write().await;
            s.desired = next;
            s.commit = commit;
        }
        Ok(())
    }

    /// Re-read the desired settings from Git (Go `Store.Refresh`).
    pub async fn refresh(&self) -> Result<()> {
        let _guard = self.save.lock().await;
        self.refresh_locked().await
    }

    /// Go `loadApplied`: returns whether the applied revision changed.
    async fn load_applied(&self) -> Result<bool> {
        let bytes = tokio::fs::read(&self.cfg.settings_file)
            .await
            .map_err(|_| anyhow!("mounted runtime settings unavailable"))?;
        if bytes == self.inner.read().await.last_file {
            self.inner.write().await.apply_error.clear();
            return Ok(false);
        }
        let next = Settings::decode(&bytes)?;
        let mut s = self.inner.write().await;
        let changed = next.revision != s.applied.revision;
        s.applied = next;
        s.last_file = bytes;
        s.applied_at = now();
        s.apply_error.clear();
        Ok(changed)
    }

    /// Go `Store.Run`: poll the projected file every second. An invalid file
    /// keeps the last good settings and reports the error in the status.
    pub async fn run(self: Arc<Self>) {
        if self.cfg.demo {
            return;
        }
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            match self.load_applied().await {
                Ok(true) => {
                    // A projected ConfigMap change (not a Git poll) triggers refresh.
                    match tokio::time::timeout(Duration::from_secs(20), self.refresh()).await {
                        Ok(Ok(())) => {}
                        Ok(Err(e)) => tracing::warn!("settings refresh after apply failed: {e:#}"),
                        Err(_) => tracing::warn!("settings refresh after apply timed out"),
                    }
                }
                Ok(false) => {}
                Err(e) => self.inner.write().await.apply_error = e.to_string(),
            }
        }
    }

    /// Go `Store.Save`: CAS on the shared settings version, then commit+push.
    /// Production never changes the applied settings here; Fleet does.
    pub async fn save(
        self: &Arc<Self>,
        base: &str,
        mut next: Settings,
        actor: &str,
    ) -> Result<ApplyStatus> {
        ensure!(!self.cfg.read_only, "read-only validation mode");
        next.validate()?;
        let _guard = self.save.lock().await;
        self.refresh_locked().await?;
        let (old, current) = self.desired().await;
        if base != current {
            return Err(Conflict(CONFLICT).into());
        }
        next.revision = random_id();
        let commit = match &self.repo {
            Some(repo) => {
                let files =
                    BTreeMap::from([(self.cfg.git_path.clone(), repo.encode_settings(&next)?)]);
                let message = format!(
                    "{}\n\nEdited by: {}",
                    change_summary(&old, &next),
                    sanitize(actor)
                );
                repo.write(&files, &message).await?
            }
            None => format!("demo-{}", next.revision),
        };
        {
            let mut s = self.inner.write().await;
            s.desired = next.clone();
            s.commit = commit;
        }
        if self.cfg.demo {
            // Demo: pretend the GitOps pipeline applied the change.
            let store = self.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(1500)).await;
                let mut s = store.inner.write().await;
                if s.desired.revision == next.revision {
                    s.applied = next;
                    s.applied_at = now();
                }
            });
        }
        Ok(self.status().await)
    }

    async fn profile_locked(&self, user: &Identity) -> Result<FavoriteProfile> {
        match &self.repo {
            Some(repo) => {
                let commit = self.inner.read().await.commit.clone();
                Ok(repo.read_favorites(user, &commit).await?.0)
            }
            None => Ok(self
                .inner
                .read()
                .await
                .profiles
                .get(&user.subject)
                .cloned()
                .unwrap_or_else(|| FavoriteProfile::for_user(user.admin))),
        }
    }

    /// Go `Store.Favorites`. Successful pushes are durable immediately, so
    /// favourites are read from the confirmed Git commit.
    pub async fn favorites(&self, user: &Identity) -> Result<FavoriteState> {
        ensure!(!user.subject.is_empty(), "로그인이 필요합니다");
        let _guard = self.save.lock().await;
        Ok(self.profile_locked(user).await?.state(user))
    }

    /// Go `Store.SaveFavorites`. `accessible` is the caller's catalog built
    /// from the *applied* settings: new default targets/addresses must be
    /// visible to the user right now.
    pub async fn save_favorites(
        &self,
        user: &Identity,
        expected: &str,
        mut profile: FavoriteProfile,
        accessible: &Catalog,
    ) -> Result<FavoriteState> {
        ensure!(!self.cfg.read_only, "read-only validation mode");
        ensure!(!user.subject.is_empty(), "로그인이 필요합니다");
        profile.validate()?;
        let single = profile.collections.len() == 1
            && profile.collections[0].id == "favorites"
            && profile.collections[0].name == "즐겨찾기";
        ensure!(user.admin || single, "이 계정은 즐겨찾기 하나를 사용합니다");

        let _guard = self.save.lock().await;
        self.refresh_locked().await?;
        let current = self.profile_locked(user).await?.state(user);
        if current.version != expected {
            return Err(Conflict(FAVORITE_CONFLICT).into());
        }
        if !profile.default_target.is_empty()
            && profile.default_target != current.profile.default_target
        {
            ensure!(
                accessible
                    .targets
                    .iter()
                    .any(|t| t.id == profile.default_target),
                "현재 사용할 수 없는 NodePort 대상입니다"
            );
        }
        for (card, endpoint) in &profile.default_endpoints {
            if current.profile.default_endpoints.get(card) == Some(endpoint) {
                continue; // unchanged choices stay valid even if hidden now
            }
            ensure!(
                accessible
                    .cards
                    .iter()
                    .any(|c| c.id == *card && c.endpoints.iter().any(|e| e.id == *endpoint)),
                "현재 볼 수 없는 주소는 기본값으로 지정할 수 없습니다"
            );
        }
        let next = profile.state(user);
        if next.version == current.version {
            return Ok(current);
        }
        match &self.repo {
            Some(repo) => {
                // Only this user's server-derived path is staged.
                let files = BTreeMap::from([(
                    repo.favorite_path(user),
                    repo.encode_favorites(user, &next.profile)?,
                )]);
                let message = format!(
                    "{}\n\nEdited by: {}",
                    personal_summary(&current.profile, &next.profile),
                    sanitize(&user.username)
                );
                let commit = repo.write(&files, &message).await?;
                self.inner.write().await.commit = commit;
            }
            None => {
                self.inner
                    .write()
                    .await
                    .profiles
                    .insert(user.subject.clone(), next.profile.clone());
            }
        }
        Ok(next)
    }

    /// Go `EnsureFavoriteAccounts`: create files only for accounts that have
    /// none, in one commit. Existing profiles are never reset, and nothing is
    /// deleted when an account is disabled. Legacy shared default addresses
    /// move once into the primary (first configured) admin's profile.
    pub async fn ensure_accounts(&self, users: &[Identity]) -> Result<usize> {
        if self.cfg.demo || self.cfg.read_only {
            return Ok(0);
        }
        ensure!(users.len() <= 1000, "too many favorite accounts");
        let _guard = self.save.lock().await;
        self.refresh_locked().await?;
        let repo = self.repo.as_ref().expect("production repository");
        let head = self.inner.read().await.commit.clone();
        let (mut settings, _) = self.desired().await;
        let mut files = BTreeMap::new();
        let (mut migrated, mut initialized) = (false, 0);
        for user in users {
            ensure!(
                !user.subject.is_empty() && user.subject.len() <= 256,
                "invalid favorite account"
            );
            let (mut profile, exists) = repo.read_favorites(user, &head).await?;
            let mut changed = !exists;
            if !exists {
                initialized += 1;
            }
            if self.cfg.admin_subjects.first() == Some(&user.subject) {
                if !settings.default_target.is_empty() {
                    if profile.default_target.is_empty() {
                        profile.default_target = settings.default_target.clone();
                    }
                    settings.default_target.clear();
                    migrated = true;
                    changed = true;
                }
                for (id, app) in &mut settings.apps {
                    if app.default_endpoint.is_empty() {
                        continue;
                    }
                    if profile
                        .default_endpoints
                        .get(id)
                        .is_none_or(String::is_empty)
                    {
                        profile
                            .default_endpoints
                            .insert(id.clone(), app.default_endpoint.clone());
                    }
                    app.default_endpoint.clear();
                    migrated = true;
                    changed = true;
                }
            }
            if changed {
                files.insert(
                    repo.favorite_path(user),
                    repo.encode_favorites(user, &profile)?,
                );
            }
        }
        if migrated {
            settings.revision = random_id();
            files.insert(self.cfg.git_path.clone(), repo.encode_settings(&settings)?);
        }
        if files.is_empty() {
            return Ok(0);
        }
        let message = format!(
            "Initialize {initialized} personal settings files; migrate legacy address defaults: {migrated}"
        );
        let commit = repo.write(&files, &message).await?;
        let mut s = self.inner.write().await;
        s.commit = commit;
        if migrated {
            s.desired = settings;
        }
        Ok(initialized)
    }
}

/// Go `changeSummary`: a commit subject listing what changed.
pub fn change_summary(old: &Settings, next: &Settings) -> String {
    let mut parts = vec![];
    for (ns, p) in &next.namespaces {
        if old.namespaces.get(ns) != Some(p) {
            parts.push(format!("namespace {ns} Ingress visibility"));
        }
    }
    for (ns, p) in &next.node_port_namespaces {
        if old.node_port_namespaces.get(ns) != Some(p) {
            parts.push(format!("namespace {ns} NodePort visibility"));
        }
    }
    for ns in old.node_port_namespaces.keys() {
        if !next.node_port_namespaces.contains_key(ns) {
            parts.push(format!("namespace {ns} NodePort visibility reset"));
        }
    }
    let empty = AppSettings::default();
    for (id, a) in &next.apps {
        let before = old.apps.get(id).unwrap_or(&empty);
        let mut fields = vec![];
        if !old.apps.contains_key(id) || a.name != before.name {
            fields.push("name");
        }
        if a.default_endpoint != before.default_endpoint {
            fields.push("default address");
        }
        if a.icon != before.icon {
            fields.push("icon");
        }
        if a.description != before.description {
            fields.push("description");
        }
        if a.hidden != before.hidden || a.display != before.display {
            fields.push("hidden");
        }
        if a.visibility != before.visibility {
            fields.push("visibility");
        }
        if a.endpoints != before.endpoints {
            fields.push("addresses");
        }
        if a.reviewed != before.reviewed {
            fields.push("review");
        }
        if !fields.is_empty() {
            let label = if a.name.is_empty() { id } else { &a.name };
            parts.push(format!("{label} ({})", fields.join(", ")));
        }
    }
    if old.manual != next.manual {
        parts.push("manual services".into());
    }
    if old.default_target != next.default_target {
        parts.push("default NodePort target".into());
    }
    if old.targets != next.targets {
        parts.push("node domains".into());
    }
    if old.assignments != next.assignments {
        parts.push("card grouping".into());
    }
    if parts.is_empty() {
        parts.push("settings".into());
    }
    parts.sort();
    let mut msg = sanitize(&format!("Update {}", parts.join("; ")));
    // Go cuts at byte 1000, possibly inside a character; the port cuts at
    // the last character boundary so commit messages stay valid UTF-8.
    let mut end = msg.len().min(1000);
    while !msg.is_char_boundary(end) {
        end -= 1;
    }
    msg.truncate(end);
    msg
}

/// Go `personalChangeSummary`.
pub fn personal_summary(before: &FavoriteProfile, after: &FavoriteProfile) -> String {
    let mut changes = vec![];
    if before.collections != after.collections {
        changes.push("favorite collections and membership");
    }
    if before.start_collection != after.start_collection {
        changes.push("start view");
    }
    if before.default_target != after.default_target {
        changes.push("default NodePort target");
    }
    if before.default_endpoints != after.default_endpoints {
        changes.push("default service addresses");
    }
    format!("Update personal settings: {}", changes.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    fn git(dir: &std::path::Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .current_dir(dir)
            .args([
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.test",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap().trim().into()
    }

    /// A bare "remote", a seed clone with empty settings, and a store using it.
    async fn fixture() -> (TempDir, Arc<Store>) {
        let dir = TempDir::new().unwrap();
        git(
            dir.path(),
            &["init", "--bare", "--initial-branch=main", "remote.git"],
        );
        git(dir.path(), &["init", "--initial-branch=main", "seed"]);
        let seed = dir.path().join("seed");
        let mut cfg = Config::from_lookup(|k, d| {
            if k == "KUBESHELF_DEMO" {
                "true".into()
            } else {
                d.into()
            }
        })
        .unwrap();
        cfg.demo = false;
        cfg.work_dir = dir.path().join("work").to_string_lossy().into_owned();
        cfg.git_url = dir.path().join("remote.git").to_string_lossy().into_owned();
        cfg.admin_subjects = vec!["admin".into()];
        let repo = Repository::for_local_tests(cfg.clone());
        let settings = Settings::empty();
        std::fs::create_dir_all(seed.join("runtime")).unwrap();
        std::fs::write(
            seed.join("runtime/settings.yaml"),
            repo.encode_settings(&settings).unwrap(),
        )
        .unwrap();
        git(&seed, &["add", "."]);
        git(&seed, &["commit", "-m", "seed"]);
        git(&seed, &["remote", "add", "origin", &cfg.git_url]);
        git(&seed, &["push", "origin", "main"]);
        repo.init().await.unwrap();
        let store = Arc::new(Store::with_repository(cfg, Some(repo), settings, vec![]));
        store.refresh().await.unwrap();
        (dir, store)
    }

    fn user(subject: &str) -> Identity {
        Identity {
            subject: subject.into(),
            username: subject.into(),
            admin: subject == "admin",
            ..Identity::default()
        }
    }

    fn is_conflict(e: &anyhow::Error) -> bool {
        e.downcast_ref::<Conflict>().is_some()
    }

    #[tokio::test]
    async fn git_profile_isolation_shared_cas_and_applied_boundary() {
        let (_dir, s) = fixture().await;
        let (admin, alice) = (user("admin"), user("alice"));
        assert_eq!(
            s.ensure_accounts(&[admin.clone(), alice.clone()])
                .await
                .unwrap(),
            2
        );
        let (mut shared, shared_version) = s.desired().await;

        // Personal save, then a stale retry is a conflict.
        let before = s.favorites(&alice).await.unwrap();
        let mut next = before.profile.clone();
        next.collections[0].cards = vec!["card-a".into()];
        let saved = s
            .save_favorites(&alice, &before.version, next.clone(), &Catalog::default())
            .await
            .unwrap();
        assert_eq!(saved.profile.collections[0].cards, vec!["card-a"]);
        let stale = s
            .save_favorites(&alice, &before.version, next, &Catalog::default())
            .await
            .unwrap_err();
        assert!(is_conflict(&stale), "{stale}");

        // Other accounts and the shared CAS token are untouched by personal commits.
        assert_eq!(
            s.favorites(&admin).await.unwrap().profile,
            FavoriteProfile::for_user(true)
        );
        assert_eq!(
            s.desired().await.1,
            shared_version,
            "personal commit must not conflict shared settings CAS"
        );

        // A shared save is pending until the projected file changes.
        shared.namespaces.insert(
            "public".into(),
            Policy {
                mode: "public".into(),
                ..Policy::default()
            },
        );
        s.save(&shared_version, shared.clone(), "admin")
            .await
            .unwrap();
        assert!(s.status().await.pending);
        assert!(
            s.applied().await.namespaces.is_empty(),
            "push cannot bypass projected Fleet apply"
        );
        let stale_shared = s.save(&shared_version, shared, "admin").await.unwrap_err();
        assert!(is_conflict(&stale_shared));

        // New defaults must be visible in the caller's catalog.
        let mut inaccessible = saved.profile.clone();
        inaccessible.default_target = "node-secret".into();
        assert!(
            s.save_favorites(&alice, &saved.version, inaccessible, &Catalog::default())
                .await
                .is_err()
        );

        // Provisioning is idempotent and never resets existing profiles.
        assert_eq!(s.ensure_accounts(&[admin, alice.clone()]).await.unwrap(), 0);
        assert_eq!(s.favorites(&alice).await.unwrap(), saved);
    }

    #[tokio::test]
    async fn legacy_defaults_move_to_primary_admin_once() {
        let (dir, s) = fixture().await;
        let seed = dir.path().join("seed");
        let mut legacy = Settings::empty();
        legacy.default_target = "node-a".into();
        legacy.apps.insert(
            "card".into(),
            AppSettings {
                default_endpoint: "ep".into(),
                ..AppSettings::default()
            },
        );
        std::fs::write(
            seed.join("runtime/settings.yaml"),
            s.repo.as_ref().unwrap().encode_settings(&legacy).unwrap(),
        )
        .unwrap();
        git(&seed, &["commit", "-am", "legacy"]);
        git(&seed, &["push", "origin", "main"]);
        assert_eq!(
            s.ensure_accounts(&[user("admin"), user("bob")])
                .await
                .unwrap(),
            2
        );
        let (shared, _) = s.desired().await;
        assert!(
            shared.default_target.is_empty() && shared.apps["card"].default_endpoint.is_empty()
        );
        let admin = s.favorites(&user("admin")).await.unwrap().profile;
        assert_eq!(admin.default_target, "node-a");
        assert_eq!(admin.default_endpoints["card"], "ep");
        assert!(
            s.favorites(&user("bob"))
                .await
                .unwrap()
                .profile
                .default_target
                .is_empty()
        );
    }

    #[tokio::test]
    async fn stale_push_rejected_without_overwriting_other_commit() {
        let (dir, s) = fixture().await;
        let repo = s.repo.as_ref().unwrap();
        let seed = dir.path().join("seed");
        std::fs::write(seed.join("unrelated.txt"), "external change").unwrap();
        git(&seed, &["add", "unrelated.txt"]);
        git(&seed, &["commit", "-m", "external"]);
        git(&seed, &["push", "origin", "main"]);
        let mut settings = Settings::empty();
        settings.revision = "stale-writer".into();
        let files = BTreeMap::from([(
            s.cfg.git_path.clone(),
            repo.encode_settings(&settings).unwrap(),
        )]);
        assert!(repo.write(&files, "stale update").await.is_err());
        s.refresh().await.unwrap();
        assert_eq!(s.desired().await.0.revision, "initial");
        assert_eq!(
            git(&seed, &["show", "origin/main:unrelated.txt"]),
            "external change"
        );
    }

    #[tokio::test]
    async fn favorite_manifest_subject_mismatch_rejected() {
        let (dir, s) = fixture().await;
        let repo = s.repo.as_ref().unwrap();
        let (alice, bob) = (user("alice"), user("bob"));
        let seed = dir.path().join("seed");
        let file = repo.favorite_path(&alice);
        std::fs::create_dir_all(seed.join("runtime/users")).unwrap();
        // Bob's manifest placed at Alice's path must not be accepted for Alice.
        std::fs::write(
            seed.join(&file),
            repo.encode_favorites(&bob, &FavoriteProfile::for_user(false))
                .unwrap(),
        )
        .unwrap();
        git(&seed, &["add", "."]);
        git(&seed, &["commit", "-m", "bad subject"]);
        git(&seed, &["push", "origin", "main"]);
        s.refresh().await.unwrap();
        assert!(s.favorites(&alice).await.is_err());
    }
}
