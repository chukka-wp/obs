use std::sync::Arc;

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, State,
    },
    http::{header, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use rust_embed::RustEmbed;
use serde::Deserialize;
use tower_http::cors::CorsLayer;
use tracing::{info, warn};

use crate::state::AppState;

#[derive(RustEmbed)]
#[folder = "overlay/"]
struct OverlayAssets;

/// Build the axum router with all endpoints.
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/dock", get(dock_handler))
        .route("/dock-state", get(dock_state_ws_handler))
        .route("/state", get(state_ws_handler))
        .route("/display", get(display_ws_handler))
        .route("/config", get(config_handler))
        .route("/overlay/composite", get(composite_handler))
        .route("/assets/{*file}", get(assets_handler))
        .route("/favicon.ico", get(favicon_handler))
        .route("/favicon-32.png", get(favicon_32_handler))
        .route("/connect", post(connect_handler))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Static / embedded asset handlers
// ---------------------------------------------------------------------------

async fn dock_handler() -> impl IntoResponse {
    serve_embedded("dock.html")
}

async fn composite_handler() -> impl IntoResponse {
    serve_embedded("composite.html")
}

async fn favicon_handler() -> impl IntoResponse {
    serve_embedded("favicon.ico")
}

async fn favicon_32_handler() -> impl IntoResponse {
    serve_embedded("favicon-32.png")
}

async fn assets_handler(Path(file): Path<String>) -> impl IntoResponse {
    serve_embedded(&format!("assets/{file}"))
}

fn serve_embedded(path: &str) -> impl IntoResponse {
    match OverlayAssets::get(path) {
        Some(content) => {
            let mime = mime_guess::from_path(path)
                .first_or_octet_stream()
                .to_string();

            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, mime)],
                content.data.to_vec(),
            )
                .into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

// ---------------------------------------------------------------------------
// /config — team branding JSON
// ---------------------------------------------------------------------------

async fn config_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let mc = state.match_config.read().await;

    match &*mc {
        Some(config) => Json(serde_json::to_value(config).unwrap()).into_response(),
        None => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "no match connected"})),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// POST /connect — accept token URL from dock
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ConnectRequest {
    code: Option<String>,
    url: Option<String>,
    match_id: Option<String>,
    obs_token: Option<String>,
}

enum Credentials {
    Code { match_id: String, code: String },
    Token { match_id: String, obs_token: String },
}

async fn connect_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<ConnectRequest>,
) -> impl IntoResponse {
    let api_url = {
        let config = state.config.read().await;
        config.cloud_api_url.clone()
    };

    let result = resolve_credentials(&body, &api_url).await;

    let creds = match result {
        Ok(creds) => creds,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": e.to_string()})),
            )
                .into_response();
        }
    };

    {
        let mut config = state.config.write().await;

        match &creds {
            Credentials::Code { match_id, code } => {
                info!(%match_id, %code, "Connecting via short code");
                config.match_id = Some(match_id.clone());
                config.obs_code = Some(code.clone());
                config.obs_token = None;
            }
            Credentials::Token { match_id, obs_token } => {
                info!(%match_id, "Connecting via token");
                config.match_id = Some(match_id.clone());
                config.obs_token = Some(obs_token.clone());
                config.obs_code = None;
            }
        }

        if let Err(e) = config.save() {
            warn!(error = %e, "Failed to persist config to disk");
        }
    }

    state.reconnect_signal.notify_one();

    Json(serde_json::json!({"status": "connecting"})).into_response()
}

