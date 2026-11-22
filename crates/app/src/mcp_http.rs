//! The MCP server inside the GUI, over HTTP on 127.0.0.1 only ("streamable HTTP"
//! transport with plain JSON responses). Claude Code, started by the GUI, connects to it.
//!
//! Security: bound to the loopback interface, and every request must carry a random
//! bearer secret that only the GUI and the Claude Code process it starts know.

use std::hash::{BuildHasher, Hasher};
use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use dp_platform::Platform;
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::mcp::{McpServer, StopState};

/// A running in-process MCP endpoint.
#[derive(Debug)]
pub(crate) struct HttpMcp {
    pub url: String,
    pub secret: String,
    pub server: Arc<Mutex<McpServer>>,
    pub stop: StopState,
}

#[derive(Clone)]
struct AppState {
    secret: Arc<str>,
    server: Arc<Mutex<McpServer>>,
    stop: StopState,
}

/// 128 random bits as hex, from the OS-seeded hasher keys (no RNG crate needed).
fn random_secret() -> String {
    let mut out = String::new();
    for i in 0..2_u64 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(i);
        h.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        );
        h.write_u32(std::process::id());
        out.push_str(&format!("{:016x}", h.finish()));
    }
    out
}

fn json_response(status: StatusCode, body: Value) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

async fn handle_post(State(st): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let expected = format!("Bearer {}", st.secret);
    let authorized = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == expected);
    if !authorized {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let msg: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": format!("parse error: {e}")}}),
            );
        }
    };
    let messages = match msg {
        Value::Array(items) => items,
        single => vec![single],
    };
    let mut responses = Vec::new();
    for m in messages {
        if m["method"] == "notifications/cancelled" {
            // Must not wait for the busy server lock.
            st.stop.cancel_current();
            continue;
        }
        if let Some(r) = st.server.lock().await.handle(m).await {
            responses.push(r);
        }
    }
    match responses.len() {
        0 => StatusCode::ACCEPTED.into_response(),
        1 => json_response(StatusCode::OK, responses.remove(0)),
        _ => json_response(StatusCode::OK, Value::Array(responses)),
    }
}

async fn method_not_allowed() -> StatusCode {
    // We do not offer a server-initiated SSE stream (allowed by the spec).
    StatusCode::METHOD_NOT_ALLOWED
}

/// Starts the endpoint on a free loopback port.
pub(crate) async fn start(
    platform: Platform,
    options: dp_core::agent::ComputerOptions,
) -> anyhow::Result<HttpMcp> {
    let stop = StopState::default();
    let mut inner = McpServer::new(platform, options, stop.clone());
    // Other local agents (via `--mcp-bridge`) can reach this server too: only act for
    // tasks the user started in the window.
    inner.set_require_gui(true);
    let server = Arc::new(Mutex::new(inner));
    let secret = random_secret();
    let state = AppState {
        secret: Arc::from(secret.as_str()),
        server: server.clone(),
        stop: stop.clone(),
    };
    let app = Router::new()
        .route(
            "/mcp",
            post(handle_post)
                .get(method_not_allowed)
                .delete(method_not_allowed),
        )
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!("MCP HTTP server stopped: {e}");
        }
    });
    tracing::info!("in-app MCP server listening on {addr}");
    let url = format!("http://{addr}/mcp");
    write_endpoint(&url, &secret);
    Ok(HttpMcp {
        url,
        secret,
        server,
        stop,
    })
}

/// Where the window publishes its local endpoint for `--mcp-bridge` (in the user's
/// own app-data folder; deleted when the window closes).
fn endpoint_file() -> std::path::PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("DesktopPilot")
        .join("gui-endpoint.json")
}

#[derive(Debug, Clone)]
pub(crate) struct Endpoint {
    pub url: String,
    pub secret: String,
}

fn write_endpoint(url: &str, secret: &str) {
    let path = endpoint_file();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let body = serde_json::json!({"url": url, "secret": secret, "pid": std::process::id()});
    if let Err(e) = std::fs::write(&path, body.to_string()) {
        tracing::warn!("could not publish the local endpoint: {e}");
    }
}

pub(crate) fn read_endpoint() -> Option<Endpoint> {
    let text = std::fs::read_to_string(endpoint_file()).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    Some(Endpoint {
        url: v["url"].as_str()?.to_owned(),
        secret: v["secret"].as_str()?.to_owned(),
    })
}

/// Called when the window closes.
pub(crate) fn remove_endpoint() {
    if let Some(text) = std::fs::read_to_string(endpoint_file()).ok()
        && let Ok(v) = serde_json::from_str::<Value>(&text)
        && v["pid"].as_u64() == Some(u64::from(std::process::id()))
    {
        let _ = std::fs::remove_file(endpoint_file());
    }
}

#[cfg(test)]
mod tests {
    use super::random_secret;

    #[test]
    fn secrets_are_long_and_differ() {
        let a = random_secret();
        let b = random_secret();
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }
}
