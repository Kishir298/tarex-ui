use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use bytes::Bytes;
use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;

const HEADER_BLOCKLIST: &[&str] = &[
    "host",
    "content-length",
    "connection",
    "proxy-authorization",
    "proxy-connection",
    "te",
    "transfer-encoding",
    "upgrade",
    "trailer",
    "expect",
];

fn is_blocked_host_name(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    matches!(
        host.as_str(),
        "metadata.google.internal" | "metadata" | "metadata.azure.com"
    )
}

fn ip_kind(ip: IpAddr) -> IpKind {
    match ip {
        IpAddr::V4(v) => {
            let o = v.octets();
            // Cloud metadata IPv4: 169.254.169.254
            if v.is_link_local() {
                return IpKind::BlockedMetadata;
            }
            if v.is_loopback() || v.is_unspecified() || v.is_broadcast() || v.is_multicast() {
                return IpKind::Loopback;
            }
            // RFC1918 + CGNAT + benchmarking + IETF
            if o[0] == 10
                || (o[0] == 172 && (16..=31).contains(&o[1]))
                || (o[0] == 192 && o[1] == 168)
                || (o[0] == 100 && (64..=127).contains(&o[1]))
                || (o[0] == 198 && (o[1] == 18 || o[1] == 19))
            {
                return IpKind::Private;
            }
            IpKind::Public
        }
        IpAddr::V6(v) => {
            if v.is_loopback() || v.is_unspecified() || v.is_multicast() {
                return IpKind::Loopback;
            }
            // IPv4-mapped (::ffff:a.b.c.d) and IPv4-compatible (::a.b.c.d)
            // carry an IPv4 address in their low 32 bits. Classified as plain
            // IPv6 they read as public, which would let
            // [::ffff:127.0.0.1] and [::ffff:169.254.169.254] past every
            // IPv4 rule below.
            if let Some(embedded) = embedded_ipv4(v) {
                return ip_kind(IpAddr::V4(embedded));
            }
            // Cloud metadata IPv6 (AWS): fd00:ec2::254
            let segs = v.segments();
            if segs[0] == 0xfd00 && segs[1] == 0xec2 {
                return IpKind::BlockedMetadata;
            }
            // fe80::/10 link-local
            if segs[0] & 0xffc0 == 0xfe80 {
                return IpKind::BlockedMetadata;
            }
            // fc00::/7 unique-local (private)
            if segs[0] & 0xfe00 == 0xfc00 {
                return IpKind::Private;
            }
            IpKind::Public
        }
    }
}

/// The IPv4 address wrapped by an IPv4-mapped (`::ffff:a.b.c.d`) or
/// IPv4-compatible (`::a.b.c.d`) IPv6 address. `::` and `::1` are the
/// unspecified and loopback addresses rather than wrapped hosts and are
/// excluded so they keep their own classification even if a caller reorders
/// the checks above.
fn embedded_ipv4(v: Ipv6Addr) -> Option<Ipv4Addr> {
    if let Some(mapped) = v.to_ipv4_mapped() {
        return Some(mapped);
    }
    let s = v.segments();
    if !s[..6].iter().all(|&seg| seg == 0) {
        return None;
    }
    if s[6] == 0 && s[7] <= 1 {
        return None;
    }
    Some(Ipv4Addr::new(
        (s[6] >> 8) as u8,
        s[6] as u8,
        (s[7] >> 8) as u8,
        s[7] as u8,
    ))
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum IpKind {
    Public,
    Private,
    Loopback,
    BlockedMetadata,
}

/// Resolve `host` once and return both its safety classification and the
/// concrete IPs we resolved. Callers can pin reqwest to these IPs to defeat
/// DNS rebinding (where a second lookup returns a different address).
async fn resolve_and_classify(host: &str) -> Result<(IpKind, Vec<IpAddr>), String> {
    // Direct literal? Skip DNS.
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok((ip_kind(ip), vec![ip]));
    }
    let host_owned = host.to_string();
    let lookup = tokio::task::spawn_blocking(move || {
        (host_owned.as_str(), 0u16)
            .to_socket_addrs()
            .map(|it| it.map(|a| a.ip()).collect::<Vec<_>>())
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("dns: {e}"))?;
    if lookup.is_empty() {
        return Err("dns: no addresses".into());
    }
    let mut worst = IpKind::Public;
    for ip in &lookup {
        let k = ip_kind(*ip);
        worst = match (worst, k) {
            (_, IpKind::BlockedMetadata) => IpKind::BlockedMetadata,
            (IpKind::BlockedMetadata, _) => IpKind::BlockedMetadata,
            (IpKind::Public, x) => x,
            (x, IpKind::Public) => x,
            (a, _) => a,
        };
    }
    Ok((worst, lookup))
}

use std::net::ToSocketAddrs;

fn validate_url(url: &str, allow_private: bool) -> Result<reqwest::Url, String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("invalid url: {e}"))?;
    match parsed.scheme() {
        "http" | "https" => {}
        s => return Err(format!("scheme not allowed: {s}")),
    }
    if parsed.username() != "" || parsed.password().is_some() {
        return Err("userinfo in url is not allowed".into());
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| "missing host".to_string())?;
    if is_blocked_host_name(host) {
        return Err(format!("host not allowed: {host}"));
    }
    // The actual IP classification has to be async — caller does it.
    let _ = allow_private;
    Ok(parsed)
}

