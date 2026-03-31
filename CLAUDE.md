# CLAUDE.md — chukka-obs

Standalone Rust binary that bridges chukka-cloud and OBS Studio. Connects to chukka-cloud via WebSocket, maintains match state locally, and serves overlay templates to OBS browser sources via a local HTTP server.

**Not** a native OBS plugin. Runs as a separate process alongside OBS.

The full technical spec is at `../obs.md` — read it before any significant work.

## Stack

- **Language:** Rust (2021 edition)
- **Async runtime:** tokio
- **HTTP server:** axum (localhost:4747)
- **WebSocket client:** tokio-tungstenite (connection to chukka-cloud)
- **Serialisation:** serde / serde_json
- **Asset embedding:** rust-embed (overlays compiled into binary)
- **CLI:** clap
- **Logging:** tracing / tracing-subscriber
- **Config:** config or figment + directories (platform-appropriate paths)

## Build & Dev Commands

```bash
cargo build              # debug build
cargo build --release    # release build
cargo test               # run tests
cargo clippy             # linting
cargo run                # run locally (serves on localhost:4747)
cargo run -- --port 4748 # override port
```

### Cross-platform release builds

```bash
# macOS universal binary
cargo build --release --target aarch64-apple-darwin
cargo build --release --target x86_64-apple-darwin
lipo -create target/aarch64-apple-darwin/release/chukka-obs target/x86_64-apple-darwin/release/chukka-obs -output chukka-obs

# Windows
cargo build --release --target x86_64-pc-windows-msvc
```

## Architecture

```
chukka-cloud (WebSocket)
      │
      ▼
chukka-obs binary
      │
      ├── /dock              → streamer control panel (OBS browser dock)
      ├── /state             → WebSocket: streams GameState to custom overlays
      ├── /display           → WebSocket: streams { game_state, display } to composite overlay
      ├── /config            → JSON: team branding, colours, logos
      ├── /overlay/composite → single composited overlay (all regions)
      ├── /assets/{file}     → shared CSS/JS (embedded)
      └── POST /connect      → accept token URL from dock
```

### Producer model

chukka-obs is the **producer** — it owns all visibility/timing decisions. The composite overlay is a dumb renderer. All show/hide logic, timer management, and conflict resolution lives in Rust. The overlay reads `DisplayState` and renders; it never decides what to show.

### DisplayState

Pushed alongside GameState to the composite overlay via `/display`. Controls visibility of all overlay regions:

- `scorebug` — always visible except during shootout
- `exclusions` — visible when active exclusions exist
- `goal_animation` — 5s transient, centre region
- `foul_out` — 6s transient, centre region
- `quarter_summary` — until next period_start, centre region
- `possession_clock` — while clock running, bottom-right
- `shootout` — replaces scorebug when status is shootout
- `lower_third` — manual trigger (v2)

Centre region shows one overlay at a time. Goal animation > foul-out > quarter summary priority.

## Key Endpoints

| Method | Path | Purpose |
|---|---|---|
| GET | `/dock` | Streamer control panel |
| GET | `/state` | WebSocket — raw GameState for custom overlays |
| GET | `/display` | WebSocket — GameState + DisplayState for composite |
| GET | `/config` | Team branding JSON |
| GET | `/overlay/composite` | Single composited overlay HTML |
| GET | `/assets/{file}` | Shared embedded assets |
| POST | `/connect` | Token URL submission from dock |

## WebSocket Connection to chukka-cloud

```
wss://chukka.app/ws/match/{match_id}?token={obs_token}
```

- Full GameState pushed on connect and on every update
- Reconnect on disconnect: 2s initial, exponential backoff to 30s max
- Single active match connection at a time

## Configuration

Platform-appropriate config paths:
- macOS: `~/Library/Application Support/chukka-obs/config.toml`
- Windows: `%APPDATA%\chukka-obs\config.toml`

Fields: `cloud_url`, `obs_token`, `match_id`, `port` (default 4747), `log_level`

## Embedded Overlays

Overlay HTML/CSS/JS lives in `overlay/` directory, embedded via `rust-embed` at compile time. Zero filesystem setup — binary is self-contained.

All overlays use CSS transitions (200ms in, 300ms out). No JS animation libraries.

## Conventions

- Rust 2021 edition, stable toolchain
- `cargo clippy` must pass with no warnings
- `cargo test` must pass before committing
- Overlay HTML/CSS/JS in `overlay/` — changes require a new binary release
- GameState and DisplayState structs are hand-maintained from chukka-spec (not generated)
- Overlays consume full state on every push — no delta/patching
- All timers (goal animation, foul-out, etc.) are managed in Rust via tokio, not in overlay JS

## Water Polo Domain Context

See the parent `../CLAUDE.md` for full domain context. Key points for obs:

- Up to 3 concurrent exclusion timers per team (20s countdown)
- Possession clock: 28s standard, 18s reduced mode
- Penalty shootout replaces scorebug entirely
- Foul-out display suppressed when `rule_set.foul_limit_enforced` is false
- Quarter summary shows players on 2 fouls (one from ejection)
