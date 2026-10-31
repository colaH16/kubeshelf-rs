//! Kubernetes snapshot source (Go `source.go`, `user_settings_watch.go`).
//!
//! * Every 10 s, six cluster-wide lists run in parallel (Namespaces,
//!   Services, Pods, Nodes, EndpointSlices, Ingresses) with
//!   `resourceVersion=0`, so the API server may answer from its watch cache.
//!   If any list fails, the previous snapshot is kept and marked disconnected.
//! * Personal ConfigMaps (`kubeshelf.io/user-settings=true`) are watched in
//!   the pod namespace. When Fleet applies a batch of them, changes are
//!   coalesced into one Git refresh, retried every 5 s until it succeeds.
//!   There is no periodic Git poll.
//!
//! Read-only RBAC is sufficient: list on the six kinds and list/watch on
//! ConfigMaps in the release namespace (see the Helm chart).

use crate::{
    catalog::Snapshot,
    config::Config,
    model::{Settings, now},
    store::Store,
};
use anyhow::Result;
use futures::{StreamExt, pin_mut};
use kube::{
    Api, Client,
    api::{ApiResource, DynamicObject, GroupVersionKind, ListParams},
    runtime::{WatchStreamExt, watcher},
};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio::sync::{RwLock, mpsc};

pub struct Source {
    /// Replaced atomically; readers clone the `Arc`, not the data.
    snapshot: RwLock<Arc<Snapshot>>,
    client: Option<Client>,
}

/// (group, version, kind) in `Snapshot` field order.
const KINDS: [(&str, &str, &str); 6] = [
    ("", "v1", "Namespace"),
    ("", "v1", "Service"),
    ("", "v1", "Pod"),
    ("", "v1", "Node"),
    ("discovery.k8s.io", "v1", "EndpointSlice"),
    ("networking.k8s.io", "v1", "Ingress"),
];

fn api(
    client: Client,
    (group, version, kind): (&str, &str, &str),
    namespace: Option<&str>,
) -> Api<DynamicObject> {
    let resource = ApiResource::from_gvk(&GroupVersionKind::gvk(group, version, kind));
    match namespace {
        Some(ns) => Api::namespaced_with(client, ns, &resource),
        None => Api::all_with(client, &resource),
    }
}

/// The synthetic demo cluster and settings, exported from the original Go
/// `NewDemoSource`/`demoSettings` (documentation addresses only).
pub fn demo() -> (Snapshot, Settings) {
    let v: serde_json::Value =
        serde_json::from_str(include_str!("../tests/demo.json")).expect("checked demo fixture");
    let mut snapshot: Snapshot =
        serde_json::from_value(v["snapshot"].clone()).expect("demo snapshot");
    snapshot.updated_at = now();
    let settings = serde_json::from_value(v["settings"].clone()).expect("demo settings");
    (snapshot, settings)
}

impl Source {
    pub async fn open(cfg: &Config) -> Result<Arc<Self>> {
        if cfg.demo {
            return Ok(Arc::new(Self {
                snapshot: RwLock::new(Arc::new(demo().0)),
                client: None,
            }));
        }
        let mut kube_config = if cfg.kubeconfig.is_empty() {
            kube::Config::incluster()?
        } else {
            let file = kube::config::Kubeconfig::read_from(&cfg.kubeconfig)?;
            kube::Config::from_custom_kubeconfig(file, &kube::config::KubeConfigOptions::default())
                .await?
        };
        kube_config.connect_timeout = Some(Duration::from_secs(10));
        // Long enough for watch requests; plain lists use their own 10 s deadline.
        kube_config.read_timeout = Some(Duration::from_secs(330));
        let source = Arc::new(Self {
            snapshot: RwLock::new(Arc::new(Snapshot::default())),
            client: Some(Client::try_from(kube_config)?),
        });
        source.refresh().await;
        Ok(source)
    }

    /// The current snapshot (cheap: clones an `Arc`).
    pub async fn snapshot(&self) -> Arc<Snapshot> {
        self.snapshot.read().await.clone()
    }

