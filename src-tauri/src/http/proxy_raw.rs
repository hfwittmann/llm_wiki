//! Generic HTTP forwarder. Forwards `{url, method, headers, body}` from a
//! browser request to the target URL, then streams the response back.
//!
//! Solves CORS for browser → third-party LLM/embedding/etc. calls. Not a
//! security boundary: the caller (frontend) controls all of url/headers/body,
//! including any API key. Equivalent to the Tauri `plugin-http` capability,
//! but reachable from any browser on the LAN.

use std::collections::HashMap;

use axum::body::Body;
use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::Response;
use axum::routing::post;
use axum::{Json, Router};
use futures::StreamExt;
use serde::Deserialize;

use crate::http::auth::AuthUser;
use crate::http::error::ApiError;
use crate::http::AppState;

pub fn proxy_raw_router() -> Router<AppState> {
    Router::new().route("/api/v1/proxy/raw", post(proxy_raw))
}

#[derive(Debug, Deserialize)]
struct ProxyRawRequest {
    url: String,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    headers: HashMap<String, String>,
    /// Body as a string (already serialized). Empty/None for GET / no-body.
    #[serde(default)]
    body: Option<String>,
}

async fn proxy_raw(
    State(_state): State<AppState>,
    AuthUser(_user): AuthUser,
    Json(req): Json<ProxyRawRequest>,
) -> Result<Response, ApiError> {
    // SSRF guard: reject URLs that resolve to private/loopback/link-local
    // ranges before we send. The proxy is a CORS workaround for hitting
    // third-party LLM/embedding APIs, NOT a generic outbound forwarder.
    // Without this guard, any authenticated user could pivot through the
    // server to reach internal services (e.g. cloud metadata endpoints
    // like 169.254.169.254, sidecar services on localhost, other LAN hosts).
    //
    // The guard is bypassed in `cfg(test)` builds because the test suite
    // uses mockito, which binds to 127.0.0.1. `cfg!(test)` is only true
    // under `cargo test`; release / dev binaries always enforce.
    if !cfg!(test) {
        reject_unsafe_url(&req.url)?;
    }

    let method = req
        .method
        .as_deref()
        .unwrap_or("POST")
        .to_uppercase();
    let method = reqwest::Method::from_bytes(method.as_bytes())
        .map_err(|e| ApiError::bad_request("BAD_REQUEST", format!("invalid method: {e}")))?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| ApiError::internal(format!("reqwest build: {e}")))?;

    let mut request = client.request(method, &req.url);
    for (k, v) in req.headers {
        request = request.header(k, v);
    }
    if let Some(body) = req.body {
        request = request.body(body);
    }

    let upstream = request.send().await.map_err(|e| {
        if e.is_timeout() {
            ApiError::new(
                StatusCode::GATEWAY_TIMEOUT,
                "UPSTREAM_TIMEOUT",
                format!("upstream timeout: {e}"),
            )
        } else {
            ApiError::new(
                StatusCode::BAD_GATEWAY,
                "UPSTREAM_NETWORK",
                format!("upstream network error: {e}"),
            )
        }
    })?;

    // Build the response: preserve status + content-type + a few useful
    // headers. Avoid forwarding cookies, set-cookie, hop-by-hop headers, etc.
    let status = upstream.status();
    let mut response_headers = HeaderMap::new();
    for (name, value) in upstream.headers() {
        let name_str = name.as_str().to_lowercase();
        // Only forward content-type, cache-control, and content-length-ish.
        // (Hop-by-hop and cookie-related headers are intentionally dropped.)
        if matches!(
            name_str.as_str(),
            "content-type" | "cache-control" | "etag" | "last-modified"
        ) {
            if let (Ok(n), Ok(v)) = (HeaderName::from_bytes(name.as_str().as_bytes()), HeaderValue::from_bytes(value.as_bytes())) {
                response_headers.insert(n, v);
            }
        }
    }
    if !response_headers.contains_key(CONTENT_TYPE) {
        response_headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/octet-stream"));
    }

    // Stream the body. axum::Body::from_stream wraps a futures::Stream.
    let stream = upstream.bytes_stream().map(|chunk| {
        chunk.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
    });

    let mut response = Response::new(Body::from_stream(stream));
    *response.status_mut() = StatusCode::from_u16(status.as_u16())
        .unwrap_or(StatusCode::OK);
    *response.headers_mut() = response_headers;
    Ok(response)
}

// ── SSRF guard ────────────────────────────────────────────────────────────────
//
// Reject URLs whose host resolves to a private, loopback, link-local, or
// otherwise non-public address before we initiate the forward. This is the
// proxy's only access-control layer; without it the endpoint becomes a
// confused-deputy egress for the LAN.