/// Classify the host AND return safe IPs to pin reqwest's resolver to.
/// Defeats DNS rebinding (second-lookup-returns-different-IP) by reusing
/// exactly the addresses that passed `ip_kind`.
async fn classify_and_collect_safe_ips(
    host: &str,
    allow_private: bool,
) -> Result<Vec<IpAddr>, String> {
    let (worst, ips) = resolve_and_classify(host).await?;
    match worst {
        IpKind::BlockedMetadata => return Err(format!("host not allowed: {host}")),
        IpKind::Loopback | IpKind::Private if !allow_private => {
            return Err(format!(
                "host {host} resolves to a private/loopback address; this endpoint requires explicit opt-in",
            ));
        }
        _ => {}
    }
    let safe: Vec<IpAddr> = ips
        .into_iter()
        .filter(|ip| match ip_kind(*ip) {
            IpKind::BlockedMetadata => false,
            IpKind::Loopback | IpKind::Private => allow_private,
            IpKind::Public => true,
        })
        .collect();
    if safe.is_empty() {
        return Err(format!("host {host}: no safe IPs"));
    }
    Ok(safe)
}

fn sanitize_headers(headers: Option<HashMap<String, String>>) -> Result<HeaderMap, String> {
    let mut map = HeaderMap::new();
    let Some(h) = headers else { return Ok(map) };
    for (k, v) in h {
        let lower = k.to_ascii_lowercase();
        if HEADER_BLOCKLIST.contains(&lower.as_str()) {
            return Err(format!("header not allowed: {k}"));
        }
        // CRLF injection: header value must not contain CR / LF / NUL.
        if v.as_bytes().iter().any(|b| matches!(b, 0 | b'\r' | b'\n')) {
            return Err(format!("header value contains control bytes: {k}"));
        }
        let name = HeaderName::from_bytes(k.as_bytes()).map_err(|e| e.to_string())?;
        let value = HeaderValue::from_str(&v).map_err(|e| e.to_string())?;
        map.insert(name, value);
    }
    Ok(map)
}

#[tauri::command]
pub async fn lm_ping(base_url: String) -> Result<u16, String> {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("empty base url".into());
    }
    let probe = format!("{trimmed}/models");
    let parsed = validate_url(&probe, true)?;
    let host = parsed
        .host_str()
        .ok_or_else(|| "missing host".to_string())?
        .to_string();
    let safe_ips = classify_and_collect_safe_ips(&host, true).await?;

    let mut builder = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none());
    let addrs: Vec<SocketAddr> = safe_ips.iter().map(|ip| SocketAddr::new(*ip, 0)).collect();
    builder = builder.resolve_to_addrs(&host, &addrs);
    let client = builder.build().map_err(|e| e.to_string())?;
    client
        .get(parsed)
        .send()
        .await
        .map(|r| r.status().as_u16())
        .map_err(|e| e.to_string())
}
// AI HTTP proxy — bypasses webview CORS / Mixed-Content / PNA so local-network
// model servers (LM Studio, Ollama, vLLM) work in the production bundle.

