//! Catalog building (Go `catalog.go`).
//!
//! Turns a Kubernetes snapshot plus the applied settings into the per-user
//! `/api/catalog` response. The order of operations matters for security:
//!
//! 1. every endpoint is authorised with its *own* policy
//!    (endpoint → card → namespace; NodePorts use the NodePort namespace
//!    policy and never inherit the Ingress one);
//! 2. only then is it placed into a presentation group (`assignments`), so
//!    grouping can never widen access;
//! 3. NodePort targets (node IPs) are only returned when an endpoint that the
//!    user may see actually uses them.
//!
//! Kubernetes objects are kept as generic `serde_json::Value`s (the same JSON
//! the API server returns), read with small accessor helpers. Missing fields
//! read as `""`/`null`, which matches Go's zero values for absent fields.

use crate::{config::Config, gourl::GoUrl, model::*};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// The Go `Snapshot` struct. Field names stay PascalCase because Go encodes
/// untagged struct fields that way (the demo fixture was exported from Go).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct Snapshot {
    pub namespaces: Vec<Value>,
    pub services: Vec<Value>,
    pub pods: Vec<Value>,
    pub nodes: Vec<Value>,
    pub slices: Vec<Value>,
    pub ingresses: Vec<Value>,
    pub connected: bool,
    pub updated_at: String,
    pub problem: String,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            namespaces: vec![],
            services: vec![],
            pods: vec![],
            nodes: vec![],
            slices: vec![],
            ingresses: vec![],
            connected: false,
            updated_at: ZERO_TIME.into(),
            problem: String::new(),
        }
    }
}

/// String field accessor: non-strings read as "" (Go zero value).
pub fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}
/// Array field accessor: non-arrays read as empty.
fn a(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or_default()
}
fn name(v: &Value) -> &str {
    s(&v["metadata"]["name"])
}
fn ns(v: &Value) -> &str {
    s(&v["metadata"]["namespace"])
}
fn key(v: &Value) -> String {
    format!("{}/{}", ns(v), name(v))
}

/// The first `Ready` condition decides (Go `nodeReady` / last part of `podReady`).
fn ready_condition(v: &Value) -> bool {
    a(&v["status"]["conditions"])
        .iter()
        .find(|c| c["type"] == "Ready")
        .is_some_and(|c| c["status"] == "True")
}

fn pod_ready(v: &Value) -> bool {
    v["metadata"]["deletionTimestamp"].is_null()
        && v["status"]["phase"] == "Running"
        && ready_condition(v)
}

/// Go `labels.SelectorFromSet(selector).Matches(podLabels)`: every selector
/// key must exist on the pod with the same value. (No label syntax
/// validation happens in Go either; a non-empty selector is required by the
/// caller.)
fn selector_matches(selector: &serde_json::Map<String, Value>, pod: &Value) -> bool {
    let labels = &pod["metadata"]["labels"];
    selector
        .iter()
        .all(|(k, v)| labels.get(k).is_some_and(|l| l.is_string() && l == v))
}