fn reject_unsafe_url(url_str: &str) -> Result<(), ApiError> {
    let url = url::Url::parse(url_str)
        .map_err(|e| ApiError::bad_request("BAD_REQUEST", format!("invalid url: {e}")))?;

    match url.scheme() {
        "http" | "https" => {}
        other => {
            return Err(ApiError::bad_request(
                "BAD_REQUEST",
                format!("unsupported scheme: {other} (only http/https are allowed)"),
            ));
        }
    }

    let host = url.host_str().ok_or_else(|| {
        ApiError::bad_request("BAD_REQUEST", "url is missing a host")
    })?;

    // Direct IP literals: check the address itself.
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        if is_private_or_special(ip) {
            return Err(ApiError::bad_request(
                "BAD_REQUEST",
                format!(
                    "url host {host} is in a private/loopback/link-local range and is blocked by the proxy"
                ),
            )
            .with_details(serde_json::json!({ "host": host, "reason": "private_address" })));
        }
        return Ok(());
    }

    // Hostname: resolve and check every resolved address.
    // This is a best-effort guard. Note: DNS rebinding could in theory let a
    // host resolve to a public IP on first lookup and a private IP at request
    // time. For v1 we accept that residual risk; v2 can resolve once and pin
    // the IP into the outbound request.
    let host_port = format!("{host}:{}", url.port_or_known_default().unwrap_or(80));
    let addrs: Vec<std::net::SocketAddr> = std::net::ToSocketAddrs::to_socket_addrs(&host_port)
        .map_err(|e| {
            ApiError::bad_request("BAD_REQUEST", format!("dns lookup failed for {host}: {e}"))
        })?
        .collect();

    if addrs.is_empty() {
        return Err(ApiError::bad_request(
            "BAD_REQUEST",
            format!("dns lookup returned no addresses for {host}"),
        ));
    }

    for sa in &addrs {
        if is_private_or_special(sa.ip()) {
            return Err(ApiError::bad_request(
                "BAD_REQUEST",
                format!(
                    "url host {host} resolves to a private/loopback/link-local address ({}) and is blocked by the proxy",
                    sa.ip()
                ),
            )
            .with_details(serde_json::json!({
                "host": host,
                "resolved": sa.ip().to_string(),
                "reason": "private_address",
            })));
        }
    }
    Ok(())
}

