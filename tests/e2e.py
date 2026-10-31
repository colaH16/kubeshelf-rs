#!/usr/bin/env python3
"""End-to-end checks against the real kubeshelf-rs binary (no external services).

Two runs:

* demo mode: static/SPA routes, security headers, roles, CSRF, favourites
  CAS, admin save with the simulated delayed apply, logout, SIGTERM.
* production mode, fully local: mock OIDC provider (RS256 tokens signed with
  the openssl CLI), mock Authentik-style directory, mock Kubernetes API
  (lists + ConfigMap watch), and a bare Git repository reached through a fake
  `ssh` on PATH, so the real SSH code path and options are exercised. Fleet is
  simulated by rewriting the mounted settings file.

Usage: python3 tests/e2e.py [path/to/kubeshelf-rs]
Needs: git, openssl, and target/debug/reference (cargo build --bin reference).
All addresses are loopback or documentation ranges.
"""
import base64
import hashlib
import http.client
import http.server
import json
import os
import pathlib
import queue
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
import urllib.parse

ROOT = pathlib.Path(__file__).resolve().parent.parent
BINARY = sys.argv[1] if len(sys.argv) > 1 else str(ROOT / "target/debug/kubeshelf-rs")
REFERENCE = os.environ.get("KUBESHELF_RUST_REFERENCE", str(ROOT / "target/debug/reference"))
STATIC = os.environ.get("KUBESHELF_E2E_STATIC", str(ROOT / "web/dist"))
DEMO = json.loads((ROOT / "tests/demo.json").read_text())
checks = 0


def check(condition, message):
    global checks
    if not condition:
        raise AssertionError(message)
    checks += 1


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class Client:
    """Minimal HTTP client with a cookie jar (no redirects followed)."""

    def __init__(self, port, origin):
        self.port, self.origin, self.cookies = port, origin, {}

    def request(self, method, path, body=None, csrf=None, origin=True, content_type="application/json", cookies=True):
        conn = http.client.HTTPConnection("127.0.0.1", self.port, timeout=30)
        headers = {}
        if cookies and self.cookies:
            headers["Cookie"] = "; ".join(f"{k}={v}" for k, v in self.cookies.items())
        if origin:
            headers["Origin"] = self.origin
        if csrf:
            headers["X-CSRF-Token"] = csrf
        data = None
        if body is not None:
            data = body if isinstance(body, (bytes, str)) else json.dumps(body)
            headers["Content-Type"] = content_type
        conn.request(method, path, body=data, headers=headers)
        r = conn.getresponse()
        raw = r.read()
        for header, value in r.getheaders():
            if header.lower() == "set-cookie":
                name, _, rest = value.partition("=")
                val = rest.split(";", 1)[0]
                if "Max-Age=0" in value:
                    self.cookies.pop(name, None)
                else:
                    self.cookies[name] = val
        conn.close()
        return r.status, {k.lower(): v for k, v in r.getheaders()}, raw

    def json(self, method, path, body=None, csrf=None, **kw):
        status, headers, raw = self.request(method, path, body, csrf, **kw)
        return status, (json.loads(raw) if raw and headers.get("content-type", "").startswith("application/json") else raw)


def start(env, log_path):
    log = open(log_path, "wb")
    proc = subprocess.Popen([BINARY], env=env, stdout=log, stderr=subprocess.STDOUT)
    return proc, log


def wait_ready(port, proc, timeout=60):
    deadline = time.time() + timeout
    while time.time() < deadline:
        if proc.poll() is not None:
            raise AssertionError(f"binary exited early with {proc.returncode}")
        try:
            conn = http.client.HTTPConnection("127.0.0.1", port, timeout=2)
            conn.request("GET", "/readyz")
            r = conn.getresponse()
            body = r.read()
            if r.status == 200:
                return body
        except OSError:
            pass
        time.sleep(0.2)
    raise AssertionError("binary did not become ready")


def stop(proc, log):
    started = time.time()
    proc.send_signal(signal.SIGTERM)
    try:
        code = proc.wait(timeout=15)
    finally:
        log.close()
    check(code == 0, f"graceful SIGTERM exit code {code}")
    check(time.time() - started < 12, "shutdown is bounded")