/// Go `serviceHealth`: ready/total pods of a Service port, from selector
/// matches plus EndpointSlices for that port.
fn service_health(snap: &Snapshot, svc: &Value, port: &Value) -> Health {
    let mut h = Health {
        state: "unknown".into(),
        reason: "Pod 상태를 판단할 수 없습니다".into(),
        ..Health::default()
    };
    let pods: BTreeMap<String, &Value> = snap.pods.iter().map(|p| (key(p), p)).collect();
    let (mut total, mut ready, mut nodes) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
    // ExternalName Services, and slices whose endpoints are not Pods, are
    // "external": no pod health can be judged.
    let mut external = svc["spec"]["type"] == "ExternalName";

    if let Some(selector) = svc["spec"]["selector"]
        .as_object()
        .filter(|m| !m.is_empty())
    {
        for p in &snap.pods {
            let finished = ["Succeeded", "Failed"].contains(&s(&p["status"]["phase"]));
            if ns(p) == ns(svc) && selector_matches(selector, p) && !finished {
                total.insert(key(p));
            }
        }
    }

    for slice in &snap.slices {
        if ns(slice) != ns(svc)
            || s(&slice["metadata"]["labels"]["kubernetes.io/service-name"]) != name(svc)
        {
            continue;
        }
        // A slice belongs to this port if a slice port has the same name
        // (absent == "") and a compatible protocol.
        let port_matches = a(&slice["ports"]).iter().any(|p| {
            s(&p["name"]) == s(&port["name"])
                && (p["protocol"].is_null()
                    || p["protocol"] == port["protocol"]
                    || s(&port["protocol"]).is_empty())
        });
        if !port_matches {
            continue;
        }
        for ep in a(&slice["endpoints"]) {
            if ep["targetRef"]["kind"] != "Pod" {
                external = true;
                continue;
            }
            let pod_ns = match s(&ep["targetRef"]["namespace"]) {
                "" => ns(svc),
                other => other,
            };
            let id = format!("{pod_ns}/{}", s(&ep["targetRef"]["name"]));
            total.insert(id.clone());
            let Some(pod) = pods.get(&id) else { continue };
            // Go: Ready == nil || *Ready, Terminating == nil || !*Terminating.
            if pod_ready(pod)
                && ep["conditions"]["ready"] != false
                && ep["conditions"]["terminating"] != true
            {
                ready.insert(id);
                let node = s(&pod["spec"]["nodeName"]);
                if !node.is_empty() {
                    nodes.insert(node.to_owned());
                }
            }
        }
    }

    h.ready = ready.len() as i32;
    h.total = total.len() as i32;
    h.ready_nodes = nodes.into_iter().collect();
    if !snap.connected {
        h.reason = "Kubernetes 상태를 확인할 수 없습니다".into();
        h.ready_nodes.clear();
    } else if h.total == 0 && external {
        h.reason = "외부 백엔드 · Pod 상태 확인 대상 아님".into();
    } else if h.ready == 0 {
        h.state = "down".into();
        h.reason = "준비된 Pod 없음".into();
    } else if h.ready < h.total {
        h.state = "partial".into();
        h.reason = "일부 Pod 준비 중".into();
    } else {
        h.state = "healthy".into();
        h.reason = "Pod Ready".into();
    }
    h
}

/// Go `schemeForPort`: guess http/https/tcp from the port name/appProtocol.
fn scheme_for_port(p: &Value) -> String {
    if !["", "TCP"].contains(&s(&p["protocol"])) {
        return "tcp".into();
    }
    let mut hint = s(&p["name"]).to_lowercase();
    if let Some(app) = p["appProtocol"].as_str() {
        hint.push(' ');
        hint.push_str(&app.to_lowercase());
    }
    if hint.contains("https") {
        "https".into()
    } else if hint.contains("http") {
        "http".into()
    } else {
        "tcp".into()
    }
}

/// Go `namespacePolicy`: NodePorts expose node addresses and never inherit
/// an Ingress namespace policy. Unconfigured namespaces are admin-only.
fn namespace_policy(settings: &Settings, e: &Endpoint) -> Policy {
    let policies = if e.kind == "nodeport" {
        &settings.node_port_namespaces
    } else {
        &settings.namespaces
    };
    match policies.get(&e.namespace) {
        Some(p) if !e.namespace.is_empty() => p.clone(),
        _ => Policy::admin(),
    }
}

/// Go `endpointPolicy`: endpoint override → card override → namespace.
fn endpoint_policy(settings: &Settings, app: &AppSettings, e: &Endpoint) -> Policy {
    app.endpoints
        .get(&e.id)
        .and_then(|o| o.visibility.clone())
        .or_else(|| app.visibility.clone())
        .unwrap_or_else(|| namespace_policy(settings, e))
}

