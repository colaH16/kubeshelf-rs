#!/usr/bin/env python3
"""Differential tests: the Rust port against the original Go implementation.

Each case is a JSON document sent to two small adapters that call the real
functions of each implementation:

  Go:   tests/go-reference/*.go, built from the pinned original commit by
        scripts/build_reference.py
  Rust: src/bin/reference.rs (cargo build --bin reference)

No cluster, network, or credentials are used. All addresses are documentation
ranges (192.0.2.0/24, 2001:db8::/32) and example.* domains.

Usage:
  KUBESHELF_GO_REFERENCE=/path/to/go-reference python3 tests/compare.py
Optional: KUBESHELF_RUST_REFERENCE (default target/debug/reference).
"""
import concurrent.futures
import copy
import json
import os
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
GO = os.environ["KUBESHELF_GO_REFERENCE"]
RUST = os.environ.get("KUBESHELF_RUST_REFERENCE", str(ROOT / "target/debug/reference"))
DEMO = json.loads((ROOT / "tests/demo.json").read_text())

# Cases where the Rust port is intentionally stricter than Go. For these the
# check is: Go accepts, Rust returns an error. Keep docs/compatibility.md in sync.
RUST_STRICTER = {
    "decode: trailing data after the JSON document",
    "decode: field name in different letter case",
    "store: manifest path with ./ prefix",
    "store: manifest path with .. in the middle",
    "store: manifest path with empty segment",
    "store: manifest path with trailing slash",
    "store: manifest path with nested .git directory",
    "read-settings: number as an extra data value",
}
# Go cuts commit subjects at byte 1000 (possibly mid-character); Rust cuts at
# the previous character boundary. Checked as: Go text minus U+FFFD == Rust.
UTF8_TRUNCATION = {"change-summary truncation"}
# Both reject the input, with different messages.
BOTH_REJECT = set()

cases = []


def add(label, op, **fields):
    cases.append((label, dict(Op=op, **fields)))


# --------------------------------------------------------------------------
# Kubernetes object builders (API JSON shape)
# --------------------------------------------------------------------------
def meta(name, namespace=None, labels=None, **extra):
    m = {"name": name}
    if namespace:
        m["namespace"] = namespace
    if labels:
        m["labels"] = labels
    m.update(extra)
    return m


def namespace(name):
    return {"metadata": meta(name)}


def node(name, addresses, ready=True):
    return {
        "metadata": meta(name),
        "status": {
            "addresses": [{"type": t, "address": a} for t, a in addresses],
            "conditions": [{"type": "Ready", "status": "True" if ready else "False"}],
        },
    }


def service(ns, name, ports, selector=None, kind=None, local=False):
    spec = {"ports": ports}
    if selector is not None:
        spec["selector"] = selector
    if kind:
        spec["type"] = kind
    if local:
        spec["externalTrafficPolicy"] = "Local"
    return {"metadata": meta(name, ns), "spec": spec}


def pod(ns, name, labels, node_name, phase="Running", ready=True, deleting=False):
    m = meta(name, ns, labels)
    if deleting:
        m["deletionTimestamp"] = "2026-01-01T00:00:00Z"
    return {
        "metadata": m,
        "spec": {"nodeName": node_name},
        "status": {"phase": phase, "conditions": [{"type": "Ready", "status": "True" if ready else "False"}]},
    }


def endpoint_slice(ns, svc, ports, endpoints, name=None):
    return {
        "metadata": meta(name or svc + "-slice", ns, {"kubernetes.io/service-name": svc}),
        "ports": ports,
        "endpoints": endpoints,
    }


def target_pod(name, ns=None, ready=None, terminating=None, kind="Pod"):
    ref = {"kind": kind, "name": name}
    if ns:
        ref["namespace"] = ns
    conditions = {}
    if ready is not None:
        conditions["ready"] = ready
    if terminating is not None:
        conditions["terminating"] = terminating
    return {"targetRef": ref, "conditions": conditions}


def ingress(ns, name, rules, tls=None, cls=None, default_backend=None):
    spec = {"rules": rules}
    if tls is not None:
        spec["tls"] = tls
    if cls is not None:
        spec["ingressClassName"] = cls
    if default_backend is not None:
        spec["defaultBackend"] = default_backend
    return {"metadata": meta(name, ns), "spec": spec}


def rule(host, *paths):
    r = {"http": {"paths": [{"path": p, "pathType": "Prefix", "backend": {"service": b}} for p, b in paths]}}
    if host is not None:
        r["host"] = host
    return r


def backend(name, number=None, port_name=None):
    port = {}
    if number is not None:
        port["number"] = number
    if port_name is not None:
        port["name"] = port_name
    return {"name": name, "port": port}


def snapshot(**kinds):
    snap = {"Namespaces": [], "Services": [], "Pods": [], "Nodes": [], "Slices": [], "Ingresses": [],
            "Connected": True, "UpdatedAt": "2026-01-01T00:00:00Z", "Problem": "sync delayed"}
    snap.update(kinds)
    return snap