def until(predicate, message, timeout=15):
    deadline = time.time() + timeout
    while time.time() < deadline:
        if predicate():
            return
        time.sleep(0.25)
    raise AssertionError(message)


SECURITY_HEADERS = {"cache-control": "private, no-store, max-age=0", "x-frame-options": "DENY",
                    "x-content-type-options": "nosniff", "referrer-policy": "no-referrer", "vary": "Cookie"}


def common_checks(c):
    """Routes and authorisation shared by both modes."""
    status, headers, body = c.request("GET", "/healthz")
    check(status == 200 and body == b"ok\n", "healthz")
    for name, value in SECURITY_HEADERS.items():
        check(headers.get(name) == value, f"security header {name}")
    check("frame-ancestors 'none'" in headers.get("content-security-policy", ""), "CSP")
    index = (pathlib.Path(STATIC) / "index.html").read_bytes()
    for view in ["/", "/favorites", "/discovery", "/namespaces", "/hidden"]:
        status, headers, body = c.request("GET", view)
        check(status == 200 and body == index and headers["content-type"].startswith("text/html"), f"SPA view {view}")
    asset = next((pathlib.Path(STATIC) / "assets").glob("*.js")).name
    status, headers, body = c.request("GET", f"/assets/{asset}")
    check(status == 200 and "javascript" in headers["content-type"] and headers.get("cache-control", "").startswith("private"), "hashed asset")
    check(c.request("GET", "/favicon.svg")[0] == 200, "favicon")
    for missing in ["/nope", "/favorites/", "/assets/missing.js", "/../Cargo.toml", "/assets/../../Cargo.toml"]:
        check(c.request("GET", missing)[0] == 404, f"404 for {missing}")
    check(c.request("POST", "/healthz")[0] == 405, "method not allowed")
    status, body = c.json("GET", "/api/me/favorites")
    check(status == 401 and body["error"] == "로그인이 필요합니다", "anonymous favourites 401")
    status, body = c.json("GET", "/api/admin/state")
    check(status == 403, "anonymous admin 403")
    status, body = c.json("GET", "/api/catalog")
    check(status == 200 and body["identity"]["subject"] == "" and "csrf" not in body, "anonymous catalog")
    text = json.dumps(body)
    check("192.0.2." not in text, "anonymous catalog has no node addresses")
    return body


def exercise_favorites(c, csrf, owner_check=None):
    status, state = c.json("GET", "/api/me/favorites")
    check(status == 200 and state["version"], "get favourites")
    profile = state["profile"]
    profile["collections"][0]["cards"] = ["card-e2e"]
    check(c.json("PUT", "/api/me/favorites", {"version": state["version"], "profile": profile})[0] == 403, "favourites CSRF required")
    check(c.json("PUT", "/api/me/favorites", {"version": state["version"], "profile": profile}, csrf=csrf, origin=False)[0] == 403,
          "favourites Origin required")
    status, body = c.json("PUT", "/api/me/favorites", {"version": state["version"], "profile": profile, "x": 1}, csrf=csrf)
    check(status == 400, "unknown request field rejected")
    status, body = c.json("PUT", "/api/me/favorites", [state["version"]], csrf=csrf)
    check(status == 400, "positional JSON rejected")
    status, body = c.json("PUT", "/api/me/favorites", "{}", csrf=csrf, content_type="text/plain")
    check(status == 400 and body["error"] == "JSON request required", "content type required")
    status, saved = c.json("PUT", "/api/me/favorites", {"version": state["version"], "profile": profile}, csrf=csrf)
    check(status == 200 and saved["profile"]["collections"][0]["cards"] == ["card-e2e"], "save favourites")
    status, again = c.json("GET", "/api/me/favorites")
    check(again == saved, "favourites persisted")
    status, body = c.json("PUT", "/api/me/favorites", {"version": state["version"], "profile": profile}, csrf=csrf)
    check(status == 409, "stale favourites version is a conflict")
    bad = json.loads(json.dumps(saved["profile"]))
    bad["defaultTarget"] = "node-not-visible"
    status, body = c.json("PUT", "/api/me/favorites", {"version": saved["version"], "profile": bad}, csrf=csrf)
    check(status == 400, "invisible default target rejected")
    if owner_check:
        owner_check(saved)
    return saved