/// Go `appDisplay`: explicit display, else legacy `hidden`, else inherit.
fn app_display(a: &AppSettings) -> &str {
    if !a.display.is_empty() {
        &a.display
    } else if a.hidden {
        "hide"
    } else {
        "inherit"
    }
}

/// Go `endpointHidden`. A hidden presentation group hides all of its
/// addresses; showing a group never overrides the original service default.
fn endpoint_hidden(settings: &Settings, e: &Endpoint, app: &AppSettings) -> bool {
    let group_hidden = settings
        .assignments
        .get(&e.id)
        .filter(|g| !g.is_empty())
        .and_then(|g| settings.apps.get(g))
        .is_some_and(|g| app_display(g) == "hide");
    if group_hidden {
        return true;
    }
    match app_display(app) {
        "hide" => true,
        "show" => false,
        _ => namespace_policy(settings, e).hidden,
    }
}

/// Go `net.JoinHostPort`.
fn join_host_port(host: &str, port: i32) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

/// Endpoints discovered from Ingress rules (and default backends).
fn ingress_endpoints(
    snap: &Snapshot,
    cfg: &Config,
    user: &Identity,
    services: &BTreeMap<String, &Value>,
    endpoints: &mut BTreeMap<String, Endpoint>,
) {
    for ing in &snap.ingresses {
        let mut rules = a(&ing["spec"]["rules"]).to_vec();
        if !ing["spec"]["defaultBackend"].is_null() {
            // Go appends a synthetic host-less rule for the default backend.
            rules.push(json!({"http": {"paths": [{"path": "/", "backend": ing["spec"]["defaultBackend"]}]}}));
        }
        for rule in &rules {
            for path in a(&rule["http"]["paths"]) {
                let back = &path["backend"]["service"];
                if back.is_null() {
                    continue;
                }
                let service = s(&back["name"]);
                let svc = services.get(&format!("{}/{service}", ns(ing))).copied();
                // Resolve the backend port against the Service (by name or
                // number); otherwise keep what the Ingress says.
                let fallback =
                    json!({"port": back["port"]["number"], "name": back["port"]["name"]});
                let port_spec = svc
                    .and_then(|svc| {
                        a(&svc["spec"]["ports"]).iter().find(|p| {
                            let by_name = !s(&back["port"]["name"]).is_empty()
                                && back["port"]["name"] == p["name"];
                            let by_number = back["port"]["number"].as_i64().unwrap_or(0) != 0
                                && back["port"]["number"] == p["port"];
                            by_name || by_number
                        })
                    })
                    .unwrap_or(&fallback);
                let port = match port_spec["port"].as_i64().unwrap_or(0) {
                    0 => s(&port_spec["name"]).to_owned(),
                    n => n.to_string(),
                };
                let host = s(&rule["host"]);
                let raw_path = s(&path["path"]);
                let route = if raw_path.is_empty() { "/" } else { raw_path };
                let id = format!(
                    "endpoint-{}",
                    stable_id(&["ingress", ns(ing), service, &port, host, raw_path])
                );
                let tls = a(&ing["spec"]["tls"])
                    .iter()
                    .any(|t| a(&t["hosts"]).iter().any(|h| s(h) == host));
                let mut scheme = if tls { "https" } else { "http" };
                // Only an explicitly set ingressClassName can override (Go: pointer != nil).
                if let Some(class) = ing["spec"]["ingressClassName"].as_str()
                    && let Some(v) = cfg.ingress_schemes.get(class).filter(|v| !v.is_empty())
                {
                    scheme = v;
                }
                // Wildcard hosts and regex-like paths cannot be opened directly.
                let needs_url = host.is_empty()
                    || host.contains('*')
                    || route.contains(['*', '(', ')', '[', ']', '{', '}', '|', '^', '$']);
                let mut e = Endpoint {
                    id: id.clone(),
                    app_id: format!(
                        "auto-{}",
                        stable_id(&["ingress", &cfg.cluster, ns(ing), service, &port])
                    ),
                    label: format!("{host}{route}"),
                    kind: "ingress".into(),
                    namespace: ns(ing).into(),
                    service: service.into(),
                    port,
                    scheme: scheme.into(),
                    needs_url,
                    health: match svc {
                        Some(svc) => service_health(snap, svc, port_spec),
                        None => Health {
                            state: "unknown".into(),
                            reason: "Service를 찾을 수 없습니다".into(),
                            ..Health::default()
                        },
                    },
                    ..Endpoint::default()
                };
                if !needs_url {
                    e.url = GoUrl::from_parts(scheme, host, route).to_string();
                }
                if user.admin {
                    // Several Ingresses may route the same host/path to the same port.
                    let mut names = endpoints
                        .get(&id)
                        .map(|e| e.ingresses.clone())
                        .unwrap_or_default();
                    names.push(name(ing).into());
                    names.sort();
                    names.dedup();
                    e.ingresses = names;
                }
                endpoints.insert(id, e);
            }
        }
    }
}

