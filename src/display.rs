use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tracing::{debug, info};

use crate::models::*;
use crate::state::AppState;

/// The display engine is the "producer" — it owns all visibility and timing
/// decisions. The composite overlay is a dumb renderer that reads DisplayState.
pub struct DisplayEngine {
    state: Arc<AppState>,
    goal_timer: RwLock<Option<JoinHandle<()>>>,
    foul_out_timer: RwLock<Option<JoinHandle<()>>>,
}

impl DisplayEngine {
    pub fn new(state: Arc<AppState>) -> Arc<Self> {
        Arc::new(Self {
            state,
            goal_timer: RwLock::new(None),
            foul_out_timer: RwLock::new(None),
        })
    }

    /// Called every time a new GameState arrives from chukka-cloud.
    /// Compares old and new state, updates DisplayState, manages timers.
    pub async fn on_state_update(
        self: &Arc<Self>,
        old: Option<&GameState>,
        new: &GameState,
        last_event: Option<&LastEvent>,
    ) {
        let mut display = self.state.display_state.write().await;
        let match_config = self.state.match_config.read().await;
        let foul_limit_enforced = match_config
            .as_ref()
            .map(|mc| mc.rule_set.foul_limit_enforced)
            .unwrap_or(true);

        if let Some(old) = old {
            // --- Goal detection (score increased) ---
            let home_scored = new.home_score > old.home_score;
            let away_scored = new.away_score > old.away_score;

            if home_scored || away_scored {
                let scoring_team = if home_scored {
                    Possession::Home
                } else {
                    Possession::Away
                };

                // Extract scorer cap number from last_event if it's a goal.
                let cap_number = last_event
                    .filter(|e| e.event_type == "goal")
                    .and_then(|e| e.payload.get("cap_number"))
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u32);

                info!(
                    team = ?scoring_team,
                    score = format!("{}-{}", new.home_score, new.away_score),
                    cap = ?cap_number,
                    "Goal detected"
                );

                self.trigger_goal_animation(&mut display, scoring_team, new, cap_number)
                    .await;
            }

            // --- Foul-out detection (new player in excluded list) ---
            if foul_limit_enforced {
                for player_id in &new.players_excluded_for_game {
                    if !old.players_excluded_for_game.contains(player_id) {
                        let foul_count = new.player_foul_counts.get(player_id).copied();
                        let team = identify_player_team(player_id, new);
                        let cap = identify_player_cap(player_id, new);

                        info!(
                            player = %player_id,
                            fouls = ?foul_count,
                            "Foul-out detected"
                        );

                        self.trigger_foul_out(&mut display, team, cap, foul_count)
                            .await;
                    }
                }
            }

            // --- Period break / halftime → show quarter summary ---
            let entered_break =
                matches!(new.status, MatchStatus::PeriodBreak | MatchStatus::Halftime)
                    && !matches!(
                        old.status,
                        MatchStatus::PeriodBreak | MatchStatus::Halftime
                    );

            if entered_break {
                debug!(period = new.current_period, "Period break — showing quarter summary");
                self.show_quarter_summary(&mut display, new);
            }

            // --- Period start → clear quarter summary ---
            let period_started = new.status == MatchStatus::InProgress
                && matches!(
                    old.status,
                    MatchStatus::PeriodBreak
                        | MatchStatus::Halftime
                        | MatchStatus::NotStarted
                );

            if period_started {
                display.quarter_summary.visible = false;
            }
        }

        // --- Persistent overlay rules (applied every update) ---

        // Scorebug: always visible except during shootout.
        display.scorebug.visible = new.status != MatchStatus::Shootout;
        display.shootout.visible = new.status == MatchStatus::Shootout;

        // Exclusions: visible when there are active exclusions.
        display.exclusions.visible = !new.active_exclusions.is_empty();

        // Possession clock: visible when there's a value and match is in progress.
        display.possession_clock.visible =
            new.possession_clock_seconds.is_some() && new.status == MatchStatus::InProgress;

        drop(match_config);
        drop(display);