# --------------------------------------------------------------------------
# A synthetic cluster exercising health, routing and URL edge cases
# --------------------------------------------------------------------------
http = {"name": "http", "port": 80, "protocol": "TCP", "targetPort": 8080}
edge = snapshot(
    Namespaces=[namespace(n) for n in ["apps", "ops", "empty", "ext"]],
    Nodes=[
        node("n1", [("InternalIP", "192.0.2.21")]),
        node("n2", [("InternalIP", ""), ("ExternalIP", "192.0.2.22")]),
        node("n3", [("ExternalIP", "192.0.2.23")]),
        node("n4", [("InternalIP", "2001:db8::4")]),
        node("n5", [("InternalIP", "192.0.2.25")], ready=False),
        node("n6", [("Hostname", "n6")]),
    ],
    Services=[
        service("apps", "web", [http, {"name": "metrics", "port": 9090, "protocol": "TCP"}], {"app": "web"}),
        service("apps", "partial", [http], {"app": "partial"}),
        service("apps", "named", [{"name": "https-alt", "port": 8443, "protocol": "TCP"}], {"app": "named"}),
        service("ops", "local", [{"name": "web", "port": 80, "nodePort": 30080, "protocol": "TCP"}], {"app": "local"},
                kind="NodePort", local=True),
        service("ops", "dns", [{"name": "dns", "port": 53, "nodePort": 30053, "protocol": "UDP"}], {"app": "dns"},
                kind="NodePort"),
        service("ops", "grpc", [{"name": "api", "port": 9000, "nodePort": 30900, "protocol": "TCP",
                                 "appProtocol": "kubernetes.io/h2c"}], {"app": "grpc"}, kind="NodePort"),
        service("ops", "tls", [{"name": "admin", "port": 443, "nodePort": 30443, "appProtocol": "HTTPS"}], {},
                kind="NodePort"),
        service("ext", "external", [http], kind="ExternalName"),
        service("ext", "manual-backend", [http]),
    ],
    Pods=[
        pod("apps", "web-1", {"app": "web"}, "n1"),
        pod("apps", "web-2", {"app": "web"}, "n2"),
        pod("apps", "web-done", {"app": "web"}, "n1", phase="Succeeded"),
        pod("apps", "web-gone", {"app": "web"}, "n1", deleting=True),
        pod("apps", "partial-1", {"app": "partial"}, "n1"),
        pod("apps", "partial-2", {"app": "partial"}, "n2", ready=False),
        pod("apps", "named-1", {"app": "named", "x": "y"}, "n4"),
        pod("ops", "local-1", {"app": "local"}, "n1"),
        pod("ops", "local-2", {"app": "local"}, "n4"),
        pod("ops", "local-3", {"app": "local"}, "n5"),
        pod("other", "web-elsewhere", {"app": "web"}, "n3"),
    ],
    Slices=[
        endpoint_slice("apps", "web", [{"name": "http", "port": 8080, "protocol": "TCP"}],
                       [target_pod("web-1", ready=True), target_pod("web-2"), target_pod("web-gone", ready=True)]),
        endpoint_slice("apps", "web", [{"name": "metrics", "port": 9090}],
                       [target_pod("web-1", terminating=True)], name="web-metrics"),
        endpoint_slice("apps", "partial", [{"name": "http", "protocol": "UDP"}], [target_pod("partial-1")]),
        endpoint_slice("apps", "partial", [{"name": "http"}],
                       [target_pod("partial-1"), target_pod("partial-2", ready=False)], name="partial-b"),
        endpoint_slice("apps", "named", [{"name": "https-alt"}], [target_pod("named-1", ns="apps")]),
        endpoint_slice("ops", "local", [{"name": "web"}],
                       [target_pod("local-1"), target_pod("local-2"), target_pod("local-3", ready=False)]),
        endpoint_slice("ext", "manual-backend", [{"name": "http"}], [{"addresses": ["192.0.2.99"]}]),
    ],
    Ingresses=[
        ingress("apps", "web", [rule("web.example.com", ("/", backend("web", 80)), ("/metrics", backend("web", port_name="metrics")))],
                tls=[{"hosts": ["web.example.com"]}]),
        ingress("apps", "web-alias", [rule("web.example.com", ("/", backend("web", 80)))]),
        ingress("apps", "odd-paths", [rule("odd.example.com",
                                           ("/a b/ü", backend("web", 80)),
                                           ("/q?x=1#f", backend("web", 80)),
                                           ("/100%", backend("web", 80)),
                                           ("/re(gex)", backend("web", 80)),
                                           ("", backend("web", 80)),
                                           ("/$&+,:;=@!'*", backend("web", 80)))]),
        ingress("apps", "wild", [rule("*.example.com", ("/", backend("partial", 80)))], cls="nginx"),
        ingress("apps", "nohost", [rule(None, ("/", backend("partial", 80)))]),
        ingress("apps", "named", [rule("named.example.com", ("/", backend("named", port_name="https-alt")))], cls="plain-class"),
        ingress("apps", "unicode", [rule("例え.example", ("/", backend("named", 8443)))], cls=""),
        ingress("apps", "missing", [rule("missing.example.com", ("/", backend("absent", 80)))]),
        ingress("apps", "unmatched-port", [rule("port.example.com", ("/", backend("web", 81)))]),
        ingress("ext", "external", [rule("ext.example.com", ("/", backend("external", 80)))],
                default_backend={"service": backend("manual-backend", 80)}),
        ingress("ext", "resource-backend", [{"host": "res.example.com", "http": {"paths": [
            {"path": "/", "backend": {"resource": {"kind": "Bucket", "name": "b"}}}]}}]),
    ],
)

