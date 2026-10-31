// Portions derived from Go net/url. Copyright 2009 The Go Authors.
// BSD-style license: see licenses/Go-BSD-3-Clause.txt.

//! A small, faithful port of the parts of Go 1.27.1 `net/url` that KubeShelf uses.
//!
//! Why not the `url` crate? It implements the WHATWG URL standard, which is
//! deliberately lenient: it trims spaces, accepts `http:/host` and
//! `http:///host`, normalises ports and paths, and so on. The original Go
//! application validates and re-renders URLs with `net/url`, and those exact
//! strings are stored in settings and shown to users. Re-implementing the
//! Go rules keeps validation and rendering byte-for-byte compatible
//! (checked by `tests/compare.py` against the original binary).
//!
//! Go strings are byte strings, so paths and fragments are kept as `Vec<u8>`
//! after unescaping; rendering escapes them back to ASCII.

use std::net::Ipv6Addr;

/// `url.Parse` failed (Go would return a `*url.Error`; callers only need
/// to know that the input is not a valid URL).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParseError;
impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("invalid URL")
    }
}
impl std::error::Error for ParseError {}

/// The URL components that `url.Parse` produces (opaque/userinfo details are
/// reduced to what KubeShelf needs).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GoUrl {
    pub scheme: String,
    pub opaque: String,
    /// Go keeps the parsed `*Userinfo`; KubeShelf only needs to know whether it
    /// was present (any userinfo is rejected) and how to render it again.
    pub user: Option<String>,
    pub host: Vec<u8>,
    pub path: Vec<u8>,
    raw_path: String,
    omit_host: bool,
    pub force_query: bool,
    pub raw_query: String,
    pub fragment: Vec<u8>,
    raw_fragment: String,
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Path,
    Host,
    Zone,
    UserPassword,
    Fragment,
}

/// Go `shouldEscape`: whether byte `c` must be percent-encoded in `mode`.
fn should_escape(c: u8, mode: Mode) -> bool {
    if c.is_ascii_alphanumeric() {
        return false;
    }
    if matches!(mode, Mode::Host | Mode::Zone)
        && matches!(
            c,
            b'!' | b'$'
                | b'&'
                | b'\''
                | b'('
                | b')'
                | b'*'
                | b'+'
                | b','
                | b';'
                | b'='
                | b':'
                | b'['
                | b']'
                | b'<'
                | b'>'
                | b'"'
        )
    {
        return false;
    }
    match c {
        b'-' | b'_' | b'.' | b'~' => return false,
        b'$' | b'&' | b'+' | b',' | b'/' | b':' | b';' | b'=' | b'?' | b'@' => {
            return match mode {
                Mode::Path => c == b'?',
                Mode::UserPassword => matches!(c, b'@' | b'/' | b'?' | b':'),
                Mode::Fragment => false,
                // Host/zone fall through to "escape everything else" in Go.
                Mode::Host | Mode::Zone => true,
            };
        }
        _ => {}
    }
    if mode == Mode::Fragment && matches!(c, b'!' | b'(' | b')' | b'*') {
        return false;
    }
    true
}

fn unhex(c: u8) -> u8 {
    match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        b'A'..=b'F' => c - b'A' + 10,
        _ => 0,
    }
}

/// Go `unescape` for the modes used here (never query-component mode, so a
/// `+` is always literal).
fn unescape(s: &[u8], mode: Mode) -> Result<Vec<u8>, ParseError> {
    let mut i = 0;
    while i < s.len() {
        match s[i] {
            b'%' => {
                if i + 2 >= s.len()
                    || !s[i + 1].is_ascii_hexdigit()
                    || !s[i + 2].is_ascii_hexdigit()
                {
                    return Err(ParseError);
                }
                // Hosts may only percent-encode non-ASCII bytes (and %25 for zones).
                if mode == Mode::Host && unhex(s[i + 1]) < 8 && &s[i..i + 3] != b"%25" {
                    return Err(ParseError);
                }
                if mode == Mode::Zone {
                    let v = (unhex(s[i + 1]) << 4) | unhex(s[i + 2]);
                    if &s[i..i + 3] != b"%25" && v != b' ' && should_escape(v, Mode::Host) {
                        return Err(ParseError);
                    }
                }
                i += 3;
            }
            c => {
                if matches!(mode, Mode::Host | Mode::Zone) && c < 0x80 && should_escape(c, mode) {
                    return Err(ParseError);
                }
                i += 1;
            }
        }
    }
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'%' {
            out.push((unhex(s[i + 1]) << 4) | unhex(s[i + 2]));
            i += 3;
        } else {
            out.push(s[i]);
            i += 1;
        }
    }
    Ok(out)
}

