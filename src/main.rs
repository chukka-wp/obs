use tracing::info;
use tracing_subscriber::EnvFilter;

use chukka_obs::{cloud, config, display, server, state};

#[cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
fn main() {
    let cfg = config::Config::load(None::<&std::path::PathBuf>);

    // Initialise tracing.
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(&cfg.log_level));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();

    info!(port = cfg.port, "Starting chukka-obs");

    let port = cfg.port;

    tauri::Builder::default()
        .setup(move |_app| {
            let state = state::AppState::new(cfg);
            let display_engine = display::DisplayEngine::new(state.clone());

            // Start cloud WebSocket client.
            let cloud_state = state.clone();
            let cloud_display = display_engine.clone();
            tauri::async_runtime::spawn(async move {
                cloud::run(cloud_state, cloud_display).await;
            });

            // Start axum HTTP server for overlays.
            let server_state = state.clone();
            tauri::async_runtime::spawn(async move {
                let app = server::router(server_state);
                let addr = format!("127.0.0.1:{port}");

                match tokio::net::TcpListener::bind(&addr).await {
                    Ok(listener) => {
                        info!(%addr, "HTTP server listening");
                        if let Err(e) = axum::serve(listener, app).await {
                            tracing::error!(error = %e, "HTTP server error");
                        }
                    }
                    Err(e) => {
                        tracing::error!(error = %e, %addr, "Failed to bind HTTP server");
                    }
                }
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running chukka-obs");
}