ids = {}


def discover(identity):
    """Ask the Go oracle for endpoint/app ids so settings can refer to them."""
    data = json.dumps(dict(Op="catalog", Snapshot=edge, Settings={"schemaVersion": 1, "revision": "r"},
                           Identity=identity, Schemes={"nginx": "https"})).encode()
    out = json.loads(subprocess.check_output([GO], input=data))
    for card in out["cards"]:
        for e in card["endpoints"]:
            ids[(e["service"], e["label"])] = (e["id"], e["appId"], e["fingerprint"])


discover({"subject": "root", "admin": True})


def eid(service, label):
    return ids[(service, label)][0]


def app(service, label):
    return ids[(service, label)][1]


base = {
    "schemaVersion": 1, "revision": "edge",
    "namespaces": {"apps": {"mode": "public"}, "ext": {"mode": "restricted", "groups": ["family"], "hidden": True}},
    "nodePortNamespaces": {"ops": {"mode": "restricted", "users": ["user"]}},
    "apps": {
        app("web", "web.example.com/"): {"name": "Web", "icon": "globe", "description": "main",
                                          "defaultEndpoint": eid("web", "web.example.com/metrics"),
                                          "reviewed": {eid("web", "web.example.com/"): ids[("web", "web.example.com/")][2],
                                                       eid("web", "web.example.com/metrics"): "stale"},
                                          "endpoints": {eid("web", "web.example.com/metrics"): {
                                              "label": "Metrics", "scheme": "http",
                                              "visibility": {"mode": "admin"}}}},
        app("partial", "*.example.com/"): {"display": "show"},
        app("named", "named.example.com/"): {"visibility": {"mode": "restricted", "groups": ["family"]},
                                              "endpoints": {eid("named", "named.example.com/"): {"url": "https://override.example.com/x y"}}},
        app("dns", "dns"): {"hidden": True},
        app("tls", "tls"): {"endpoints": {eid("tls", "tls"): {"scheme": "tcp"}}},
        "group-shared": {"name": "Shared group", "icon": "https://icons.example.com/i.png"},
        "group-hidden": {"name": "Hidden group", "display": "hide"},
        "manual-docs": {"name": "Docs", "description": "manual"},
    },
    "manual": [
        {"id": "manual-docs", "name": "Docs", "urls": [
            {"id": "manual-docs-1", "label": "Main", "url": "HTTPS://Docs.Example.com/a b?q=1#frag"},
            {"id": "manual-docs-2", "label": "Unicode", "url": "http://例え.example/パス"},
            {"id": eid("web", "web.example.com/"), "label": "collides", "url": "https://collide.example.com"}]},
    ],
    "targets": [
        {"id": "domain-public", "name": "edge.example.com", "host": "edge.example.com", "visibility": {"mode": "public"}, "tcpEnabled": True},
        {"id": "domain-family", "name": "family.example.com", "host": "192.0.2.50", "nodeName": "n1",
         "visibility": {"mode": "restricted", "groups": ["family"]}},
        {"id": "domain-admin", "name": "admin.example.com", "host": "2001:db8::50"},
    ],
    "defaultTarget": "domain-public",
    "assignments": {
        eid("partial", "*.example.com/"): "group-shared",
        eid("named", "named.example.com/"): "group-shared",
        eid("grpc", "grpc"): "group-hidden",
    },
}

identities = {
    "anonymous": {"groups": []},
    "user": {"subject": "user", "username": "user", "groups": ["family"]},
    "other": {"subject": "other", "groups": ["guests"]},
    "admin": {"subject": "root", "admin": True, "groups": []},
}
schemes_variants = [None, {"nginx": "https", "": "https", "plain-class": "http"}]
for who, identity in identities.items():
    for connected in [True, False]:
        for schemes in schemes_variants:
            snap = copy.deepcopy(edge)
            snap["Connected"] = connected
            fields = dict(Snapshot=snap, Settings=base, Identity=identity)
            if schemes is not None:
                fields["Schemes"] = schemes
            add(f"catalog edge {who} connected={connected} schemes={schemes is not None}", "catalog", **fields)