        self.broadcast().await;
    }

    // -----------------------------------------------------------------------
    // Transient overlay triggers
    // -----------------------------------------------------------------------

    async fn trigger_goal_animation(
        self: &Arc<Self>,
        display: &mut DisplayState,
        team: Possession,
        state: &GameState,
        cap_number: Option<u32>,
    ) {
        let now_ms = now_millis();

        display.goal_animation = GoalAnimationState {
            visible: true,
            expires_at: Some(now_ms + 5_000),
            scoring_team: Some(team),
            cap_number,
            home_score: Some(state.home_score),
            away_score: Some(state.away_score),
        };

        // Centre region conflict: goal animation takes priority.
        // Hide foul-out visually but keep its fields so it can be restored.
        display.foul_out.visible = false;
        display.quarter_summary.visible = false;

        // Cancel existing goal timer.
        let mut timer = self.goal_timer.write().await;
        if let Some(handle) = timer.take() {
            handle.abort();
        }

        // Spawn new 5-second timer.
        let engine = Arc::clone(self);
        *timer = Some(tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            engine.expire_goal_animation().await;
        }));
    }

    async fn expire_goal_animation(self: &Arc<Self>) {
        // Read game_state BEFORE acquiring display_state to avoid deadlock.
        // (on_state_update path: game_state write → display_state write)
        let in_break = self
            .state
            .game_state
            .read()
            .await
            .as_ref()
            .map(|gs| matches!(gs.status, MatchStatus::PeriodBreak | MatchStatus::Halftime))
            .unwrap_or(false);

        let mut display = self.state.display_state.write().await;
        display.goal_animation.visible = false;
        display.goal_animation.expires_at = None;

        // Restore foul-out if its timer is still running (higher priority than summary).
        if display.foul_out.expires_at.map_or(false, |e| e > now_millis()) {
            display.foul_out.visible = true;
        } else if in_break {
            // Restore quarter summary if we're still in a break.
            display.quarter_summary.visible = true;
        }

        drop(display);
        self.broadcast().await;
    }

    async fn trigger_foul_out(
        self: &Arc<Self>,
        display: &mut DisplayState,
        team: Option<Possession>,
        cap: Option<u32>,
        foul_count: Option<u32>,
    ) {
        let now_ms = now_millis();

        // Always record the foul-out state and start the timer, even if the
        // goal animation is currently active. expire_goal_animation will
        // restore foul_out.visible when the goal animation clears.
        let goal_active = display.goal_animation.visible;

        display.foul_out = FoulOutState {
            visible: !goal_active,
            expires_at: Some(now_ms + 6_000),
            team,
            cap_number: cap,
            foul_count,
        };

        if !goal_active {
            // Centre region conflict: foul-out > quarter summary.
            display.quarter_summary.visible = false;
        }

        // Cancel existing foul-out timer.
        let mut timer = self.foul_out_timer.write().await;
        if let Some(handle) = timer.take() {
            handle.abort();
        }

        let engine = Arc::clone(self);
        *timer = Some(tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_secs(6)).await;
            engine.expire_foul_out().await;
        }));
    }

    async fn expire_foul_out(self: &Arc<Self>) {
        // Read game_state BEFORE acquiring display_state to avoid deadlock.
        let in_break = self
            .state
            .game_state
            .read()
            .await
            .as_ref()
            .map(|gs| matches!(gs.status, MatchStatus::PeriodBreak | MatchStatus::Halftime))
            .unwrap_or(false);

        let mut display = self.state.display_state.write().await;
        display.foul_out.visible = false;
        display.foul_out.expires_at = None;

        if in_break {
            display.quarter_summary.visible = true;
        }

        drop(display);
        self.broadcast().await;
    }

    fn show_quarter_summary(&self, display: &mut DisplayState, state: &GameState) {
        // Only show if centre region is free.
        if display.goal_animation.visible || display.foul_out.visible {
            return;
        }

        display.quarter_summary = QuarterSummaryState {
            visible: true,
            period_completed: Some(state.current_period),
            home_score: Some(state.home_score),
            away_score: Some(state.away_score),
        };
    }

    // -----------------------------------------------------------------------
    // Broadcast
    // -----------------------------------------------------------------------

    async fn broadcast(&self) {
        let gs = self.state.game_state.read().await.clone();
        let ds = self.state.display_state.read().await.clone();

        if let Some(game_state) = gs {
            // /state channel — raw GameState
            if let Ok(json) = serde_json::to_string(&game_state) {
                let _ = self.state.state_tx.send(json);
            }

            // /display channel — GameState + DisplayState
            let push = DisplayPush {
                game_state,
                display: ds,
            };

            if let Ok(json) = serde_json::to_string(&push) {
                let _ = self.state.display_tx.send(json);
            }
        }

        // Dock channel
        self.state.broadcast_dock().await;
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Try to determine a player's team from active exclusions.
fn identify_player_team(player_id: &str, state: &GameState) -> Option<Possession> {
    // Check active exclusions for team info.
    for exc in &state.active_exclusions {
        if exc.player_id == player_id {
            // We don't have a direct home/away mapping from team_id here,
            // but we can compare to match metadata if available.
            // For now, return None — the overlay will still show the foul-out
            // without team-specific styling.
            return None;
        }
    }

    None
}

/// Try to determine a player's cap number from active exclusions.
fn identify_player_cap(player_id: &str, state: &GameState) -> Option<u32> {
    for exc in &state.active_exclusions {
        if exc.player_id == player_id {
            return Some(exc.cap_number);
        }
    }

    None
}
