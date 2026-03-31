use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tracing::{debug, info, warn};

use crate::display::DisplayEngine;
use crate::models::*;
use crate::state::AppState;

const INITIAL_RETRY_DELAY: Duration = Duration::from_secs(2);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(30);

/// Main loop: connect to chukka-cloud, process messages, reconnect on failure.
/// Runs for the lifetime of the application.
pub async fn run(state: Arc<AppState>, display_engine: Arc<DisplayEngine>) {
    let mut retry_delay = INITIAL_RETRY_DELAY;
    let mut retry_count = 0u32;

    loop {
        let url = {
            let config = state.config.read().await;
            match config.ws_url() {
                Some(url) => url,
                None => {
                    debug!("Not configured — waiting for token URL");
                    drop(config);

                    // Wait until reconnect_signal (i.e. /connect POST).
                    state.reconnect_signal.notified().await;
                    continue;
                }
            }
        };

        info!(%url, "Connecting to chukka-cloud");

        {
            let mut cs = state.connection_status.write().await;
            if retry_count == 0 {
                *cs = ConnectionStatus::Disconnected { error: None };
            } else {
                *cs = ConnectionStatus::Reconnecting { retry_count };
            }
        }
        state.broadcast_dock().await;

        match tokio_tungstenite::connect_async(&url).await {
            Ok((ws_stream, _response)) => {
                info!("Connected to chukka-cloud");

                {
                    let mut cs = state.connection_status.write().await;
                    *cs = ConnectionStatus::Connected;
                }
                state.broadcast_dock().await;

                retry_delay = INITIAL_RETRY_DELAY;
                retry_count = 0;

                if let Err(e) =
                    handle_stream(ws_stream, &state, &display_engine).await
                {
                    warn!(error = %e, "Cloud connection ended");
                }
            }
            Err(e) => {
                warn!(error = %e, "Failed to connect to chukka-cloud");
            }
        }

        // Disconnected — update status and wait before retrying.
        {
            let mut cs = state.connection_status.write().await;
            *cs = ConnectionStatus::Disconnected {
                error: Some("Connection lost".to_string()),
            };
        }
        state.broadcast_dock().await;

        retry_count += 1;
        info!(delay = ?retry_delay, attempt = retry_count, "Reconnecting");

        let was_signalled = tokio::select! {
            _ = tokio::time::sleep(retry_delay) => false,
            _ = state.reconnect_signal.notified() => {
                info!("Reconnect signal received — connecting immediately");
                retry_delay = INITIAL_RETRY_DELAY;
                retry_count = 0;
                true
            }
        };

        // Only increase backoff on natural timeout, not on reconnect signal.
        if !was_signalled {
            retry_delay = (retry_delay * 2).min(MAX_RETRY_DELAY);
        }
    }
}

/// Process messages from the chukka-cloud WebSocket stream.
async fn handle_stream(
    ws_stream: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    state: &Arc<AppState>,
    display_engine: &Arc<DisplayEngine>,
) -> anyhow::Result<()> {
    let (mut write, mut read) = ws_stream.split();

    loop {
        tokio::select! {
            msg = read.next() => {
                match msg {
                    Some(Ok(WsMessage::Text(text))) => {
                        handle_message(&text, state, display_engine, &mut write).await?;
                    }
                    Some(Ok(WsMessage::Ping(data))) => {
                        write.send(WsMessage::Pong(data)).await?;
                    }
                    Some(Ok(WsMessage::Close(_))) | None => {
                        info!("Cloud WebSocket closed");
                        return Ok(());
                    }
                    Some(Err(e)) => {
                        return Err(e.into());
                    }
                    _ => {}
                }
            }
            _ = state.reconnect_signal.notified() => {
                info!("Reconnect signal — dropping current connection");
                let _ = write.send(WsMessage::Close(None)).await;
                return Ok(());
            }
        }
    }
}

async fn handle_message<S>(
    text: &str,
    state: &Arc<AppState>,
    display_engine: &Arc<DisplayEngine>,
    write: &mut S,
) -> anyhow::Result<()>
where
    S: futures_util::Sink<WsMessage> + Unpin,
    S::Error: std::error::Error + Send + Sync + 'static,
{
    let msg: CloudMessage = match serde_json::from_str(text) {
        Ok(m) => m,
        Err(e) => {
            warn!(error = %e, "Failed to parse cloud message");
            return Ok(());
        }
    };

    match msg {
        CloudMessage::State {
            game_state,
            last_event,
        } => {
            debug!(
                status = ?game_state.status,
                period = game_state.current_period,
                score = format!("{}-{}", game_state.home_score, game_state.away_score),
                "State update received"
            );

            let old = {
                let mut gs = state.game_state.write().await;
                let old = gs.clone();
                *gs = Some(game_state.clone());
                old
            };

            display_engine
                .on_state_update(old.as_ref(), &game_state, last_event.as_ref())
                .await;
        }
        CloudMessage::MatchInfo {
            home_team,
            away_team,
            rule_set,
        } => {
            info!(
                home = %home_team.short_name,
                away = %away_team.short_name,
                "Match info received"
            );

            let mut mc = state.match_config.write().await;
            *mc = Some(MatchConfig {
                home_team,
                away_team,
                rule_set,
            });
            drop(mc);

            state.broadcast_dock().await;
        }
        CloudMessage::Ping { timestamp } => {
            let pong = serde_json::json!({
                "type": "pong",
                "timestamp": timestamp,
            });

            let _ = write
                .send(WsMessage::Text(pong.to_string().into()))
                .await;
        }
    }

    Ok(())
}