#[derive(Debug, Serialize)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

fn build_request(
    client: &reqwest::Client,
    method: &str,
    url: reqwest::Url,
    headers: Option<HashMap<String, String>>,
    body: Option<Vec<u8>>,
) -> Result<reqwest::RequestBuilder, String> {
    let method = Method::from_bytes(method.as_bytes()).map_err(|e| e.to_string())?;
    let mut req = client.request(method, url);
    let map = sanitize_headers(headers)?;
    req = req.headers(map);
    if let Some(b) = body {
        req = req.body(b);
    }
    Ok(req)
}

fn build_safe_client(pinned: &[(String, Vec<IpAddr>)]) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder().connect_timeout(Duration::from_secs(10));
    // Pin reqwest's resolver to the IPs we just classified. Without this,
    // reqwest's own DNS lookup could return a different (private/metadata) IP
    // for the same hostname between classify and connect — classic DNS
    // rebinding attack. We pin port 0 because reqwest fills in the actual
    // port from the URL when wiring up the override map.
    for (host, ips) in pinned {
        let addrs: Vec<SocketAddr> = ips.iter().map(|ip| SocketAddr::new(*ip, 0)).collect();
        if !addrs.is_empty() {
            builder = builder.resolve_to_addrs(host, &addrs);
        }
    }
    // Redirects are followed by `send_checked`, not here: reqwest's hook runs
    // synchronously, so it cannot resolve and classify a hostname it has
    // never seen — which is exactly how a redirect reaches a metadata
    // endpoint.
    builder
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())
}

const MAX_REDIRECTS: u8 = 10;

/// Statuses that carry a `Location` we are willing to follow.
const FOLLOWABLE_REDIRECTS: &[u16] = &[301, 302, 303, 307, 308];

/// Credentials must not be replayed to a different origin.
const CROSS_ORIGIN_STRIP: &[&str] = &["authorization", "cookie", "proxy-authorization"];

/// Describes a body that a 301/302/303 conversion to GET throws away.
const BODY_HEADERS: &[&str] = &[
    "content-type",
    "content-length",
    "content-encoding",
    "transfer-encoding",
];

fn is_cross_origin(from: &reqwest::Url, to: &reqwest::Url) -> bool {
    from.scheme() != to.scheme()
        || from.host_str() != to.host_str()
        || from.port_or_known_default() != to.port_or_known_default()
}

fn drop_headers(headers: &mut Option<HashMap<String, String>>, list: &[&str]) {
    if let Some(map) = headers.as_mut() {
        map.retain(|k, _| !list.iter().any(|l| k.eq_ignore_ascii_case(l)));
    }
}

/// Rewrite method, body and headers for one redirect hop. Mirrors reqwest's
/// own rules so behaviour does not change, but runs against URLs the caller
/// has already validated and classified.
fn redirect_follow(
    status: u16,
    method: &str,
    body: Option<Vec<u8>>,
    mut headers: Option<HashMap<String, String>>,
    from: &reqwest::Url,
    to: &reqwest::Url,
) -> (String, Option<Vec<u8>>, Option<HashMap<String, String>>) {
    let mut method = method.to_string();
    let mut body = body;
    if matches!(status, 301..=303) && method != "GET" && method != "HEAD" {
        method = "GET".into();
        body = None;
    }
    if body.is_none() {
        drop_headers(&mut headers, BODY_HEADERS);
    }
    if is_cross_origin(from, to) {
        drop_headers(&mut headers, CROSS_ORIGIN_STRIP);
    }
    (method, body, headers)
}