async fn resolve_credentials(body: &ConnectRequest, api_url: &str) -> anyhow::Result<Credentials> {
    // Short code provided — call bootstrap API.
    if let Some(code) = &body.code {
        if !code.chars().all(|c| c.is_ascii_alphanumeric()) || code.len() != 6 {
            anyhow::bail!("Code must be exactly 6 alphanumeric characters");
        }

        // Build URL from trusted base + validated path segment to prevent SSRF.
        let mut bootstrap_url = reqwest::Url::parse(api_url)
            .map_err(|e| anyhow::anyhow!("invalid cloud_api_url: {e}"))?;
        bootstrap_url
            .path_segments_mut()
            .map_err(|_| anyhow::anyhow!("invalid cloud_api_url"))?
            .push("obs")
            .push("bootstrap")
            .push(code);

        let client = reqwest::Client::new();
        let resp = client
            // nosemgrep: rust.actix.ssrf.reqwest-taint.reqwest-taint
            .get(bootstrap_url) // URL built from app config + validated 6-char alphanumeric code
            .header("Accept", "application/json")
            .send()
            .await?;

        if !resp.status().is_success() {
            anyhow::bail!("Invalid or expired code");
        }

        let data: serde_json::Value = resp.json().await?;

        let match_id = data["match"]["id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing match.id in bootstrap response"))?
            .to_string();

        return Ok(Credentials::Code {
            match_id,
            code: code.clone(),
        });
    }

    // Direct credentials provided.
    if let (Some(mid), Some(tok)) = (&body.match_id, &body.obs_token) {
        return Ok(Credentials::Token {
            match_id: mid.clone(),
            obs_token: tok.clone(),
        });
    }

    // Resolve token URL.
    let url = body
        .url
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("provide code, url, or match_id + obs_token"))?;

    let client = reqwest::Client::new();
    let resp = client
        // nosemgrep: rust.actix.ssrf.reqwest-taint.reqwest-taint
        .get(url) // URL from dock control panel on localhost — intentional functionality
        .header("Accept", "application/json")
        .send()
        .await?;

    if !resp.status().is_success() {
        anyhow::bail!("token URL returned status {}", resp.status());
    }

    let data: serde_json::Value = resp.json().await?;

    let match_id = data["match_id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing match_id in token URL response"))?
        .to_string();

    let obs_token = data["obs_token"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing obs_token in token URL response"))?
        .to_string();

    Ok(Credentials::Token {
        match_id,
        obs_token,
    })
}

// ---------------------------------------------------------------------------
// WebSocket handlers
// ---------------------------------------------------------------------------

async fn state_ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_state_ws(socket, state))
}

async fn handle_state_ws(mut socket: WebSocket, state: Arc<AppState>) {
    // Send current state immediately on connect.
    if let Some(gs) = state.game_state.read().await.as_ref() {
        if let Ok(json) = serde_json::to_string(gs) {
            if socket.send(Message::Text(json.into())).await.is_err() {
                return;
            }
        }
    }

    let mut rx = state.state_tx.subscribe();

    loop {
        tokio::select! {
            result = rx.recv() => {
                match result {
                    Ok(json) => {
                        if socket.send(Message::Text(json.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!(skipped = n, "State subscriber lagged");
                    }
                    Err(_) => break,
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
        }
    }
}

async fn display_ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_display_ws(socket, state))
}

async fn handle_display_ws(mut socket: WebSocket, state: Arc<AppState>) {
    // Send current state + display immediately.
    {
        let gs = state.game_state.read().await.clone();
        let ds = state.display_state.read().await.clone();

        if let Some(game_state) = gs {
            let push = crate::models::DisplayPush {
                game_state,
                display: ds,
            };

            if let Ok(json) = serde_json::to_string(&push) {
                if socket.send(Message::Text(json.into())).await.is_err() {
                    return;
                }
            }
        }
    }

    let mut rx = state.display_tx.subscribe();

    loop {
        tokio::select! {
            result = rx.recv() => {
                match result {
                    Ok(json) => {
                        if socket.send(Message::Text(json.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!(skipped = n, "Display subscriber lagged");
                    }
                    Err(_) => break,
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
        }
    }
}

async fn dock_state_ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_dock_state_ws(socket, state))
}

async fn handle_dock_state_ws(mut socket: WebSocket, state: Arc<AppState>) {
    // Send current dock state immediately.
    let ds = state.dock_state().await;

    if let Ok(json) = serde_json::to_string(&ds) {
        if socket.send(Message::Text(json.into())).await.is_err() {
            return;
        }
    }

    let mut rx = state.dock_tx.subscribe();

    loop {
        tokio::select! {
            result = rx.recv() => {
                match result {
                    Ok(json) => {
                        if socket.send(Message::Text(json.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(_) => break,
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
        }
    }
}