/// Endpoints for every Service port that has a NodePort.
fn nodeport_endpoints(snap: &Snapshot, cfg: &Config, endpoints: &mut BTreeMap<String, Endpoint>) {
    for svc in &snap.services {
        for p in a(&svc["spec"]["ports"]) {
            let node_port = p["nodePort"].as_i64().unwrap_or(0) as i32;
            if node_port == 0 {
                continue;
            }
            let port = p["port"].as_i64().unwrap_or(0).to_string();
            let id = format!(
                "endpoint-{}",
                stable_id(&["nodeport", ns(svc), name(svc), &port])
            );
            let e = Endpoint {
                id: id.clone(),
                app_id: format!(
                    "auto-{}",
                    stable_id(&["nodeport", &cfg.cluster, ns(svc), name(svc), &port])
                ),
                label: name(svc).into(),
                kind: "nodeport".into(),
                namespace: ns(svc).into(),
                service: name(svc).into(),
                port,
                node_port,
                scheme: scheme_for_port(p),
                protocol: s(&p["protocol"]).into(),
                local: svc["spec"]["externalTrafficPolicy"] == "Local",
                health: service_health(snap, svc, p),
                ..Endpoint::default()
            };
            endpoints.insert(id, e);
        }
    }
}

/// Ready nodes (InternalIP, else ExternalIP) followed by admin-defined
/// domains the user may see; automatic nodes first, then by name.
fn connection_targets(snap: &Snapshot, settings: &Settings, user: &Identity) -> Vec<Target> {
    let mut targets = Vec::new();
    for n in &snap.nodes {
        if !ready_condition(n) {
            continue;
        }
        // Go takes the first address of each type, even if it is empty.
        let first = |kind: &str| {
            a(&n["status"]["addresses"])
                .iter()
                .find(|addr| s(&addr["type"]) == kind)
                .map(|addr| s(&addr["address"]))
                .unwrap_or_default()
        };
        let host = match first("InternalIP") {
            "" => first("ExternalIP"),
            ip => ip,
        };
        if !host.is_empty() {
            targets.push(Target {
                id: format!("node-{}", name(n)),
                name: name(n).into(),
                host: host.into(),
                node_name: name(n).into(),
                ..Target::default()
            });
        }
    }
    for t in &settings.targets {
        if t.visibility
            .clone()
            .unwrap_or_else(Policy::admin)
            .allows(user)
        {
            targets.push(Target {
                manual: true,
                ..t.clone()
            });
        }
    }
    targets.sort_by(|x, y| x.manual.cmp(&y.manual).then_with(|| x.name.cmp(&y.name)));
    targets
}