    async fn refresh(&self) {
        let Some(client) = &self.client else { return };
        let lists = KINDS.iter().map(|kind| {
            let api = api(client.clone(), *kind, None);
            async move {
                let list = api.list(&ListParams::default().match_any()).await?;
                list.items
                    .into_iter()
                    .map(serde_json::to_value)
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(anyhow::Error::from)
            }
        });
        let all = tokio::time::timeout(
            Duration::from_secs(10),
            futures::future::try_join_all(lists),
        )
        .await;
        let mut current = self.snapshot.write().await;
        match all {
            Ok(Ok(lists)) => {
                let [namespaces, services, pods, nodes, slices, ingresses]: [Vec<_>; 6] =
                    lists.try_into().expect("six lists");
                *current = Arc::new(Snapshot {
                    namespaces,
                    services,
                    pods,
                    nodes,
                    slices,
                    ingresses,
                    connected: true,
                    updated_at: now(),
                    problem: String::new(),
                });
            }
            failed => {
                if let Ok(Err(e)) = failed {
                    tracing::warn!("Kubernetes list failed: {e:#}");
                }
                let mut stale = (**current).clone();
                stale.connected = false;
                stale.problem = "Kubernetes 동기화가 지연되고 있습니다".into();
                *current = Arc::new(stale);
            }
        }
    }

    /// Refresh the snapshot every 10 s.
    pub async fn run(self: Arc<Self>) {
        if self.client.is_none() {
            return;
        }
        loop {
            tokio::time::sleep(Duration::from_secs(10)).await;
            self.refresh().await;
        }
    }

    /// Go `RunUserSettings`: watch personal ConfigMaps and refresh the Git
    /// snapshot when they change.
    ///
    /// Like a client-go informer, an event only counts when an object was
    /// added, deleted, or its `resourceVersion` changed; a watch restart that
    /// re-lists identical objects triggers nothing.
    pub async fn watch_users(self: Arc<Self>, store: Arc<Store>) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let (notify, mut changes) = mpsc::channel::<()>(1);
        let refresher = store.clone();
        let watching = async move {
            let api = api(client, ("", "v1", "ConfigMap"), Some(&store.cfg.namespace));
            let config = watcher::Config::default().labels("kubeshelf.io/user-settings=true");
            let stream = watcher(api, config).default_backoff();
            pin_mut!(stream);
            let mut known: BTreeMap<String, String> = BTreeMap::new();
            let mut relist: BTreeMap<String, String> = BTreeMap::new();
            let identity = |o: &DynamicObject| {
                let name = o.metadata.name.clone().unwrap_or_default();
                (
                    name,
                    o.metadata.resource_version.clone().unwrap_or_default(),
                )
            };
            while let Some(event) = stream.next().await {
                let changed = match event {
                    Ok(watcher::Event::Init) => {
                        relist.clear();
                        false
                    }
                    Ok(watcher::Event::InitApply(o)) => {
                        let (name, version) = identity(&o);
                        relist.insert(name, version);
                        false
                    }
                    Ok(watcher::Event::InitDone) => {
                        let changed = relist != known;
                        known = std::mem::take(&mut relist);
                        changed
                    }
                    Ok(watcher::Event::Apply(o)) => {
                        let (name, version) = identity(&o);
                        known.insert(name, version.clone()) != Some(version)
                    }
                    Ok(watcher::Event::Delete(o)) => {
                        known.remove(&identity(&o).0);
                        true
                    }
                    Err(e) => {
                        tracing::warn!("personal settings watch error (retrying): {e}");
                        false
                    }
                };
                if changed {
                    let _ = notify.try_send(()); // a pending signal already covers it
                }
            }
        };
        let refreshing = async move {
            while changes.recv().await.is_some() {
                // Coalesce a Fleet batch of account ConfigMaps into one fetch.
                let mut delay = Duration::from_secs(1);
                loop {
                    tokio::time::sleep(delay).await;
                    while changes.try_recv().is_ok() {}
                    match tokio::time::timeout(Duration::from_secs(30), refresher.refresh()).await {
                        Ok(Ok(())) => break,
                        Ok(Err(e)) => tracing::warn!("user settings refresh delayed: {e:#}"),
                        Err(_) => tracing::warn!("user settings refresh delayed: timeout"),
                    }
                    delay = Duration::from_secs(5);
                }
            }
        };
        tokio::join!(watching, refreshing);
    }
}