/// Go `escape`: percent-encode with upper-case hex digits.
fn escape(s: &[u8], mode: Mode) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(s.len());
    for &c in s {
        if should_escape(c, mode) {
            out.push('%');
            out.push(HEX[(c >> 4) as usize] as char);
            out.push(HEX[(c & 15) as usize] as char);
        } else {
            out.push(c as char);
        }
    }
    out
}

/// Go `validEncoded`: true if `s` needs no further escaping in `mode`.
fn valid_encoded(s: &str, mode: Mode) -> bool {
    s.bytes().all(|c| {
        matches!(
            c,
            b'!' | b'$'
                | b'&'
                | b'\''
                | b'('
                | b')'
                | b'*'
                | b'+'
                | b','
                | b';'
                | b'='
                | b':'
                | b'@'
                | b'['
                | b']'
                | b'%'
        ) || !should_escape(c, mode)
    })
}

/// Go `validOptionalPort`: empty, or `:` followed by digits only.
fn valid_optional_port(port: &str) -> bool {
    port.is_empty() || (port.starts_with(':') && port[1..].bytes().all(|b| b.is_ascii_digit()))
}

/// Go `validUserinfo`.
fn valid_userinfo(s: &str) -> bool {
    s.chars().all(|r| {
        r.is_ascii_alphanumeric()
            || matches!(
                r,
                '-' | '.'
                    | '_'
                    | ':'
                    | '~'
                    | '!'
                    | '$'
                    | '&'
                    | '\''
                    | '('
                    | ')'
                    | '*'
                    | '+'
                    | ','
                    | ';'
                    | '='
                    | '%'
                    | '@'
            )
    })
}

/// Go `getScheme`: split `scheme:rest` if the prefix is a valid scheme.
fn get_scheme(raw: &str) -> Result<(&str, &str), ParseError> {
    for (i, c) in raw.bytes().enumerate() {
        match c {
            b'a'..=b'z' | b'A'..=b'Z' => {}
            b'0'..=b'9' | b'+' | b'-' | b'.' if i > 0 => {}
            b':' if i == 0 => return Err(ParseError), // "missing protocol scheme"
            b':' => return Ok((&raw[..i], &raw[i + 1..])),
            _ => return Ok(("", raw)),
        }
    }
    Ok(("", raw))
}

/// Go 1.27 `parseHost` (host[:port] without userinfo); returns the unescaped host.
///
/// The original module declares `go 1.25.0`, so the `urlstrictcolons=0`
/// GODEBUG default applies: the port is split at the *last* colon.
fn parse_host(host: &str) -> Result<Vec<u8>, ParseError> {
    match host.rfind('[') {
        Some(open) if open > 0 => return Err(ParseError), // "invalid IP-literal"
        Some(_) => return parse_ip_literal(host),
        None => {}
    }
    if let Some(i) = host.rfind(':')
        && !valid_optional_port(&host[i..])
    {
        return Err(ParseError);
    }
    unescape(host.as_bytes(), Mode::Host)
}