/// Issue `method`/`url` and follow redirects by hand.
///
/// Every hop — the first included — goes through URL validation, DNS
/// resolution, IP classification and resolver pinning, so a redirect cannot
/// reach an address the original request would have been refused for. Returns
/// the client that owns the connection; the caller must keep it alive for as
/// long as it reads the response body.
async fn send_checked(
    method: &str,
    url: reqwest::Url,
    headers: Option<HashMap<String, String>>,
    body: Option<Vec<u8>>,
    allow_private: bool,
) -> Result<(reqwest::Client, reqwest::Response), String> {
    let mut current_url = url;
    let mut current_method = method.to_string();
    let mut current_headers = headers;
    let mut current_body = body;

    for _ in 0..=MAX_REDIRECTS {
        let host = current_url
            .host_str()
            .ok_or_else(|| "missing host".to_string())?
            .to_string();
        let safe_ips = classify_and_collect_safe_ips(&host, allow_private).await?;
        let client = build_safe_client(&[(host, safe_ips)])?;
        let req = build_request(
            &client,
            &current_method,
            current_url.clone(),
            current_headers.clone(),
            current_body.clone(),
        )?;
        let resp = req.send().await.map_err(|e| e.to_string())?;

        let status = resp.status().as_u16();
        if !FOLLOWABLE_REDIRECTS.contains(&status) {
            return Ok((client, resp));
        }
        let Some(location) = resp.headers().get(reqwest::header::LOCATION) else {
            return Ok((client, resp));
        };
        let location = location
            .to_str()
            .map_err(|_| "invalid redirect location".to_string())?;
        let next = current_url
            .join(location)
            .map_err(|e| format!("invalid redirect location: {e}"))?;
        // The redirect target obeys the same rules as the original URL:
        // scheme, no userinfo, no metadata hostname.
        validate_url(next.as_str(), allow_private)?;

        let (method, body, headers) = redirect_follow(
            status,
            &current_method,
            current_body.clone(),
            current_headers.clone(),
            &current_url,
            &next,
        );
        current_method = method;
        current_body = body;
        current_headers = headers;
        current_url = next;
    }
    Err("too many redirects".into())
}

fn header_map_to_strings(headers: &HeaderMap) -> HashMap<String, String> {
    let mut out = HashMap::with_capacity(headers.len());
    for (k, v) in headers {
        if let Ok(s) = v.to_str() {
            out.insert(k.as_str().to_ascii_lowercase(), s.to_string());
        }
    }
    out
}