# Display and grouping matrix on the edge cluster.
for display in ["", "inherit", "show", "hide"]:
    for group_display in ["", "show", "hide"]:
        st = copy.deepcopy(base)
        for k in list(st["apps"]):
            if k.startswith("auto-"):
                st["apps"][k]["display"] = display
        st["apps"]["group-shared"]["display"] = group_display
        for who in ["user", "admin"]:
            add(f"catalog display={display} group={group_display} {who}", "catalog",
                Snapshot=edge, Settings=st, Identity=identities[who])

# Demo fixture (the shipped demo) under the original policy matrix.
for who, identity in identities.items():
    for connected in [True, False]:
        snap = copy.deepcopy(DEMO["snapshot"])
        snap["Connected"] = connected
        add(f"catalog demo {who} connected={connected}", "catalog", Snapshot=snap, Settings=DEMO["settings"], Identity=identity)
for kind in ["namespaces", "nodePortNamespaces"]:
    for mode in ["public", "restricted", "admin"]:
        for hidden in [True, False]:
            st = copy.deepcopy(DEMO["settings"])
            st[kind] = {k: dict(mode=mode, hidden=hidden, groups=["family"]) for k in ["public-services", "myapps", "infra"]}
            for who, identity in identities.items():
                add(f"catalog demo {kind} {mode} hidden={hidden} {who}", "catalog",
                    Snapshot=DEMO["snapshot"], Settings=st, Identity=identity)

# --------------------------------------------------------------------------
# URL parsing/rendering (Go net/url) and host validation
# --------------------------------------------------------------------------
for raw in [
    "https://example.com", "http://example.com/", "HTTPS://Example.COM/Path", "http://x.com/a b", "http://x.com/a%20b",
    "http://x.com/a%2fb", "http://x.com/%zz", "http://x.com:99999", "http://x.com:", "http://x.com:8a", "http://[::1]:80/p",
    "http://[::ffff:192.0.2.1]/", "http://[fe80::1%25en0]/", "http://[2001:db8::1", "http://x.com?", "http://x.com?a=b?c",
    "http://x.com/#", "http://x.com/#frag ment", "http://例え.example/パス", "http://%E4%BE%8B.example/", "http://%41.example/",
    "http://a{b.com", "http://a<b>.com", "http://a b.com", "http:/x.com", "http:x.com", "http:///x.com", "//x.com/p",
    " http://x.com", "http://x.com ", "http://x.com/\x01", "http://x.com/\\", "ftp://x.com", "javascript:alert(1)",
    "http://user@x.com", "http://user:pw@x.com", "http://@x.com", "http://x.com@evil.com", "http://x.com?q=@a",
    "http://x.com/@a", "http://x.com/;p,a$t&h=+", "http://x.com/~_.-!*'()", "http://x.com/[a]", "https://x.com/%E2%82%AC",
    "http://-x.com", "http://x..com", "http://x.com./", "*", "", "http://", "http://:80", "mailto:a@example.com",
]:
    add(f"url {raw!r}", "url", Raw=raw)
for host in ["edge.example.com", "192.0.2.1", "2001:db8::1", "::ffff:192.0.2.1", "fe80::1%eth0", "01.2.3.4", "a_b.example",
             "-a.example", "a-.example", "a..b", "a.", "x" * 64 + ".example", ("a." * 127) + "a", "edge.example.com:443",
             "https://edge.example.com", "ex ample", "例え.example", ""]:
    add(f"host {host!r}", "host", Raw=host)

# --------------------------------------------------------------------------
# Settings validation, hashing and strict decoding
# --------------------------------------------------------------------------
add("settings demo", "settings", Settings=DEMO["settings"])
add("settings edge", "settings", Settings=base)
for field, value in [("schemaVersion", 2), ("revision", ""), ("revision", "r" * 101), ("defaultTarget", "x" * 301),
                     ("manual", []), ("namespaces", None), ("namespaces", {"": {"mode": "public"}}),
                     ("namespaces", {"a": {"mode": "secret"}}), ("namespaces", {"a": {"mode": "public", "users": [""]}}),
                     ("assignments", {"e": ""}), ("targets", [{"id": "domain-x", "name": "x", "host": "x.example:1"}]),
                     ("targets", [{"id": "node-x", "name": "x", "host": "x.example"}]),
                     ("manual", [{"id": "manual-a", "name": "A", "urls": []}]),
                     ("manual", [{"id": "manual-a", "name": "A", "urls": [{"id": "u", "label": "", "url": "http:/x"}]}]),
                     ("apps", {"a": {"display": "maybe"}}), ("apps", {"a": {"icon": "ftp://x"}}),
                     ("apps", {"a": {"endpoints": {"e": {"scheme": "ftp"}}}}),
                     ("apps", {"a": {"endpoints": {"e": {"url": "https://x.example/ok"}}}}),
                     ("apps", {"x" * 101: {}})]:
    st = copy.deepcopy(DEMO["settings"])
    st[field] = value
    add(f"settings {field}={str(value)[:40]}", "settings", Settings=st)
st = copy.deepcopy(DEMO["settings"])
st["apps"]["escape"] = {"name": "한글 <>&     \x08\x0c\x1f\x7f", "hidden": False}
add("settings escaping", "settings", Settings=st)