def browser_smoke(origin):
    """Render the SPA in headless Chromium/Chrome if one is installed."""
    import shutil
    browser = os.environ.get("KUBESHELF_E2E_BROWSER") or next(
        (b for b in ["google-chrome", "google-chrome-stable", "chromium", "chromium-browser"] if shutil.which(b)), None)
    if not browser:
        print("SKIP: browser smoke (no Chromium/Chrome found)")
        return
    with tempfile.TemporaryDirectory(prefix="kubeshelf-browser-") as profile:
        out = subprocess.run([browser, "--headless=new", "--no-sandbox", "--disable-gpu", f"--user-data-dir={profile}",
                              "--virtual-time-budget=5000", "--dump-dom", origin + "/"],
                             capture_output=True, text=True, timeout=120)
    if out.returncode != 0:
        # A browser that cannot start in this sandbox is an environment issue;
        # CI sets KUBESHELF_E2E_BROWSER_REQUIRED=1 where Chrome is known to work.
        check(os.environ.get("KUBESHELF_E2E_BROWSER_REQUIRED") != "1", f"browser exited with {out.returncode}")
        print(f"SKIP: browser smoke ({browser} exited with {out.returncode})")
        return
    dom = out.stdout
    # Anonymous users see the public demo services rendered by React.
    check("Nextcloud" in dom and "Immich" in dom, "browser renders public cards")
    check("Grafana" not in dom and "192.0.2." not in dom, "browser view hides admin-only data")
    print("PASS: browser smoke with " + browser)


# ---------------------------------------------------------------------------
# Demo mode
# ---------------------------------------------------------------------------
def demo_run(tmp):
    port = free_port()
    origin = f"http://127.0.0.1:{port}"
    env = {"PATH": os.environ["PATH"], "KUBESHELF_DEMO": "true", "KUBESHELF_LISTEN": f"127.0.0.1:{port}",
           "KUBESHELF_PUBLIC_URL": origin, "KUBESHELF_STATIC_DIR": STATIC, "RUST_LOG": "info"}
    proc, log = start(env, tmp / "demo.log")
    try:
        check(wait_ready(port, proc) == b"ready\n", "demo ready")
        anon = Client(port, origin)
        common_checks(anon)
        browser_smoke(origin)
        status, headers, body = anon.request("POST", "/auth/demo", {"role": "admin"}, origin=False)
        check(status == 403 and body == b"invalid origin\n", "demo login needs Origin")
        status, headers, body = anon.request("GET", "/auth/callback?state=x")
        check(status == 404, "callback disabled in demo")

        family = Client(port, origin)
        status, headers, _ = family.request("POST", "/auth/demo", {"role": "family"})
        check(status == 303 and headers["location"] == "/" and "kubeshelf-demo" in family.cookies, "demo family login")
        status, cat = family.json("GET", "/api/catalog")
        check(cat["identity"]["subject"] == "demo-family" and cat["csrf"], "family identity")
        check(any(c["namespace"] == "myapps" for c in cat["cards"]), "restricted namespace visible to its group")
        check(not any(c["namespace"] == "infra" for c in cat["cards"]), "admin namespace hidden from family")
        check(family.json("GET", "/api/admin/state")[0] == 403, "family is not admin")
        exercise_favorites(family, cat["csrf"])

        admin = Client(port, origin)
        status, headers, _ = admin.request("GET", "/auth/login?returnTo=/favorites%3Fcollection%3Ddaily")
        check(status == 303 and headers["location"] == "/favorites?collection=daily", "demo admin login keeps safe returnTo")
        status, headers, _ = Client(port, origin).request("GET", "/auth/login?returnTo=//evil.example")
        check(headers["location"] == "/", "unsafe returnTo rejected")
        status, cat = admin.json("GET", "/api/catalog")
        csrf = cat["csrf"]
        check(cat["identity"]["admin"] and cat["namespaces"], "admin catalog")
        status, state = admin.json("GET", "/api/admin/state")
        check(status == 200 and {"settings", "baseCommit", "status", "catalog", "directory"} <= set(state), "admin state")
        settings = state["settings"]
        settings["namespaces"]["infra"] = {"mode": "public"}
        status, saved = admin.json("PUT", "/api/admin/state", {"baseCommit": state["baseCommit"], "settings": settings}, csrf=csrf)
        check(status == 200 and saved["status"]["pending"], "demo save is pending")
        status, conflict = admin.json("PUT", "/api/admin/state", {"baseCommit": state["baseCommit"], "settings": settings}, csrf=csrf)
        check(status == 409, "stale admin base is a conflict")
        until(lambda: not admin.json("GET", "/api/admin/status")[1]["pending"], "demo apply completes")
        status, cat = anon.json("GET", "/api/catalog")
        check(any(c["namespace"] == "infra" for c in cat["cards"]), "applied settings change the public catalog")
        bad = json.loads(json.dumps(saved["settings"]))
        bad["defaultTarget"] = "node-x"
        status, body = admin.json("PUT", "/api/admin/state", {"baseCommit": saved["baseCommit"], "settings": bad}, csrf=csrf)
        check(status == 400, "legacy shared defaults are read-only")
        status, body = admin.json("POST", "/api/admin/tcp", {"endpointId": "x", "targetId": "domain-edge"}, csrf=csrf)
        check(status == 400, "TCP probe requires a TCP-enabled manual domain")

        check(admin.request("POST", "/auth/logout", origin=False)[0] == 403, "logout needs Origin")
        check(admin.request("POST", "/auth/logout", csrf="wrong")[0] == 403, "logout needs CSRF")
        check(admin.request("POST", "/auth/logout", csrf=csrf)[0] == 204, "logout")
        check(admin.json("GET", "/api/catalog")[1]["identity"]["subject"] == "", "session ended")
    finally:
        stop(proc, log)