/// `[ipv6]` or `[ipv6%25zone]` with an optional `:port`.
fn parse_ip_literal(host: &str) -> Result<Vec<u8>, ParseError> {
    let close = host.rfind(']').ok_or(ParseError)?;
    let colon_port = &host[close + 1..];
    if !valid_optional_port(colon_port) {
        return Err(ParseError);
    }
    let port = unescape(colon_port.as_bytes(), Mode::Host)?;
    let inner = host.get(1..close).ok_or(ParseError)?;
    let unescaped = match inner.find("%25") {
        Some(zone) => {
            let mut v = unescape(&inner.as_bytes()[..zone], Mode::Host)?;
            v.extend(unescape(&inner.as_bytes()[zone..], Mode::Zone)?);
            v
        }
        None => unescape(inner.as_bytes(), Mode::Host)?,
    };
    // netip.ParseAddr, then reject plain IPv4 (IPv4-mapped IPv6 is allowed
    // since Go 1.26).
    let text = std::str::from_utf8(&unescaped).map_err(|_| ParseError)?;
    let (addr, zone) = match text.split_once('%') {
        Some((a, z)) => (a, Some(z)),
        None => (text, None),
    };
    if zone == Some("")
        || addr.parse::<std::net::Ipv4Addr>().is_ok()
        || addr.parse::<Ipv6Addr>().is_err()
    {
        return Err(ParseError);
    }
    let mut out = b"[".to_vec();
    out.extend(unescaped);
    out.push(b']');
    out.extend(port);
    Ok(out)
}

impl GoUrl {
    /// Go `url.Parse` (non-request form).
    pub fn parse(raw: &str) -> Result<Self, ParseError> {
        let (before_fragment, fragment) = match raw.split_once('#') {
            Some((u, f)) => (u, Some(f)),
            None => (raw, None),
        };
        let mut url = Self::parse_without_fragment(before_fragment)?;
        if let Some(f) = fragment.filter(|f| !f.is_empty()) {
            url.fragment = unescape(f.as_bytes(), Mode::Fragment)?;
            url.raw_fragment = if escape(&url.fragment, Mode::Fragment) == f {
                String::new()
            } else {
                f.to_owned()
            };
        }
        Ok(url)
    }

    fn parse_without_fragment(raw: &str) -> Result<Self, ParseError> {
        if raw.bytes().any(|b| b < b' ' || b == 0x7f) {
            return Err(ParseError);
        }
        let mut url = Self::default();
        if raw == "*" {
            url.path = b"*".to_vec();
            return Ok(url);
        }
        let (scheme, mut rest) = get_scheme(raw)?;
        url.scheme = scheme.to_ascii_lowercase();
        if rest.ends_with('?') && rest.matches('?').count() == 1 {
            url.force_query = true;
            rest = &rest[..rest.len() - 1];
        } else if let Some((r, q)) = rest.split_once('?') {
            url.raw_query = q.to_owned();
            rest = r;
        }
        if !rest.starts_with('/') {
            if !url.scheme.is_empty() {
                url.opaque = rest.to_owned();
                return Ok(url);
            }
            if rest.split('/').next().unwrap_or_default().contains(':') {
                return Err(ParseError);
            }
        }
        if (!url.scheme.is_empty() || !rest.starts_with("///")) && rest.starts_with("//") {
            let (authority, path) = match rest[2..].find('/') {
                Some(i) => (&rest[2..2 + i], &rest[2 + i..]),
                None => (&rest[2..], ""),
            };
            rest = path;
            let host = match authority.rfind('@') {
                Some(i) => {
                    let info = &authority[..i];
                    if !valid_userinfo(info) {
                        return Err(ParseError);
                    }
                    let (name, password) = match info.split_once(':') {
                        Some((n, p)) => (n, Some(p)),
                        None => (info, None),
                    };
                    let host = parse_host(&authority[i + 1..])?;
                    let name = unescape(name.as_bytes(), Mode::UserPassword)?;
                    let mut rendered = escape(&name, Mode::UserPassword);
                    if let Some(p) = password {
                        let p = unescape(p.as_bytes(), Mode::UserPassword)?;
                        rendered.push(':');
                        rendered.push_str(&escape(&p, Mode::UserPassword));
                    }
                    url.user = Some(rendered);
                    host
                }
                None => parse_host(authority)?,
            };
            url.host = host;
        } else if !url.scheme.is_empty() && rest.starts_with('/') {
            url.omit_host = true;
        }
        url.path = unescape(rest.as_bytes(), Mode::Path)?;
        url.raw_path = if escape(&url.path, Mode::Path) == rest {
            String::new()
        } else {
            rest.to_owned()
        };
        Ok(url)
    }