decode_cases = {
    "decode: minimal": '{"schemaVersion":1,"revision":"r"}',
    "decode: explicit nulls": '{"schemaVersion":1,"revision":"r","namespaces":null,"apps":{"a":null,"b":{"name":null,"hidden":null,'
                              '"reviewed":{"e":null},"endpoints":{"e":null}}},"manual":null,"targets":null,"defaultTarget":null,"assignments":null}',
    "decode: null policy in map": '{"schemaVersion":1,"revision":"r","namespaces":{"a":null}}',
    "decode: null group element": '{"schemaVersion":1,"revision":"r","namespaces":{"a":{"mode":"public","groups":[null]}}}',
    "decode: null assignment": '{"schemaVersion":1,"revision":"r","assignments":{"e":null}}',
    "decode: null visibility pointer": '{"schemaVersion":1,"revision":"r","apps":{"a":{"visibility":null}}}',
    "decode: top-level null": "null",
    "decode: unknown top-level field": '{"schemaVersion":1,"revision":"r","extra":1}',
    "decode: unknown nested field": '{"schemaVersion":1,"revision":"r","apps":{"a":{"bogus":true}}}',
    "decode: unknown policy field": '{"schemaVersion":1,"revision":"r","namespaces":{"a":{"mode":"public","x":1}}}',
    "decode: wrong type": '{"schemaVersion":"1","revision":"r"}',
    "decode: float version": '{"schemaVersion":1.0,"revision":"r"}',
    "decode: array instead of object": "[]",
    "decode: empty": "",
    "decode: trailing data after the JSON document": '{"schemaVersion":1,"revision":"r"} {}',
    "decode: field name in different letter case": '{"SchemaVersion":1,"revision":"r"}',
    "decode: duplicate JSON key": '{"schemaVersion":1,"revision":"a","revision":"b"}',
    "decode: whitespace around": ' \n{"schemaVersion":1,"revision":"r"}\n ',
}
for label, raw in decode_cases.items():
    add(label, "decode-settings", Raw=raw)

# --------------------------------------------------------------------------
# Git ConfigMap manifests (read through the original Git code path)
# --------------------------------------------------------------------------
def settings_yaml(body, extra_meta="", top="", data_key="settings.json"):
    lines = "\n".join("    " + line for line in body.splitlines())
    return (f"apiVersion: v1\ndata:\n  {data_key}: |\n{lines}\n{top}kind: ConfigMap\nmetadata:\n{extra_meta}"
            "  name: kubeshelf-settings\n  namespace: public-services\n")


minimal = json.dumps({"schemaVersion": 1, "revision": "git"}, indent=2)
read_settings = {
    "read-settings: minimal": settings_yaml(minimal),
    "read-settings: creationTimestamp null and labels": settings_yaml(minimal, "  creationTimestamp: null\n  labels:\n    a: b\n"),
    "read-settings: nested unknown metadata field": settings_yaml(minimal, "  bogus: 1\n"),
    "read-settings: unknown top-level field": settings_yaml(minimal, top="spec: {}\n"),
    "read-settings: duplicate key": settings_yaml(minimal, "  name: other\n"),
    "read-settings: wrong name": settings_yaml(minimal).replace("name: kubeshelf-settings", "name: other"),
    "read-settings: wrong namespace": settings_yaml(minimal).replace("namespace: public-services", "namespace: other"),
    "read-settings: wrong kind": settings_yaml(minimal).replace("kind: ConfigMap", "kind: Secret"),
    "read-settings: invalid settings json": settings_yaml('{"schemaVersion":2,"revision":"x"}'),
    "read-settings: settings json unknown field": settings_yaml('{"schemaVersion":1,"revision":"x","y":1}'),
    "read-settings: missing data key": settings_yaml(minimal, data_key="other.json"),
    "read-settings: immutable and binaryData": settings_yaml(minimal, top="binaryData:\n  x: AA==\nimmutable: true\n"),
    "read-settings: ownerReferences unknown subfield": settings_yaml(minimal, "  ownerReferences:\n  - apiVersion: v1\n    kind: X\n    name: y\n    uid: z\n    bogus: 1\n"),
    "read-settings: not yaml": "{{{",
    "read-settings: YAML 1.1 boolean word as an extra data value": settings_yaml(minimal).replace("  settings.json: |\n", "  other: yes\n  settings.json: |\n"),
    "read-settings: number as an extra data value": settings_yaml(minimal).replace("  settings.json: |\n", "  other: 1\n  settings.json: |\n"),
}
for label, raw in read_settings.items():
    add(label, "read-settings", GitPath="runtime/settings.yaml", Raw=raw)


def favorites_yaml(subject, name, body, annotation=None, extra=""):
    ann = subject if annotation is None else annotation
    lines = "\n".join("    " + line for line in body.splitlines())
    return (f"apiVersion: v1\ndata:\n  favorites.json: |\n{lines}\n{extra}kind: ConfigMap\nmetadata:\n  annotations:\n"
            f"    kubeshelf.io/subject: {json.dumps(ann)}\n  name: {name}\n  namespace: public-services\n")