# ---------------------------------------------------------------------------
# Production mode with local mocks
# ---------------------------------------------------------------------------
def b64url(data):
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode()


class Mocks:
    def __init__(self, tmp):
        self.tmp = tmp
        self.key = tmp / "oidc.pem"
        subprocess.run(["openssl", "genrsa", "-traditional", "-out", str(self.key), "2048"], check=True, capture_output=True)
        modulus = subprocess.run(["openssl", "rsa", "-in", str(self.key), "-noout", "-modulus"], check=True,
                                 capture_output=True, text=True).stdout.strip().split("=", 1)[1]
        self.jwk = {"kty": "RSA", "kid": "k1", "use": "sig", "alg": "RS256", "n": b64url(bytes.fromhex(modulus)), "e": "AQAB"}
        self.codes = {}
        self.users = [
            {"uuid": "admin-uuid", "username": "admin", "name": "Admin", "is_active": True, "type": "internal", "groups_obj": [{"name": "operators"}]},
            {"uuid": "alice-uuid", "username": "alice", "name": "Alice", "is_active": True, "type": "internal", "groups_obj": [{"name": "family"}]},
            {"uuid": "bob-uuid", "username": "bob", "name": "Bob", "is_active": False, "type": "internal", "groups_obj": []},
            {"uuid": "robot", "username": "robot", "is_active": True, "type": "service_account", "groups_obj": []},
        ]
        self.watch_events = queue.Queue()
        self.port = free_port()
        self.issuer = f"http://127.0.0.1:{self.port}"
        mocks = self

        class Handler(http.server.BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def log_message(self, *args):
                pass

            def send_json(self, value, status=200):
                body = json.dumps(value).encode()
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def do_GET(self):
                url = urllib.parse.urlparse(self.path)
                q = urllib.parse.parse_qs(url.query)
                kinds = {"/api/v1/namespaces": "Namespaces", "/api/v1/services": "Services", "/api/v1/pods": "Pods",
                         "/api/v1/nodes": "Nodes", "/apis/discovery.k8s.io/v1/endpointslices": "Slices",
                         "/apis/networking.k8s.io/v1/ingresses": "Ingresses"}
                if url.path == "/.well-known/openid-configuration":
                    return self.send_json({"issuer": mocks.issuer, "authorization_endpoint": mocks.issuer + "/authorize",
                                           "token_endpoint": mocks.issuer + "/token", "jwks_uri": mocks.issuer + "/jwks",
                                           "response_types_supported": ["code"], "subject_types_supported": ["public"],
                                           "id_token_signing_alg_values_supported": ["RS256"]})
                if url.path == "/jwks":
                    return self.send_json({"keys": [mocks.jwk]})
                if url.path.startswith("/api/v3/core/"):
                    if self.headers.get("Authorization") != "Bearer directory-token":
                        return self.send_json({"detail": "denied"}, 403)
                    if url.path == "/api/v3/core/users/":
                        return self.send_json({"pagination": {"next": 0}, "results": mocks.users})
                    return self.send_json({"pagination": {"next": 0}, "results": [{"name": "family"}, {"name": "empty"}]})
                if url.path in kinds:
                    check(q.get("resourceVersion") == ["0"], "lists use resourceVersion=0")
                    return self.send_json({"apiVersion": "v1", "kind": "List", "metadata": {"resourceVersion": "10"},
                                           "items": DEMO["snapshot"][kinds[url.path]]})
                if url.path == "/api/v1/namespaces/public-services/configmaps":
                    check(q.get("labelSelector") == ["kubeshelf.io/user-settings=true"], "ConfigMap watch is label-scoped")
                    if q.get("watch") not in (["true"], ["1"]):
                        return self.send_json({"apiVersion": "v1", "kind": "ConfigMapList", "metadata": {"resourceVersion": "10"}, "items": []})
                    self.send_response(200)
                    self.send_header("Content-Type", "application/json")
                    self.send_header("Transfer-Encoding", "chunked")
                    self.end_headers()
                    deadline = time.time() + 20
                    try:
                        while time.time() < deadline:
                            try:
                                event = mocks.watch_events.get(timeout=0.5)
                            except queue.Empty:
                                continue
                            line = (json.dumps(event) + "\n").encode()
                            self.wfile.write(f"{len(line):x}\r\n".encode() + line + b"\r\n")
                            self.wfile.flush()
                        self.wfile.write(b"0\r\n\r\n")
                    except OSError:
                        pass
                    return
                self.send_json({"error": "not found " + url.path}, 404)

            def do_POST(self):
                body = self.rfile.read(int(self.headers.get("Content-Length", 0))).decode()
                if self.path != "/token":
                    return self.send_json({}, 404)
                form = urllib.parse.parse_qs(body)
                expected = "Basic " + base64.b64encode(b"kubeshelf:client-secret").decode()
                pending = mocks.codes.pop(form.get("code", [""])[0], None)
                verifier = form.get("code_verifier", [""])[0]
                challenge = b64url(hashlib.sha256(verifier.encode()).digest())
                if self.headers.get("Authorization") != expected or not pending or challenge != pending["challenge"]:
                    return self.send_json({"error": "invalid_grant"}, 400)
                now = int(time.time())
                claims = {"iss": mocks.issuer, "aud": "kubeshelf", "sub": pending["subject"], "iat": now, "exp": now + 300,
                          "nonce": pending["nonce"]}
                return self.send_json({"access_token": "access", "token_type": "Bearer", "expires_in": 300,
                                       "id_token": mocks.sign(claims)})

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", self.port), Handler)
        self.server.daemon_threads = True
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def sign(self, claims):
        header = b64url(json.dumps({"alg": "RS256", "kid": "k1", "typ": "JWT"}).encode())
        payload = b64url(json.dumps(claims).encode())
        signing_input = f"{header}.{payload}".encode()
        signature = subprocess.run(["openssl", "dgst", "-sha256", "-sign", str(self.key)], input=signing_input,
                                   check=True, capture_output=True).stdout
        return f"{header}.{payload}.{b64url(signature)}"


def git(*args, cwd=None):
    out = subprocess.run(["git", "-c", "user.name=E2E", "-c", "user.email=e2e@example.invalid", "-c", "commit.gpgsign=false",
                          *args], cwd=cwd, check=True, capture_output=True, text=True)
    return out.stdout.strip()


def reference(case):
    return json.loads(subprocess.run([REFERENCE], input=json.dumps(case).encode(), check=True, capture_output=True).stdout)


def oidc_login(c, mocks, subject, return_to="/favorites"):
    status, headers, _ = c.request("GET", "/auth/login?returnTo=" + urllib.parse.quote(return_to))
    check(status == 302, "login redirects to the provider")
    location = urllib.parse.urlparse(headers["location"])
    q = dict(urllib.parse.parse_qsl(location.query))
    check(location.geturl().startswith(mocks.issuer + "/authorize"), "authorization endpoint")
    check(q["code_challenge_method"] == "S256" and q["scope"] == "openid profile email" and q["client_id"] == "kubeshelf",
          "PKCE and scopes")
    check(q["redirect_uri"] == "https://shelf.example.test/auth/callback", "redirect URI from public URL")
    code = "code-" + subject
    mocks.codes[code] = {"subject": subject, "nonce": q["nonce"], "challenge": q["code_challenge"]}
    status, headers, body = c.request("GET", f"/auth/callback?state={q['state']}&code={code}")
    return status, headers, body


def production_run(tmp):
    mocks = Mocks(tmp)
    # Git remote with seeded shared settings.
    remote = tmp / "remote.git"
    seed = tmp / "seed"
    git("init", "--bare", "--initial-branch=main", str(remote))
    git("init", "--initial-branch=main", str(seed))
    settings = json.loads(json.dumps(DEMO["settings"]))
    (seed / "runtime").mkdir()
    (seed / "runtime/settings.yaml").write_text(reference(dict(Op="encode-settings", Settings=settings))["yaml"])
    git("add", "-A", cwd=seed)
    git("commit", "-m", "seed", cwd=seed)
    git("push", str(remote), "main", cwd=seed)
    mounted = tmp / "config/settings.json"
    mounted.parent.mkdir()
    mounted.write_text(json.dumps(settings))

    # Credentials (synthetic) and a fake ssh that runs the Git service locally.
    creds = tmp / "credentials"
    creds.mkdir()
    (creds / "ssh-privatekey").write_text("synthetic key material\n")
    os.chmod(creds / "ssh-privatekey", 0o440)
    (creds / "known_hosts").write_text("git.example.invalid ssh-ed25519 AAAA\n")
    (creds / "client-secret").write_text("client-secret\n")
    (creds / "directory-token").write_text("directory-token\n")
    fakebin = tmp / "bin"
    fakebin.mkdir()
    ssh_log = tmp / "ssh.log"
    (fakebin / "ssh").write_text(f"""#!/bin/sh
echo "$*" >> {ssh_log}
while [ $# -gt 1 ]; do
  if [ "$1" = "-i" ]; then stat -c 'key-mode=%a' "$2" >> {ssh_log}; fi
  shift
done
exec sh -c "$1"
""")
    os.chmod(fakebin / "ssh", 0o755)
    kubeconfig = tmp / "kubeconfig"
    kubeconfig.write_text(json.dumps({
        "apiVersion": "v1", "kind": "Config", "current-context": "mock",
        "clusters": [{"name": "mock", "cluster": {"server": mocks.issuer}}],
        "users": [{"name": "mock", "user": {}}],
        "contexts": [{"name": "mock", "context": {"cluster": "mock", "user": "mock"}}]}))
    work = tmp / "work"
    work.mkdir()
    port = free_port()
    origin = "https://shelf.example.test"
    env = {
        "PATH": f"{fakebin}:{os.environ['PATH']}", "HOME": str(tmp), "RUST_LOG": "info",
        "KUBESHELF_LISTEN": f"127.0.0.1:{port}", "KUBESHELF_PUBLIC_URL": origin, "KUBESHELF_STATIC_DIR": STATIC,
        "KUBESHELF_KUBECONFIG": str(kubeconfig), "KUBESHELF_SETTINGS_FILE": str(mounted), "KUBESHELF_WORK_DIR": str(work),
        "KUBESHELF_GIT_URL": f"git@git.example.invalid:{remote}", "KUBESHELF_SSH_KEY_FILE": str(creds / "ssh-privatekey"),
        "KUBESHELF_KNOWN_HOSTS_FILE": str(creds / "known_hosts"), "KUBESHELF_OIDC_ISSUER": mocks.issuer,
        "KUBESHELF_OIDC_CLIENT_ID": "kubeshelf", "KUBESHELF_OIDC_SECRET_FILE": str(creds / "client-secret"),
        "KUBESHELF_DIRECTORY_URL": mocks.issuer, "KUBESHELF_DIRECTORY_TOKEN_FILE": str(creds / "directory-token"),
        "KUBESHELF_ADMIN_SUBJECTS": "admin-uuid", "POD_NAMESPACE": "public-services",
    }

    # Arguments are refused; configuration is environment-only.
    refused = subprocess.run([BINARY, "--help"], env=env, capture_output=True, timeout=20)
    check(refused.returncode == 1 and b"environment variables only" in refused.stdout + refused.stderr, "CLI arguments refused")
    # Bounded failing startup: an unreachable repository stops the process with a clear error.
    broken = dict(env, KUBESHELF_GIT_URL=f"git@git.example.invalid:{tmp}/missing.git", KUBESHELF_WORK_DIR=str(tmp / "work-broken"))
    (tmp / "work-broken").mkdir()
    started = time.time()
    failed = subprocess.run([BINARY], env=broken, capture_output=True, timeout=90)
    output = (failed.stdout + failed.stderr).decode()
    check(failed.returncode == 1 and "Git fetch" in output and time.time() - started < 65, "unreachable Git fails startup")
    check("client-secret" not in output and "directory-token" not in output, "startup errors contain no secret values")

    proc, log = start(env, tmp / "production.log")
    try:
        check(wait_ready(port, proc) == b"ready\n", "production ready")
        ssh_text = ssh_log.read_text()
        check("StrictHostKeyChecking=yes" in ssh_text and f"UserKnownHostsFile={creds / 'known_hosts'}" in ssh_text
              and "IdentitiesOnly=yes" in ssh_text and "-F /dev/null" in ssh_text, "SSH options")
        check("key-mode=600" in ssh_text, "SSH key copied with mode 0600")

        # Personal files for active, non-service accounts only.
        def user_file(subject):
            return "runtime/users/kubeshelf-user-" + hashlib.sha256(subject.encode()).hexdigest()[:40] + ".yaml"

        def remote_has(path):
            return subprocess.run(["git", "--git-dir", str(remote), "cat-file", "-e", f"main:{path}"], capture_output=True).returncode == 0

        until(lambda: remote_has(user_file("alice-uuid")) and remote_has(user_file("admin-uuid")), "accounts provisioned")
        check(not remote_has(user_file("bob-uuid")) and not remote_has(user_file("robot")), "inactive/service accounts skipped")

        anon = Client(port, origin)
        public = common_checks(anon)
        check(public["connected"] and not any(c["namespace"] == "myapps" for c in public["cards"]), "restricted cards hidden")

        # Callback errors.
        status, _, body = anon.request("GET", "/auth/callback?state=forged&code=x")
        check(status == 400 and body.decode() == "로그인 요청이 일치하지 않습니다\n", "callback state must match cookie")
        intruder = Client(port, origin)
        status, headers, body = oidc_login(intruder, mocks, "bob-uuid")
        check(status == 403 and "활성화된 사용자만" in body.decode(), "inactive account cannot log in")
        check("__Host-kubeshelf-login" not in intruder.cookies, "login cookie cleared after callback")

        alice = Client(port, origin)
        status, headers, _ = oidc_login(alice, mocks, "alice-uuid")
        check(status == 303 and headers["location"] == "/favorites" and "__Host-kubeshelf" in alice.cookies, "OIDC login")
        status, cat = alice.json("GET", "/api/catalog")
        check(cat["identity"]["username"] == "alice" and cat["identity"]["groups"] == ["family"], "identity from directory")
        check(any(c["namespace"] == "myapps" for c in cat["cards"]), "group policy applies")
        check(alice.json("GET", "/api/admin/state")[0] == 403, "non-admin denied")

        def favourites_in_git(saved):
            text = git("--git-dir", str(remote), "show", f"main:{user_file('alice-uuid')}")
            check("card-e2e" in text and "kubeshelf.io/subject: alice-uuid" in text, "favourites committed to the user's file")
            message = git("--git-dir", str(remote), "log", "-1", "--format=%B", "main")
            check(message.startswith("Update personal settings") and "Edited by: alice" in message, "personal commit message")

        exercise_favorites(alice, cat["csrf"], favourites_in_git)

        admin = Client(port, origin)
        status, headers, _ = oidc_login(admin, mocks, "admin-uuid", "/namespaces")
        check(status == 303 and headers["location"] == "/namespaces", "admin login")
        status, cat = admin.json("GET", "/api/catalog")
        csrf = cat["csrf"]
        check(cat["identity"]["admin"] and "192.0.2." in json.dumps(cat), "admin sees node targets")
        status, state = admin.json("GET", "/api/admin/state")
        check(status == 200 and [u["username"] for u in state["directory"]["users"]] == ["admin", "alice"], "directory in admin state")
        before_revision = state["status"]["appliedRevision"]
        new_settings = state["settings"]
        new_settings["namespaces"]["myapps"] = {"mode": "public"}
        status, saved = admin.json("PUT", "/api/admin/state", {"baseCommit": state["baseCommit"], "settings": new_settings}, csrf=csrf)
        check(status == 200 and saved["status"]["pending"] and saved["status"]["appliedRevision"] == before_revision,
              "production save is pending")
        check(git("--git-dir", str(remote), "rev-parse", "main") == saved["status"]["commit"], "save pushed to Git")
        message = git("--git-dir", str(remote), "log", "-1", "--format=%B", "main")
        check("namespace myapps Ingress visibility" in message and "Edited by: admin" in message, "shared commit message")
        status, cat = anon.json("GET", "/api/catalog")
        check(not any(c["namespace"] == "myapps" for c in cat["cards"]), "push does not change applied settings")
        status, body = admin.json("PUT", "/api/admin/state", {"baseCommit": state["baseCommit"], "settings": new_settings}, csrf=csrf)
        check(status == 409, "stale base commit conflicts")

        # Simulate Fleet projecting the new ConfigMap.
        mounted.write_text(json.dumps(saved["settings"]))
        until(lambda: not admin.json("GET", "/api/admin/status")[1]["pending"], "projected settings applied")
        status, cat = anon.json("GET", "/api/catalog")
        check(any(c["namespace"] == "myapps" for c in cat["cards"]), "applied settings take effect")
        # An invalid projection keeps the last good settings and reports an error.
        mounted.write_text("{not json")
        until(lambda: admin.json("GET", "/api/admin/status")[1].get("error"), "invalid projection reported")
        check(anon.request("GET", "/readyz")[0] == 200, "still ready with last good settings")
        status, cat = anon.json("GET", "/api/catalog")
        check(any(c["namespace"] == "myapps" for c in cat["cards"]), "last good settings kept")
        mounted.write_text(json.dumps(saved["settings"]))
        until(lambda: "error" not in admin.json("GET", "/api/admin/status")[1], "error clears")

        # Another writer commits; a user ConfigMap watch event refreshes the Git snapshot.
        clone = tmp / "other"
        git("clone", str(remote), str(clone))
        (clone / "unrelated.txt").write_text("external\n")
        git("add", "unrelated.txt", cwd=clone)
        git("commit", "-m", "external", cwd=clone)
        git("push", "origin", "main", cwd=clone)
        head = git("--git-dir", str(remote), "rev-parse", "main")
        mocks.watch_events.put({"type": "ADDED", "object": {"apiVersion": "v1", "kind": "ConfigMap", "metadata": {
            "name": "kubeshelf-user-x", "namespace": "public-services", "resourceVersion": "11",
            "labels": {"kubeshelf.io/user-settings": "true"}}}})
        until(lambda: admin.json("GET", "/api/admin/status")[1]["commit"] == head, "watch event refreshes Git snapshot", timeout=25)
        # The unrelated commit does not invalidate the shared settings CAS token.
        status, state = admin.json("GET", "/api/admin/state")
        check(state["baseCommit"] == saved["baseCommit"], "CAS token is scoped to the settings file")

        check(oct((work / "git-identity").stat().st_mode & 0o777) == "0o600", "private key copy mode")
        check(admin.request("POST", "/auth/logout", csrf=csrf)[0] == 204, "logout")
    finally:
        stop(proc, log)
        mocks.server.shutdown()


def main():
    with tempfile.TemporaryDirectory(prefix="kubeshelf-e2e-") as tmp:
        tmp = pathlib.Path(tmp)
        try:
            (tmp / "demo").mkdir()
            demo_run(tmp / "demo")
            (tmp / "prod").mkdir()
            production_run(tmp / "prod")
        except Exception:
            for log in sorted(tmp.rglob("*.log")):
                print(f"--- {log.name} (last 40 lines)")
                print("\n".join(log.read_text(errors="replace").splitlines()[-40:]))
            raise
    print(f"PASS: {checks} end-to-end checks (demo + local production mocks)")


if __name__ == "__main__":
    main()