    /// Go `URL.Hostname`: host without a valid port and without IPv6 brackets.
    pub fn hostname(&self) -> &[u8] {
        let mut host = self.host.as_slice();
        if let Some(colon) = host.iter().rposition(|&b| b == b':')
            && std::str::from_utf8(&host[colon..]).is_ok_and(valid_optional_port)
        {
            host = &host[..colon];
        }
        if host.starts_with(b"[") && host.ends_with(b"]") && host.len() >= 2 {
            host = &host[1..host.len() - 1];
        }
        host
    }

    /// Go `URL.EscapedPath`.
    fn escaped_path(&self) -> String {
        if !self.raw_path.is_empty()
            && valid_encoded(&self.raw_path, Mode::Path)
            && unescape(self.raw_path.as_bytes(), Mode::Path).as_deref() == Ok(&self.path[..])
        {
            return self.raw_path.clone();
        }
        if self.path == b"*" {
            return "*".into();
        }
        escape(&self.path, Mode::Path)
    }

    /// Go `URL.EscapedFragment`.
    fn escaped_fragment(&self) -> String {
        if !self.raw_fragment.is_empty()
            && valid_encoded(&self.raw_fragment, Mode::Fragment)
            && unescape(self.raw_fragment.as_bytes(), Mode::Fragment).as_deref()
                == Ok(&self.fragment[..])
        {
            return self.raw_fragment.clone();
        }
        escape(&self.fragment, Mode::Fragment)
    }

    /// Build a URL the way `(&url.URL{Scheme, Host, Path}).String()` does.
    pub fn from_parts(scheme: &str, host: &str, path: &str) -> Self {
        Self {
            scheme: scheme.into(),
            host: host.as_bytes().to_vec(),
            path: path.as_bytes().to_vec(),
            ..Self::default()
        }
    }
}

/// Go `URL.String`.
impl std::fmt::Display for GoUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut out = String::new();
        if !self.scheme.is_empty() {
            out.push_str(&self.scheme);
            out.push(':');
        }
        if !self.opaque.is_empty() {
            out.push_str(&self.opaque);
        } else {
            if (!self.scheme.is_empty() || !self.host.is_empty() || self.user.is_some())
                && !(self.omit_host && self.host.is_empty() && self.user.is_none())
            {
                if !self.host.is_empty() || !self.path.is_empty() || self.user.is_some() {
                    out.push_str("//");
                }
                if let Some(user) = &self.user {
                    out.push_str(user);
                    out.push('@');
                }
                out.push_str(&escape(&self.host, Mode::Host));
            }
            let mut path = self.escaped_path();
            if self.omit_host
                && self.host.is_empty()
                && self.user.is_none()
                && path.starts_with("//")
            {
                out.push_str("%2F");
                path.remove(0);
            }
            if !path.is_empty() && !path.starts_with('/') && !self.host.is_empty() {
                out.push('/');
            }
            if out.is_empty() && path.split('/').next().unwrap_or_default().contains(':') {
                out.push_str("./");
            }
            out.push_str(&path);
        }
        if self.force_query || !self.raw_query.is_empty() {
            out.push('?');
            out.push_str(&self.raw_query);
        }
        if !self.fragment.is_empty() {
            out.push('#');
            out.push_str(&self.escaped_fragment());
        }
        f.write_str(&out)
    }
}

#[cfg(test)]
mod tests {
    use super::GoUrl;

    // Expected values were produced by the original Go 1.27.1 net/url
    // (see tests/compare.py, which checks many more cases end to end).
    #[test]
    fn matches_go_rendering() {
        for (raw, rendered) in [
            ("http://x.com/a b", "http://x.com/a%20b"),
            ("HTTP://X.com", "http://X.com"),
            ("http://x.com:99999", "http://x.com:99999"),
            ("http://[::1]:80/p", "http://[::1]:80/p"),
            ("http://x.com?", "http://x.com?"),
            (
                "http://例.com/ü#frag ment",
                "http://%E4%BE%8B.com/%C3%BC#frag%20ment",
            ),
            ("http:/x.com", "http:/x.com"),
            ("http:///x.com", "http:///x.com"),
        ] {
            assert_eq!(GoUrl::parse(raw).unwrap().to_string(), rendered, "{raw}");
        }
        for raw in [" http://x.com", "http://a{b.com", "http://x.com/%zz"] {
            assert!(GoUrl::parse(raw).is_err(), "{raw}");
        }
    }
}