import hashlib  # noqa: E402

alice = {"subject": "alice", "username": "alice", "groups": []}
alice_name = "kubeshelf-user-" + hashlib.sha256(b"alice").hexdigest()[:40]
fav = json.dumps({"defaultEndpoints": {}, "collections": [{"id": "favorites", "name": "즐겨찾기", "cards": ["b", "a"]}],
                  "startCollection": ""}, indent=2)
read_favorites = {
    "read-favorites: valid": favorites_yaml("alice", alice_name, fav),
    "read-favorites: other subject annotation": favorites_yaml("alice", alice_name, fav, annotation="bob"),
    "read-favorites: other name": favorites_yaml("alice", "kubeshelf-user-0", fav),
    "read-favorites: extra data key": favorites_yaml("alice", alice_name, fav, extra="  other: x\n"),
    "read-favorites: unknown json field": favorites_yaml("alice", alice_name, '{"collections":[],"x":1}'),
    "read-favorites: trailing json": favorites_yaml("alice", alice_name, '{"collections":[]} {}'),
    "read-favorites: invalid collection": favorites_yaml("alice", alice_name, '{"collections":[{"id":"bad id","name":"x"}]}'),
    "read-favorites: nulls": favorites_yaml("alice", alice_name, '{"defaultEndpoints":null,"collections":null,"startCollection":null}'),
}
for label, raw in read_favorites.items():
    add(label, "read-favorites", GitPath="runtime/settings.yaml", Identity=alice, Raw=raw)

# --------------------------------------------------------------------------
# Manifest encoding: byte-identical YAML for ordinary values
# --------------------------------------------------------------------------
add("encode-settings demo", "encode-settings", Settings=DEMO["settings"])
add("encode-settings edge", "encode-settings", Settings=base)
plain_subjects = ["alice", "2f1c9a0e-5b7d-4c11-9a8e-0123456789ab", "12345678-1234-1234-1234-123456789abc",
                  "a3f1" * 16, "user.name_1"]
odd_subjects = ["on", "Yes", "12345", "1e10", "0x1F", "2024-01-02", "a b", "한글", "x:y", "-dash", "", "quote\"back\\slash"]
profile = {"defaultTarget": "domain-public", "defaultEndpoints": {"b": "y", "a": "x"},
           "collections": [{"id": "daily", "name": "일상 <&>", "cards": ["c"]}], "startCollection": "daily"}
for subject in plain_subjects:
    add(f"encode-favorites {subject!r}", "encode-favorites", Identity={"subject": subject}, Profile=profile)
for subject in plain_subjects + odd_subjects:
    add(f"favorite-path {subject!r}", "favorite-path", GitPath="runtime/settings.yaml", Identity={"subject": subject})
for path in ["settings.yaml", "a/b/c.yaml"]:
    add(f"favorite-path dir {path}", "favorite-path", GitPath=path, Identity={"subject": "alice"})

# --------------------------------------------------------------------------
# Favourites, routes, summaries, config and store validation
# --------------------------------------------------------------------------
for who, identity in identities.items():
    add(f"default-favorites {who}", "default-favorites", Identity=identity)
    for n, prof in enumerate([
        {},
        {"collections": [{"id": "favorites", "name": "즐겨찾기", "cards": ["a", "b"]}], "startCollection": "favorites"},
        {"collections": [{"id": "a", "name": "  한글 <>&  ", "cards": []}], "defaultEndpoints": {"b": "y", "a": "x"}},
        {"collections": [{"id": "x", "name": "A", "cards": ["a", "a"]}]},
        {"collections": [{"id": "x", "name": "A", "cards": []}, {"id": "y", "name": "a", "cards": []}]},
        {"collections": [{"id": "x", "name": "ΑΣ", "cards": []}, {"id": "y", "name": "ασ", "cards": []}]},
        {"collections": [{"id": "x", "name": "İ", "cards": []}, {"id": "y", "name": "i", "cards": []}]},
        {"collections": [{"id": "x", "name": "a\u0007b", "cards": []}]},
        {"collections": [{"id": "x", "name": "가" * 41, "cards": []}]},
        {"collections": [{"id": "x" * 81, "name": "long id", "cards": []}]},
        {"collections": [{"id": "a", "name": "A", "cards": ["z", "y"]}, {"id": "b", "name": "B", "cards": ["y"]}], "startCollection": "b"},
        {"collections": [{"id": "a", "name": "A"}], "startCollection": "missing"},
        {"defaultEndpoints": {"": "x"}},
        {"defaultTarget": "t" * 301},
    ]):
        add(f"favorites #{n} {who}", "favorites", Profile=prof, Identity=identity)

