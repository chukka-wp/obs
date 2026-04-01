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
    url: Option<String>,
    match_id: Option<String>,
    obs_token: Option<String>,
}

async fn connect_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<ConnectRequest>,
) -> impl IntoResponse {
    let result = resolve_credentials(&body).await;

    let (match_id, obs_token) = match result {
        Ok(creds) => creds,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": e.to_string()})),
            )
                .into_response();
        }
    };

    info!(%match_id, "Updating connection credentials");

    {
        let mut config = state.config.write().await;
        config.match_id = Some(match_id);
        config.obs_token = Some(obs_token);

        if let Err(e) = config.save() {
            warn!(error = %e, "Failed to persist config to disk");
        }
    }

    state.reconnect_signal.notify_one();

    Json(serde_json::json!({"status": "connecting"})).into_response()
}

async fn resolve_credentials(body: &ConnectRequest) -> anyhow::Result<(String, String)> {
    // Direct credentials provided.
    if let (Some(mid), Some(tok)) = (&body.match_id, &body.obs_token) {
        return Ok((mid.clone(), tok.clone()));
    }

    // Resolve token URL.
    let url = body
        .url
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("provide url or match_id + obs_token"))?;

    let client = reqwest::Client::new();
    let resp = client
        .get(url)
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

    Ok((match_id, obs_token))
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