fn is_private_or_special(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            v4.is_loopback()                  // 127.0.0.0/8
                || v4.is_private()            // 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16
                || v4.is_link_local()         // 169.254.0.0/16 (cloud metadata!)
                || v4.is_broadcast()
                || v4.is_unspecified()        // 0.0.0.0
                || v4.is_multicast()
                || v4.octets()[0] == 100 && (v4.octets()[1] & 0xc0) == 64 // 100.64.0.0/10 carrier-grade NAT
                || v4.octets()[0] == 192 && v4.octets()[1] == 0 && v4.octets()[2] == 2 // 192.0.2.0/24 TEST-NET
                || v4.octets()[0] == 198 && v4.octets()[1] == 51 && v4.octets()[2] == 100 // 198.51.100.0/24
                || v4.octets()[0] == 203 && v4.octets()[1] == 0 && v4.octets()[2] == 113 // 203.0.113.0/24
        }
        std::net::IpAddr::V6(v6) => {
            v6.is_loopback()                  // ::1
                || v6.is_unspecified()        // ::
                || v6.is_multicast()
                || (v6.segments()[0] & 0xfe00) == 0xfc00   // fc00::/7 unique-local
                || (v6.segments()[0] & 0xffc0) == 0xfe80   // fe80::/10 link-local
                // IPv4-mapped IPv6 (::ffff:0:0/96) — unwrap and recurse.
                || v6.to_ipv4_mapped().map(|v4| is_private_or_special(std::net::IpAddr::V4(v4))).unwrap_or(false)
        }
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::http::Request;
    use std::sync::Arc;
    use tempfile::TempDir;
    use tower::ServiceExt;

    use crate::auth::sessions::Sessions;
    use crate::auth::users::{hash_password, Users};
    use crate::config::ServerConfig;
    use crate::http::{main_router, AppState};
    use crate::storage::session_bus::SessionBus;
    use crate::storage::user_data::UserData;

    fn build_state(username: &str, password: &str) -> (TempDir, AppState) {
        let dir = TempDir::new().unwrap();
        let hash = hash_password(password).unwrap();
        let users_path = dir.path().join("users.toml");
        std::fs::write(
            &users_path,
            format!("[users.{username}]\npassword_hash = \"{hash}\"\n"),
        )
        .unwrap();
        let users = Users::load(&users_path).unwrap();
        let sessions = Sessions::open(&dir.path().join("sessions")).unwrap();
        let user_data = UserData::new(dir.path().to_path_buf());
        let bus = SessionBus::new();
        let projects_root = dir.path().join("projects");
        std::fs::create_dir_all(&projects_root).unwrap();
        let cfg = ServerConfig {
            bind: "127.0.0.1".into(),
            port: 8080,
            projects_root,
            data_root: dir.path().to_path_buf(),
            legacy_19828_enabled: true,
            session_cookie_name: "test_session".into(),
        };
        let state = AppState {
            users: Arc::new(users),
            sessions,
            user_data,
            session_bus: bus,
            config: Arc::new(cfg),
            llm_client: Arc::new(crate::core::llm_client::LlmClient::new()),
        };
        (dir, state)
    }

    async fn do_login(app: axum::Router, username: &str, password: &str) -> String {
        let body = format!(r#"{{"username":"{username}","password":"{password}"}}"#);
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/auth/login")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 200, "login must succeed");
        resp.headers()
            .get(axum::http::header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string()
    }

    #[tokio::test]
    async fn proxy_raw_without_cookie_is_401() {
        let (_dir, state) = build_state("alice", "pw");
        let app = main_router(state);
        let body = serde_json::json!({
            "url": "https://httpbin.org/post",
            "method": "POST",
            "headers": { "Content-Type": "application/json" },
            "body": "{\"hello\":\"world\"}"
        })
        .to_string();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/proxy/raw")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 401);
    }

    #[tokio::test]
    async fn proxy_raw_with_auth_and_mock_upstream_streams_response() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/v1/chat/completions")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"streamed":true}"#)
            .create_async()
            .await;

        let (_dir, state) = build_state("alice", "pw");
        let app = main_router(state.clone());
        let cookie = do_login(app.clone(), "alice", "pw").await;

        let target_url = format!("{}/v1/chat/completions", server.url());
        let body = serde_json::json!({
            "url": target_url,
            "method": "POST",
            "headers": { "content-type": "application/json" },
            "body": "{\"messages\":[]}"
        })
        .to_string();

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/proxy/raw")
                    .header("content-type", "application/json")
                    .header("cookie", cookie)
                    .body(axum::body::Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let bytes = to_bytes(resp.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["streamed"], true);
    }

    #[tokio::test]
    async fn proxy_raw_preserves_upstream_non_ok_status() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/v1/embed")
            .with_status(429)
            .with_header("content-type", "application/json")
            .with_body(r#"{"error":"rate limited"}"#)
            .create_async()
            .await;

        let (_dir, state) = build_state("alice", "pw");
        let app = main_router(state.clone());
        let cookie = do_login(app.clone(), "alice", "pw").await;

        let target_url = format!("{}/v1/embed", server.url());
        let body = serde_json::json!({
            "url": target_url,
            "method": "POST",
            "headers": {},
            "body": "{}"
        })
        .to_string();

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/proxy/raw")
                    .header("content-type", "application/json")
                    .header("cookie", cookie)
                    .body(axum::body::Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 429);
        let bytes = to_bytes(resp.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["error"], "rate limited");
    }

    #[tokio::test]
    async fn proxy_raw_get_request_works() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", "/api/data")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"ok":true}"#)
            .create_async()
            .await;

        let (_dir, state) = build_state("alice", "pw");
        let app = main_router(state.clone());
        let cookie = do_login(app.clone(), "alice", "pw").await;

        let target_url = format!("{}/api/data", server.url());
        let body = serde_json::json!({
            "url": target_url,
            "method": "GET",
            "headers": {}
        })
        .to_string();

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/proxy/raw")
                    .header("content-type", "application/json")
                    .header("cookie", cookie)
                    .body(axum::body::Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let bytes = to_bytes(resp.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["ok"], true);
    }

    // Direct unit tests of the SSRF guard — the proxy_raw handler bypasses
    // it under cfg(test), but the guard logic itself needs coverage so we
    // don't regress when adding more blocked ranges later.

    #[test]
    fn ssrf_guard_blocks_loopback_v4() {
        assert!(reject_unsafe_url("http://127.0.0.1/").is_err());
        assert!(reject_unsafe_url("http://127.255.0.1:8080/foo").is_err());
    }

    #[test]
    fn ssrf_guard_blocks_loopback_v6() {
        assert!(reject_unsafe_url("http://[::1]/").is_err());
    }

    #[test]
    fn ssrf_guard_blocks_link_local_metadata() {
        // AWS / GCP / Azure metadata endpoint
        assert!(reject_unsafe_url("http://169.254.169.254/latest/meta-data/").is_err());
    }

    #[test]
    fn ssrf_guard_blocks_private_v4() {
        assert!(reject_unsafe_url("http://10.0.0.1/").is_err());
        assert!(reject_unsafe_url("http://172.16.0.1/").is_err());
        assert!(reject_unsafe_url("http://192.168.1.1/").is_err());
    }

    #[test]
    fn ssrf_guard_blocks_ipv4_mapped_v6() {
        // ::ffff:127.0.0.1 — the IPv4-mapped form of loopback
        assert!(reject_unsafe_url("http://[::ffff:127.0.0.1]/").is_err());
    }

    #[test]
    fn ssrf_guard_rejects_non_http_schemes() {
        assert!(reject_unsafe_url("file:///etc/passwd").is_err());
        assert!(reject_unsafe_url("gopher://example.com/").is_err());
    }

    #[test]
    fn ssrf_guard_rejects_malformed_url() {
        assert!(reject_unsafe_url("not a url").is_err());
        assert!(reject_unsafe_url("https:///no-host").is_err());
    }
}