for route in ["/", "/favorites?collection=a", "/hidden", "/discovery", "/namespaces", "//evil.test", "https://evil.test",
              "/api/admin/state", "/favorites?collection=a&collection=b", "/favorites?other=a", "/favorites?collection=a%0a",
              "/favorites#x", "/favorites#", "/favorites?", "/%66avorites", "/favorites?collection=a;b", "/favorites?collection=%zz",
              "/favorites?collection=a+b", "/favorites?collection=ok_-1&", "/hidden?collection=a", "javascript:alert(1)",
              "/favorites/", "", "\\\\evil.test", "/favorites?collection=" + "x" * 81]:
    add(f"route {route!r}", "route", Route=route)

old = copy.deepcopy(base)
new = copy.deepcopy(base)
new["namespaces"]["apps"] = {"mode": "admin"}
new["namespaces"]["fresh"] = {"mode": "public"}
del new["nodePortNamespaces"]["ops"]
new["apps"]["group-shared"]["icon"] = "star"
new["apps"]["new-card"] = {"description": "d\nwith newline"}
new["manual"] = []
new["targets"] = []
new["assignments"] = {}
add("change-summary many", "change-summary", Old=old, Settings=new)
add("change-summary none", "change-summary", Old=old, Settings=old)
long_names = copy.deepcopy(base)
for n in range(40):
    long_names["apps"][f"long-{n}"] = {"name": "긴 이름 " * 8}
add("change-summary truncation", "change-summary", Old=base, Settings=long_names)
add("personal-summary all", "personal-summary", Before=profile,
    Profile={"defaultEndpoints": {}, "collections": [], "startCollection": ""})
add("personal-summary none", "personal-summary", Before=profile, Profile=profile)
legacy = copy.deepcopy(base)
legacy["apps"]["manual-docs"]["defaultEndpoint"] = "manual-docs-1"
add("legacy-defaults same", "legacy-defaults", Old=base, Settings=base)
add("legacy-defaults changed", "legacy-defaults", Old=base, Settings=legacy)
legacy_target = copy.deepcopy(base)
legacy_target["defaultTarget"] = ""
add("legacy-defaults target", "legacy-defaults", Old=base, Settings=legacy_target)

production = {"KUBESHELF_PUBLIC_URL": "https://shelf.example.com", "KUBESHELF_OIDC_ISSUER": "https://sso.example.com/app/",
              "KUBESHELF_OIDC_CLIENT_ID": "client", "KUBESHELF_ADMIN_SUBJECTS": " a , ,b ",
              "KUBESHELF_GIT_URL": "git@git.example.com:org/config.git"}
config_cases = {
    "config demo defaults": {"KUBESHELF_DEMO": "true"},
    "config demo invalid bool": {"KUBESHELF_DEMO": "TRUE"},
    "config production": production,
    "config production trailing slash": {**production, "KUBESHELF_PUBLIC_URL": "https://shelf.example.com/"},
    "config production path": {**production, "KUBESHELF_PUBLIC_URL": "https://shelf.example.com/app"},
    "config production query": {**production, "KUBESHELF_PUBLIC_URL": "https://shelf.example.com?x"},
    "config production force query": {**production, "KUBESHELF_PUBLIC_URL": "https://shelf.example.com?"},
    "config production http": {**production, "KUBESHELF_PUBLIC_URL": "http://shelf.example.com"},
    "config production no admins": {**production, "KUBESHELF_ADMIN_SUBJECTS": " , "},
    "config production no git": {**production, "KUBESHELF_GIT_URL": ""},
    "config missing public url": {},
    "config schemes": {**production, "KUBESHELF_INGRESS_SCHEMES": '{"traefik":"https","nginx":"http"}'},
    "config schemes null": {**production, "KUBESHELF_INGRESS_SCHEMES": "null"},
    "config schemes null value": {**production, "KUBESHELF_INGRESS_SCHEMES": '{"x":null}'},
    "config schemes invalid": {**production, "KUBESHELF_INGRESS_SCHEMES": '{"x":"ftp"}'},
    "config demo overrides demo-https": {"KUBESHELF_DEMO": "true", "KUBESHELF_INGRESS_SCHEMES": '{"demo-https":"http","a":"http"}'},
    "config custom paths": {**production, "KUBESHELF_LISTEN": "127.0.0.1:9000", "KUBESHELF_WORK_DIR": "/tmp/w",
                            "KUBESHELF_GIT_BRANCH": "config", "KUBESHELF_GIT_PATH": "a/b.yaml", "KUBESHELF_CLUSTER_NAME": "c"},
}
for label, env in config_cases.items():
    add(label, "config", Env=env)


def store_case(label, url="git@git.example.com:org/config.git", branch="main", path="runtime/settings.yaml"):
    add(label, "store-validation", GitURL=url, Branch=branch, GitPath=path)