#[tauri::command]
pub async fn ai_http_request(
    url: String,
    method: String,
    headers: Option<HashMap<String, String>>,
    body: Option<Vec<u8>>,
    allow_private_network: Option<bool>,
) -> Result<HttpResponse, String> {
    let allow_private = allow_private_network.unwrap_or(false);
    let parsed = validate_url(&url, allow_private)?;

    // One code path for both commands so a redirect can never be validated
    // differently depending on which one called it.
    let (client, resp) = send_checked(&method, parsed, headers, body, allow_private).await?;
    // Keep the connection pool alive while the body is read.
    let _client = client;

    let status = resp.status().as_u16();
    let headers = header_map_to_strings(resp.headers());
    let body = resp.bytes().await.map_err(|e| e.to_string())?.to_vec();
    Ok(HttpResponse {
        status,
        headers,
        body,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AiStreamEvent {
    Headers {
        status: u16,
        headers: HashMap<String, String>,
    },
    Chunk {
        bytes: Vec<u8>,
    },
    End,
    Error {
        message: String,
    },
}

#[tauri::command]
pub async fn ai_http_stream(
    url: String,
    method: String,
    headers: Option<HashMap<String, String>>,
    body: Option<Vec<u8>>,
    allow_private_network: Option<bool>,
    on_event: Channel<AiStreamEvent>,
) -> Result<(), String> {
    let allow_private = allow_private_network.unwrap_or(false);
    let parsed = match validate_url(&url, allow_private) {
        Ok(p) => p,
        Err(e) => {
            let _ = on_event.send(AiStreamEvent::Error { message: e.clone() });
            return Err(e);
        }
    };
    // One code path for both commands so a redirect can never be validated
    // differently depending on which one called it.
    let (client, resp) = match send_checked(&method, parsed, headers, body, allow_private).await {
        Ok(v) => v,
        Err(e) => {
            let _ = on_event.send(AiStreamEvent::Error { message: e.clone() });
            return Err(e);
        }
    };
    // Held for as long as the response body streams.
    let _client = client;

    let status = resp.status().as_u16();
    let headers = header_map_to_strings(resp.headers());
    let _ = on_event.send(AiStreamEvent::Headers { status, headers });

    let mut stream = resp.bytes_stream();
    while let Some(item) = stream.next().await {
        match item {
            Ok(chunk) => {
                let bytes: Bytes = chunk;
                if on_event
                    .send(AiStreamEvent::Chunk {
                        bytes: bytes.to_vec(),
                    })
                    .is_err()
                {
                    // Channel dropped (frontend aborted) — stop streaming.
                    return Ok(());
                }
            }
            Err(e) => {
                let _ = on_event.send(AiStreamEvent::Error {
                    message: e.to_string(),
                });
                return Err(e.to_string());
            }
        }
    }

    let _ = on_event.send(AiStreamEvent::End);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn metadata_ips_classified_as_blocked() {
        // AWS / Google / Azure all share the IPv4 169.254.169.254 link-local.
        assert_eq!(
            ip_kind(IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254))),
            IpKind::BlockedMetadata
        );
        // AWS IPv6 metadata
        assert_eq!(
            ip_kind("fd00:ec2::254".parse().unwrap()),
            IpKind::BlockedMetadata
        );
        // Any link-local IPv4 (169.254/16) — same network range, still blocked.
        assert_eq!(
            ip_kind(IpAddr::V4(Ipv4Addr::new(169, 254, 1, 1))),
            IpKind::BlockedMetadata
        );
        // IPv6 link-local fe80::/10
        assert_eq!(ip_kind("fe80::1".parse().unwrap()), IpKind::BlockedMetadata);
    }

    #[test]
    fn private_ips_classified_correctly() {
        assert_eq!(
            ip_kind(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))),
            IpKind::Private
        );
        assert_eq!(
            ip_kind(IpAddr::V4(Ipv4Addr::new(172, 16, 0, 1))),
            IpKind::Private
        );
        assert_eq!(
            ip_kind(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))),
            IpKind::Private
        );
        // CGNAT 100.64/10
        assert_eq!(
            ip_kind(IpAddr::V4(Ipv4Addr::new(100, 64, 0, 1))),
            IpKind::Private
        );
    }

    #[test]
    fn loopback_classified_as_loopback() {
        assert_eq!(
            ip_kind(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))),
            IpKind::Loopback
        );
        assert_eq!(ip_kind("::1".parse().unwrap()), IpKind::Loopback);
    }

    #[test]
    fn public_ips_classified_as_public() {
        assert_eq!(
            ip_kind(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))),
            IpKind::Public
        );
        assert_eq!(
            ip_kind(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))),
            IpKind::Public
        );
    }

    #[test]
    fn validate_url_blocks_userinfo_and_metadata_hostnames() {
        // URLs with userinfo can confuse browsers / leak creds in redirects.
        assert!(validate_url("http://user:pass@example.com/", true).is_err());
        // Cloud metadata-by-name.
        assert!(validate_url("http://metadata.google.internal/", true).is_err());
        assert!(validate_url("http://metadata/", true).is_err());
        assert!(validate_url("http://metadata.azure.com/", true).is_err());
    }

    #[test]
    fn validate_url_rejects_non_http_schemes() {
        assert!(validate_url("ftp://example.com/", true).is_err());
        assert!(validate_url("file:///etc/passwd", true).is_err());
        assert!(validate_url("javascript:alert(1)", true).is_err());
    }

    #[test]
    fn sanitize_headers_blocks_crlf_injection() {
        let mut h = HashMap::new();
        h.insert("X-Foo".to_string(), "bar\r\nX-Evil: yes".to_string());
        assert!(sanitize_headers(Some(h)).is_err());
    }

    #[test]
    fn sanitize_headers_blocks_hop_by_hop_headers() {
        for hop in [
            "host",
            "content-length",
            "connection",
            "proxy-authorization",
        ] {
            let mut h = HashMap::new();
            h.insert(hop.to_string(), "value".to_string());
            assert!(
                sanitize_headers(Some(h)).is_err(),
                "expected {hop} to be rejected"
            );
        }
    }

    #[test]
    fn special_ipv4_ranges_do_not_classify_as_public() {
        assert_eq!(ip_kind(IpAddr::V4(Ipv4Addr::UNSPECIFIED)), IpKind::Loopback);
        assert_eq!(ip_kind(IpAddr::V4(Ipv4Addr::BROADCAST)), IpKind::Loopback);
        assert_eq!(
            ip_kind(IpAddr::V4(Ipv4Addr::new(224, 0, 0, 1))),
            IpKind::Loopback
        );
    }

    #[test]
    fn private_range_boundaries_hold_on_ipv4() {
        assert_eq!(
            ip_kind(IpAddr::V4(Ipv4Addr::new(172, 31, 255, 255))),
            IpKind::Private
        );
        assert_eq!(
            ip_kind(IpAddr::V4(Ipv4Addr::new(172, 32, 0, 1))),
            IpKind::Public
        );
        assert_eq!(
            ip_kind(IpAddr::V4(Ipv4Addr::new(100, 127, 255, 255))),
            IpKind::Private
        );
        assert_eq!(
            ip_kind(IpAddr::V4(Ipv4Addr::new(100, 128, 0, 1))),
            IpKind::Public
        );
        assert_eq!(
            ip_kind(IpAddr::V4(Ipv4Addr::new(198, 18, 0, 1))),
            IpKind::Private
        );
        assert_eq!(
            ip_kind(IpAddr::V4(Ipv4Addr::new(198, 19, 255, 255))),
            IpKind::Private
        );
        assert_eq!(
            ip_kind(IpAddr::V4(Ipv4Addr::new(198, 20, 0, 1))),
            IpKind::Public
        );
    }

    #[test]
    fn ipv6_unique_local_and_multicast_are_not_public() {
        assert_eq!(ip_kind("fd12:3456::1".parse().unwrap()), IpKind::Private);
        assert_eq!(ip_kind("fc00::1".parse().unwrap()), IpKind::Private);
        assert_eq!(ip_kind("ff02::1".parse().unwrap()), IpKind::Loopback);
        assert_eq!(ip_kind("::".parse().unwrap()), IpKind::Loopback);
        assert_eq!(ip_kind("2606:4700::1111".parse().unwrap()), IpKind::Public);
    }

    #[test]
    fn blocked_host_name_match_is_case_insensitive_and_exact() {
        assert!(is_blocked_host_name("METADATA.google.internal"));
        assert!(!is_blocked_host_name("metadata.evil.internal"));
        assert!(!is_blocked_host_name("my-metadata"));
    }

    #[test]
    fn validate_url_requires_a_host() {
        // A URL that parses but carries no host must not slip through.
        assert!(validate_url("http://", true).is_err());
    }

    #[test]
    fn ipv4_mapped_and_compatible_addresses_use_ipv4_rules() {
        // Without unwrapping, these read as plain IPv6 and classify as public,
        // which is how [::ffff:127.0.0.1] and [::ffff:169.254.169.254] used to
        // slip past every IPv4 rule.
        assert_eq!(
            ip_kind("::ffff:127.0.0.1".parse().unwrap()),
            IpKind::Loopback
        );
        assert_eq!(
            ip_kind("::ffff:169.254.169.254".parse().unwrap()),
            IpKind::BlockedMetadata
        );
        assert_eq!(ip_kind("::ffff:10.0.0.1".parse().unwrap()), IpKind::Private);
        assert_eq!(
            ip_kind("::ffff:192.168.1.1".parse().unwrap()),
            IpKind::Private
        );
        assert_eq!(ip_kind("::ffff:8.8.8.8".parse().unwrap()), IpKind::Public);
        // Deprecated IPv4-compatible form (::a.b.c.d).
        assert_eq!(ip_kind("::10.0.0.1".parse().unwrap()), IpKind::Private);
        assert_eq!(ip_kind("::127.0.0.1".parse().unwrap()), IpKind::Loopback);
        // The bare IPv6 addresses keep their own meaning.
        assert_eq!(ip_kind("::1".parse().unwrap()), IpKind::Loopback);
        assert_eq!(ip_kind("::".parse().unwrap()), IpKind::Loopback);
        assert_eq!(
            ip_kind("fd00:ec2::254".parse().unwrap()),
            IpKind::BlockedMetadata
        );
    }

    #[test]
    fn embedded_ipv4_only_unwraps_the_wrapped_forms() {
        assert_eq!(
            embedded_ipv4("::ffff:127.0.0.1".parse().unwrap()),
            Some(Ipv4Addr::new(127, 0, 0, 1))
        );
        assert_eq!(
            embedded_ipv4("::10.0.0.1".parse().unwrap()),
            Some(Ipv4Addr::new(10, 0, 0, 1))
        );
        assert_eq!(embedded_ipv4("::1".parse().unwrap()), None);
        assert_eq!(embedded_ipv4("::".parse().unwrap()), None);
        assert_eq!(embedded_ipv4("fd00:ec2::254".parse().unwrap()), None);
        assert_eq!(embedded_ipv4("2606:4700::1111".parse().unwrap()), None);
    }

    fn url(s: &str) -> reqwest::Url {
        reqwest::Url::parse(s).unwrap()
    }

    #[test]
    fn redirect_302_turns_a_body_carrying_method_into_get() {
        let headers = HashMap::from([("Content-Type".to_string(), "text/plain".to_string())]);
        let (m, b, h) = redirect_follow(
            302,
            "POST",
            Some(b"x".to_vec()),
            Some(headers),
            &url("http://a.example/x"),
            &url("http://a.example/y"),
        );
        assert_eq!(m, "GET");
        assert!(b.is_none(), "the body must not follow a converted GET");
        assert!(
            h.unwrap().is_empty(),
            "body headers must not survive the conversion"
        );
    }

    #[test]
    fn redirect_307_preserves_method_body_and_headers() {
        let headers = HashMap::from([("Content-Type".to_string(), "text/plain".to_string())]);
        let (m, b, h) = redirect_follow(
            307,
            "POST",
            Some(b"x".to_vec()),
            Some(headers),
            &url("http://a.example/x"),
            &url("http://a.example/y"),
        );
        assert_eq!(m, "POST");
        assert_eq!(b.as_deref(), Some(b"x".as_ref()));
        assert!(h.unwrap().contains_key("Content-Type"));
    }

    #[test]
    fn redirect_strips_credentials_only_across_origins() {
        let headers = || {
            HashMap::from([
                ("Authorization".to_string(), "Bearer t".to_string()),
                ("X-Keep".to_string(), "1".to_string()),
            ])
        };

        let (.., same) = redirect_follow(
            302,
            "GET",
            None,
            Some(headers()),
            &url("http://a.example/x"),
            &url("http://a.example/y"),
        );
        assert_eq!(
            same.unwrap().get("Authorization").map(String::as_str),
            Some("Bearer t"),
            "same-origin redirects keep the credential"
        );

        let (.., cross) = redirect_follow(
            302,
            "GET",
            None,
            Some(headers()),
            &url("http://a.example/x"),
            &url("https://b.example/y"),
        );
        let cross = cross.unwrap();
        assert!(
            !cross.contains_key("Authorization"),
            "a cross-origin redirect must not replay the credential"
        );
        assert_eq!(cross.get("X-Keep").map(String::as_str), Some("1"));

        let (.., port) = redirect_follow(
            302,
            "GET",
            None,
            Some(headers()),
            &url("http://a.example:80/x"),
            &url("http://a.example:8080/y"),
        );
        assert!(!port.unwrap().contains_key("Authorization"));
    }

    #[test]
    fn redirect_target_must_still_pass_url_rules() {
        // A redirect is only followed after the same validation the first URL
        // gets — a Location header must not smuggle userinfo or a metadata
        // hostname past the initial check.
        assert!(validate_url("http://user:pass@evil.example/", false).is_err());
        assert!(validate_url("http://metadata.google.internal/", false).is_err());
        assert!(validate_url("http://169.254.169.254/", false).is_ok());
    }
}
