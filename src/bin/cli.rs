use std::path::PathBuf;

use clap::Parser;
use tracing::info;
use tracing_subscriber::EnvFilter;

use chukka_obs::{cloud, config, display, server, state};

#[derive(Parser)]
#[command(name = "chukka-obs", about = "OBS bridge for chukka water polo streaming")]
struct Cli {
    /// Override config file path
    #[arg(long)]
    config: Option<PathBuf>,

    /// Override HTTP server port
    #[arg(long, short)]
    port: Option<u16>,

    /// Log level (error, warn, info, debug, trace)
    #[arg(long)]
    log_level: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Load configuration (file -> env -> CLI overrides).
    let mut cfg = config::Config::load(cli.config.as_ref());

    if let Some(port) = cli.port {
        cfg.port = port;
    }

    if let Some(ref level) = cli.log_level {
        cfg.log_level = level.clone();
    }

    // Initialise tracing.
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(&cfg.log_level));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();

    info!(port = cfg.port, "Starting chukka-obs");

    if let Some(path) = config::Config::config_path() {
        info!(path = %path.display(), "Config path");
    }

    // Shared state.
    let state = state::AppState::new(cfg.clone());
    let display_engine = display::DisplayEngine::new(state.clone());

    // Start cloud WebSocket client in background.
    let cloud_state = state.clone();
    let cloud_display = display_engine.clone();
    tokio::spawn(async move {
        cloud::run(cloud_state, cloud_display).await;
    });

    // Build and start HTTP server.
    let app = server::router(state);
    let addr = format!("127.0.0.1:{}", cfg.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    info!(%addr, "HTTP server listening");
    info!("Dock:      http://{addr}/dock");
    info!("Composite: http://{addr}/overlay/composite");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    info!("Shutting down");
    Ok(())
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to listen for ctrl+c");

    info!("Received shutdown signal");
}
