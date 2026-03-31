use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Game state types (hand-maintained from chukka-spec section 4)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MatchStatus {
    NotStarted,
    InProgress,
    PeriodBreak,
    Halftime,
    Overtime,
    Shootout,
    Completed,
    Abandoned,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Possession {
    Home,
    Away,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PossessionClockMode {
    Standard,
    Reduced,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExclusionType {
    Standard,
    ViolentAction,
    ForGame,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveExclusion {
    pub player_id: String,
    pub team_id: String,
    pub cap_number: u32,
    pub remaining_seconds: f64,
    pub exclusion_type: ExclusionType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub substitute_eligible_at: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShootoutShot {
    pub team_id: String,
    pub player_id: String,
    pub cap_number: u32,
    pub round: u32,
    pub outcome: String,
    pub home_shootout_score_after: u32,
    pub away_shootout_score_after: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShootoutState {
    pub home_score: u32,
    pub away_score: u32,
    pub current_round: u32,
    pub shots: Vec<ShootoutShot>,
    pub next_shooting_team: Possession,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameState {
    pub match_id: String,
    pub status: MatchStatus,
    pub current_period: u32,
    pub period_clock_seconds: f64,
    pub home_score: u32,
    pub away_score: u32,
    pub possession: Possession,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub possession_clock_seconds: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub possession_clock_mode: Option<PossessionClockMode>,
    pub home_timeouts_remaining: u32,
    pub away_timeouts_remaining: u32,
    pub active_exclusions: Vec<ActiveExclusion>,
    #[serde(default)]
    pub player_foul_counts: HashMap<String, u32>,
    #[serde(default)]
    pub players_excluded_for_game: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shootout_state: Option<ShootoutState>,
}

// ---------------------------------------------------------------------------
// Cloud WebSocket message envelope
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CloudMessage {
    State {
        game_state: GameState,
        #[serde(default)]
        last_event: Option<LastEvent>,
    },
    MatchInfo {
        home_team: TeamConfig,
        away_team: TeamConfig,
        rule_set: RuleSetConfig,
    },
    Ping {
        #[serde(default)]
        timestamp: Option<u64>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LastEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(default)]
    pub payload: serde_json::Value,
}

// ---------------------------------------------------------------------------
// Display state — producer output consumed by composite overlay
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayState {
    pub scorebug: OverlayVisibility,
    pub exclusions: OverlayVisibility,
    pub goal_animation: GoalAnimationState,
    pub foul_out: FoulOutState,
    pub quarter_summary: QuarterSummaryState,
    pub lower_third: LowerThirdState,
    pub possession_clock: OverlayVisibility,
    pub shootout: OverlayVisibility,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverlayVisibility {
    pub visible: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalAnimationState {
    pub visible: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scoring_team: Option<Possession>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cap_number: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub home_score: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub away_score: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FoulOutState {
    pub visible: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team: Option<Possession>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cap_number: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub foul_count: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuarterSummaryState {
    pub visible: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period_completed: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub home_score: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub away_score: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LowerThirdState {
    pub visible: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cap_number: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub player_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team: Option<Possession>,
}

impl Default for DisplayState {
    fn default() -> Self {
        Self {
            scorebug: OverlayVisibility { visible: true },
            exclusions: OverlayVisibility { visible: false },
            goal_animation: GoalAnimationState {
                visible: false,
                expires_at: None,
                scoring_team: None,
                cap_number: None,
                home_score: None,
                away_score: None,
            },
            foul_out: FoulOutState {
                visible: false,
                expires_at: None,
                team: None,
                cap_number: None,
                foul_count: None,
            },
            quarter_summary: QuarterSummaryState {
                visible: false,
                period_completed: None,
                home_score: None,
                away_score: None,
            },
            lower_third: LowerThirdState {
                visible: false,
                cap_number: None,
                player_name: None,
                team: None,
            },
            possession_clock: OverlayVisibility { visible: false },
            shootout: OverlayVisibility { visible: false },
        }
    }
}

// ---------------------------------------------------------------------------
// /config endpoint response — team branding + rule set flags
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    pub short_name: String,
    pub cap_colour: String,
    pub cap_label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logo_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleSetConfig {
    pub possession_clock_enabled: bool,
    pub foul_limit_enforced: bool,
    pub periods: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchConfig {
    pub home_team: TeamConfig,
    pub away_team: TeamConfig,
    pub rule_set: RuleSetConfig,
}

// ---------------------------------------------------------------------------
// /display WebSocket push — combines game state + display decisions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayPush {
    pub game_state: GameState,
    pub display: DisplayState,
}

// ---------------------------------------------------------------------------
// Dock state — pushed to /dock-state WebSocket
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum ConnectionStatus {
    Connected,
    Reconnecting { retry_count: u32 },
    Disconnected { error: Option<String> },
    NotConfigured,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockState {
    pub connection: ConnectionStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub match_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clock: Option<String>,
    pub overlay_url: String,
}