/// Go `BuildCatalog`.
pub fn build(snap: &Snapshot, settings: &Settings, cfg: &Config, user: &Identity) -> Catalog {
    let mut out = Catalog {
        connected: snap.connected,
        updated_at: if snap.updated_at.is_empty() {
            ZERO_TIME.into()
        } else {
            snap.updated_at.clone()
        },
        identity: user.clone(),
        demo: cfg.demo,
        applied_revision: settings.revision.clone(),
        problem: if user.admin {
            snap.problem.clone()
        } else {
            String::new()
        },
        ..Catalog::default()
    };

    // 1. Discover every endpoint (later inserts with the same id win, as in Go).
    let services: BTreeMap<String, &Value> = snap.services.iter().map(|v| (key(v), v)).collect();
    let mut endpoints: BTreeMap<String, Endpoint> = BTreeMap::new();
    ingress_endpoints(snap, cfg, user, &services, &mut endpoints);
    nodeport_endpoints(snap, cfg, &mut endpoints);
    for m in &settings.manual {
        for u in &m.urls {
            let scheme = GoUrl::parse(&u.url).map_or_else(|_| "https".into(), |p| p.scheme);
            endpoints.insert(
                u.id.clone(),
                Endpoint {
                    id: u.id.clone(),
                    app_id: m.id.clone(),
                    label: u.label.clone(),
                    url: u.url.clone(),
                    kind: "custom".into(),
                    service: m.name.clone(),
                    scheme,
                    health: Health {
                        state: "unknown".into(),
                        reason: "수동 등록 서비스".into(),
                        ..Health::default()
                    },
                    ..Endpoint::default()
                },
            );
        }
    }
    let targets = connection_targets(snap, settings, user);

    // 2. Authorise, decorate and group (BTreeMap iterates ids in sorted order,
    //    like Go's sort.Strings(ids)).
    let mut cards: BTreeMap<String, Card> = BTreeMap::new();
    let mut used_targets = BTreeSet::new();
    let mut ns_services: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut ns_kinds: BTreeMap<(String, String), i32> = BTreeMap::new();
    for (_, mut e) in endpoints {
        // Namespace statistics count every discovered service, visible or not.
        if ns_services
            .entry(e.namespace.clone())
            .or_default()
            .insert(e.app_id.clone())
        {
            *ns_kinds
                .entry((e.namespace.clone(), e.kind.clone()))
                .or_default() += 1;
        }
        let app = settings.apps.get(&e.app_id).cloned().unwrap_or_default();
        let hidden = endpoint_hidden(settings, &e, &app);
        if !endpoint_policy(settings, &app, &e).allows(user) || (hidden && !user.admin) {
            continue;
        }

        let overrides = app.endpoints.get(&e.id).cloned().unwrap_or_default();
        if !overrides.label.is_empty() {
            e.label = overrides.label;
        }
        if !overrides.scheme.is_empty() {
            e.scheme = overrides.scheme;
        }
        if !overrides.url.is_empty() {
            e.url = overrides.url;
            e.needs_url = false;
        } else if !e.url.is_empty()
            && e.scheme != "tcp"
            && let Ok(mut u) = GoUrl::parse(&e.url)
        {
            // Re-render with the (possibly overridden) scheme, like Go's
            // url.Parse + u.Scheme = … + u.String().
            u.scheme = e.scheme.clone();
            e.url = u.to_string();
        }

        if e.kind == "nodeport" {
            for t in &targets {
                // externalTrafficPolicy: Local only answers on nodes with a ready pod.
                if e.local
                    && (t.node_name.is_empty() || !e.health.ready_nodes.contains(&t.node_name))
                {
                    continue;
                }
                let address = join_host_port(&t.host, e.node_port);
                let url = if !e.url.is_empty() {
                    e.url.clone()
                } else if e.scheme == "http" || e.scheme == "https" {
                    format!("{}://{address}", e.scheme)
                } else {
                    String::new()
                };
                e.targets.push(Connection {
                    target_id: t.id.clone(),
                    name: t.name.clone(),
                    url,
                    address,
                    node_name: t.node_name.clone(),
                });
            }
        }
        e.fingerprint = stable_id(&[
            &e.id,
            &e.url,
            &e.scheme,
            &e.node_port.to_string(),
            &e.protocol,
        ]);

        // Authorisation above used the original endpoint policy; grouping is
        // presentation only.
        let group = settings
            .assignments
            .get(&e.id)
            .filter(|g| !g.is_empty())
            .cloned()
            .unwrap_or_else(|| e.app_id.clone());
        let meta = settings.apps.get(&group).cloned().unwrap_or_default();
        let card = cards
            .entry(format!("{}/{group}", e.kind))
            .or_insert_with(|| Card {
                id: group.clone(),
                name: [&meta.name, &app.name, &e.service]
                    .into_iter()
                    .find(|n| !n.is_empty())
                    .cloned()
                    .unwrap_or_default(),
                icon: if meta.icon.is_empty() {
                    app.icon.clone()
                } else {
                    meta.icon.clone()
                },
                description: if meta.description.is_empty() {
                    app.description.clone()
                } else {
                    meta.description.clone()
                },
                source: e.kind.clone(),
                namespace: e.namespace.clone(),
                hidden: true,
                ..Card::default()
            });
        card.hidden &= hidden;
        if user.admin {
            e.hidden = hidden;
        }
        used_targets.extend(e.targets.iter().map(|t| t.target_id.clone()));
        if e.kind != "custom" {
            match app.reviewed.get(&e.id) {
                None => card.new = true,
                Some(prior) if *prior != e.fingerprint => card.changed = true,
                Some(_) => {}
            }
        }
        if !user.admin {
            // Fingerprints and node placement are admin-only details.
            e.fingerprint.clear();
            e.health.ready_nodes.clear();
        }
        card.endpoints.push(e);
    }

    for (_, mut card) in cards {
        if let Some(app) = settings.apps.get(&card.id)
            && card.endpoints.iter().any(|e| e.id == app.default_endpoint)
        {
            card.default_endpoint = app.default_endpoint.clone();
        }
        out.cards.push(card);
    }
    // Go uses an unstable sort on the lower-cased name; Rust's stable sort
    // gives the same order whenever names differ.
    out.cards.sort_by_cached_key(|c| go_lower(&c.name));

    // 3. Only expose targets that a visible endpoint uses (admins see all).
    for mut t in targets {
        if user.admin || used_targets.contains(&t.id) {
            t.visibility = None;
            if t.id == settings.default_target {
                out.default_target = t.id.clone();
            }
            out.targets.push(t);
        }
    }

    if user.admin {
        for n in &snap.namespaces {
            let nm = name(n);
            // Namespace rows follow discovered endpoints, not namespace age.
            let services = ns_services.get(nm).map_or(0, BTreeSet::len);
            if services == 0 {
                continue;
            }
            let ingress = settings.namespaces.get(nm);
            let nodeport = settings.node_port_namespaces.get(nm);
            let count = |kind: &str| {
                *ns_kinds
                    .get(&(nm.to_owned(), kind.to_owned()))
                    .unwrap_or(&0)
            };
            let (ingress_count, nodeport_count) = (count("ingress"), count("nodeport"));
            out.namespaces.push(NamespaceInfo {
                name: nm.into(),
                configured: (ingress_count == 0 || ingress.is_some())
                    && (nodeport_count == 0 || nodeport.is_some()),
                services: services as i32,
                policy: ingress.cloned().unwrap_or_else(Policy::admin),
                ingress_configured: ingress.is_some(),
                ingress_services: ingress_count,
                node_port_configured: nodeport.is_some(),
                node_port_services: nodeport_count,
                node_port_policy: nodeport.cloned().unwrap_or_else(Policy::admin),
            });
        }
        out.namespaces.sort_by(|x, y| x.name.cmp(&y.name));
    }
    out
}
