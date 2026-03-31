use std::sync::Arc;
use tokio::sync::{broadcast, Notify, RwLock};

use crate::config::Config;
use crate::models::*;

/// Shared application state accessible from all tasks.
pub struct AppState {
    pub config: RwLock<Config>,
    pub game_state: RwLock<Option<GameState>>,
    pub display_state: RwLock<DisplayState>,
    pub match_config: RwLock<Option<MatchConfig>>,
    pub connection_status: RwLock<ConnectionStatus>,

    /// Broadcast full GameState to /state WebSocket subscribers.
    pub state_tx: broadcast::Sender<String>,

    /// Broadcast DisplayPush (game_state + display) to /display subscribers.
    pub display_tx: broadcast::Sender<String>,

    /// Broadcast DockState to /dock-state subscribers.
    pub dock_tx: broadcast::Sender<String>,

    /// Signal the cloud client to drop its current connection and reconnect.
    pub reconnect_signal: Notify,
}

impl AppState {
    pub fn new(config: Config) -> Arc<Self> {
        let (state_tx, _) = broadcast::channel(64);
        let (display_tx, _) = broadcast::channel(64);
        let (dock_tx, _) = broadcast::channel(16);

        let connection_status = if config.is_configured() {
            ConnectionStatus::Disconnected { error: None }
        } else {
            ConnectionStatus::NotConfigured
        };

        Arc::new(Self {
            config: RwLock::new(config),
            game_state: RwLock::new(None),
            display_state: RwLock::new(DisplayState::default()),
            match_config: RwLock::new(None),
            connection_status: RwLock::new(connection_status),
            state_tx,
            display_tx,
            dock_tx,
            reconnect_signal: Notify::new(),
        })
    }

    /// Build a DockState snapshot from current state.
    pub async fn dock_state(&self) -> DockState {
        let config = self.config.read().await;
        let conn = self.connection_status.read().await.clone();
        let gs = self.game_state.read().await;
        let mc = self.match_config.read().await;

        let (match_name, score, period, clock) = match (&*gs, &*mc) {
            (Some(gs), Some(mc)) => {
                let name = format!("{} vs {}", mc.home_team.short_name, mc.away_team.short_name);
                let score = format!("{}\u{2013}{}", gs.home_score, gs.away_score);
                let period = format_period(gs.current_period, &gs.status);
                let secs = gs.period_clock_seconds as u64;
                let clock = format!("{}:{:02}", secs / 60, secs % 60);
                (Some(name), Some(score), Some(period), Some(clock))
            }
            _ => (None, None, None, None),
        };

        DockState {
            connection: conn,
            match_name,
            score,
            period,
            clock,
            overlay_url: format!("localhost:{}/overlay/composite", config.port),
        }
    }

    /// Push current dock state to all dock WebSocket subscribers.
    pub async fn broadcast_dock(&self) {
        let ds = self.dock_state().await;

        if let Ok(json) = serde_json::to_string(&ds) {
            let _ = self.dock_tx.send(json);
        }
    }
}

fn format_period(period: u32, status: &MatchStatus) -> String {
    match status {
        MatchStatus::NotStarted => "Pre".to_string(),
        MatchStatus::Completed => "Final".to_string(),
        MatchStatus::Abandoned => "ABD".to_string(),
        MatchStatus::Halftime => "HT".to_string(),
        MatchStatus::Shootout => "SO".to_string(),
        MatchStatus::Overtime => format!("OT{}", period.saturating_sub(4)),
        MatchStatus::PeriodBreak => format!("Q{} End", period.saturating_sub(1).max(1)),
        MatchStatus::InProgress => {
            if period <= 4 {
                format!("Q{period}")
            } else {
                format!("OT{}", period - 4)
            }
        }
    }
}