store_case("store: valid scp-style URL")
store_case("store: valid ssh URL", url="ssh://git@git.example.com:2222/org/config.git")
store_case("store: https URL rejected", url="https://git.example.com/org/config.git")
store_case("store: file URL rejected", url="/srv/config.git")
store_case("store: ext transport rejected", url="ext::sh -c touch% /tmp/x")
store_case("store: branch with space", branch="a b")
store_case("store: branch with dots", branch="a..b")
store_case("store: branch leading dash", branch="-x")
store_case("store: branch nested ok", branch="release/1.0")
store_case("store: absolute manifest path", path="/etc/settings.yaml")
store_case("store: parent manifest path", path="../settings.yaml")
store_case("store: .git manifest path", path=".git/config")
store_case("store: manifest path with ./ prefix", path="./settings.yaml")
store_case("store: manifest path with .. in the middle", path="a/../settings.yaml")
store_case("store: manifest path with empty segment", path="a//settings.yaml")
store_case("store: manifest path with trailing slash", path="runtime/")
store_case("store: manifest path with nested .git directory", path="a/.git/settings.yaml")
STORE_MESSAGES = {"Git URL must use SSH", "invalid Git branch", "invalid Git manifest path"}


# --------------------------------------------------------------------------
# Runner
# --------------------------------------------------------------------------
def run(binary, case):
    env = {k: v for k, v in os.environ.items() if not k.startswith("KUBESHELF_") and k != "POD_NAMESPACE"}
    out = subprocess.run([binary], input=json.dumps(case).encode(), capture_output=True, env=env, timeout=60)
    if out.returncode != 0:
        raise RuntimeError(f"{binary} failed: {out.stderr.decode()[-2000:]}")
    return json.loads(out.stdout)


def normalise(case, value):
    if case["Op"] == "store-validation":
        return value if value in STORE_MESSAGES else ""
    if case["Op"] == "catalog":
        # Go sorts cards with an unstable sort over map iteration order, so
        # cards with equal lower-cased names have no defined order.
        value["cards"].sort(key=lambda c: (c["name"].lower(), c["id"], c["source"]))
    if case["Op"] == "config" and value.get("IngressSchemes", {}) is None:
        value["IngressSchemes"] = {}  # Go json.Unmarshal("null") leaves a nil map
    return value


def is_error(value):
    return isinstance(value, dict) and "error" in value and value.get("error") is not None and "yaml" not in value


def check(item):
    label, case = item
    go, rust = normalise(case, run(GO, case)), normalise(case, run(RUST, case))
    if label in RUST_STRICTER:
        if case["Op"] == "store-validation":
            return label, go == "" and rust in STORE_MESSAGES, go, rust
        return label, not is_error(go) and is_error(rust), go, rust
    if label in UTF8_TRUNCATION:
        return label, go.rstrip("\ufffd") == rust, go, rust
    if label in BOTH_REJECT:
        return label, is_error(go) and is_error(rust), go, rust
    return label, go == rust, go, rust


def cross_read(subject):
    """Rust-written and Go-written favourites must be read back identically by both."""
    identity = {"subject": subject}
    case = dict(Op="encode-favorites", Identity=identity, Profile=profile)
    go_yaml, rust_yaml = run(GO, case)["yaml"], run(RUST, case)["yaml"]
    results = []
    for reader in (GO, RUST):
        for writer, text in (("go", go_yaml), ("rust", rust_yaml)):
            results.append(run(reader, dict(Op="read-favorites", GitPath="runtime/settings.yaml", Identity=identity, Raw=text)))
    return subject, all(r == results[0] and not is_error(r) for r in results), results


def main():
    failures = []
    # The demo fixture must be exactly what the pinned original exports.
    exported = subprocess.run([GO, "demo"], capture_output=True, check=True).stdout
    if exported != (ROOT / "tests/demo.json").read_bytes():
        failures.append(dict(case="tests/demo.json differs from `go-reference demo`"))
    with concurrent.futures.ThreadPoolExecutor(max_workers=os.cpu_count() or 4) as pool:
        for label, ok, go, rust in pool.map(check, cases):
            if not ok:
                failures.append(dict(case=label, go=go, rust=rust))
        for subject, ok, results in pool.map(cross_read, odd_subjects + plain_subjects[:2]):
            if not ok:
                failures.append(dict(case=f"cross-read favorites {subject!r}", results=results))
    total = len(cases) + len(odd_subjects) + 2
    if failures:
        out = pathlib.Path(os.environ.get("KUBESHELF_MISMATCH_FILE", "/tmp/kubeshelf-mismatch.json"))
        out.write_text(json.dumps(failures, ensure_ascii=False, indent=2))
        print(f"FAIL: {len(failures)}/{total} differential cases differ; details in {out}")
        for f in failures[:15]:
            print("  -", f["case"])
        return 1
    ops = {}
    for _, case in cases:
        ops[case["Op"]] = ops.get(case["Op"], 0) + 1
    summary = ", ".join(f"{k} {v}" for k, v in sorted(ops.items()))
    print("PASS: tests/demo.json is byte-identical to the original's demo export")
    print(f"PASS: {total} Go/Rust differential cases ({summary}, cross-read {len(odd_subjects) + 2}); "
          f"{len(RUST_STRICTER)} documented Rust-stricter and {len(BOTH_REJECT)} both-reject cases verified")
    return 0


if __name__ == "__main__":
    sys.exit(main())
