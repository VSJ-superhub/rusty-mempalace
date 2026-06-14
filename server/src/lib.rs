//! HTTP server + REST API + embedded web dashboard for `yourmemory serve`.
//!
//! Secure by default: binds loopback only unless an access token exists. All data
//! access goes through `core`'s scoped query functions behind the auth middleware,
//! so the network surface shares one write path and one audit trail with the CLI/MCP.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use anyhow::{bail, Context};
use axum::{
    extract::Path,
    http::{header, StatusCode},
    middleware,
    response::{IntoResponse, Redirect, Response},
    routing::get,
    Router,
};
use rust_embed::RustEmbed;
use yourmemory_core::storage::{Palace, SqliteStorage};

pub mod api;
pub mod auth;
pub mod error;

/// Static dashboard assets, embedded into the binary at build time.
#[derive(RustEmbed)]
#[folder = "static/"]
struct Assets;

/// Shared application state. `SqliteStorage` (rusqlite) is `Send` but not `Sync`,
/// so it lives behind a `Mutex`; DB calls are synchronous and short, and the guard
/// is never held across an `.await`.
#[derive(Clone)]
pub struct AppState {
    pub storage: Arc<Mutex<SqliteStorage>>,
    /// Whether the server is bound to a loopback address. Anonymous access is only
    /// ever permitted on loopback when no tokens exist.
    pub is_loopback: bool,
    /// Path to the palace SQLite file, for reporting on-disk size in stats.
    pub db_path: std::path::PathBuf,
}

/// Default listen address — loopback only.
pub const DEFAULT_LISTEN: &str = "127.0.0.1:7700";

/// Start the dashboard server. Blocks until shutdown (Ctrl-C).
///
/// `listen` defaults to [`DEFAULT_LISTEN`]. Binding a non-loopback address requires
/// at least one access token, or this returns an error with instructions.
pub fn serve(listen: Option<&str>, open_browser: bool, palace: Option<&str>) -> anyhow::Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .try_init();

    let addr: SocketAddr = listen
        .unwrap_or(DEFAULT_LISTEN)
        .parse()
        .with_context(|| format!("invalid --listen address: {:?}", listen))?;
    let is_loopback = addr.ip().is_loopback();

    let palace = match palace {
        Some(p) => Palace::open_at(std::path::Path::new(p))
            .map_err(|e| anyhow::anyhow!("cannot open palace at {p}: {e}"))?,
        None => {
            let cwd = std::env::current_dir().context("cannot determine current directory")?;
            Palace::open(&cwd)
                .map_err(|e| anyhow::anyhow!("cannot open palace: {e} (run `yourmemory init` first)"))?
        }
    };
    let db_path = palace.root.join("palace.db");
    tracing::info!("serving palace at {}", palace.root.display());
    let storage = palace.storage;

    let token_count = storage.active_token_count()?;
    if !is_loopback && token_count == 0 {
        bail!(
            "refusing to bind {addr}: no access tokens exist.\n\
             Binding a non-loopback address would expose the palace anonymously.\n\
             Create a token first:  yourmemory token create --label <name> --grant <wing>:read\n\
             ...or bind loopback:    yourmemory serve  (defaults to {DEFAULT_LISTEN})"
        );
    }
    if is_loopback && token_count == 0 {
        tracing::warn!("no access tokens configured — loopback requests run as local admin");
    }

    let palace_desc = db_path.parent().unwrap_or(&db_path).display().to_string();
    let state = AppState { storage: Arc::new(Mutex::new(storage)), is_loopback, db_path };
    let app = build_router(state);

    let rt = tokio::runtime::Runtime::new().context("cannot start tokio runtime")?;
    rt.block_on(async move {
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .with_context(|| format!("cannot bind {addr}"))?;
        let url = format!("http://{addr}/ui");
        tracing::info!("yourmemory dashboard listening on {url}");
        println!("yourmemory dashboard: {url}  (Ctrl-C to stop)");
        println!("  palace: {palace_desc}");
        if open_browser {
            open_in_browser(&url);
        }
        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown_signal())
            .await
            .context("server error")
    })?;

    Ok(())
}

/// Build the full router. Auth + scoping apply to `/api/*` only; the `/ui` shell and
/// static assets are public (all data lives behind `/api`).
pub fn build_router(state: AppState) -> Router {
    let api = Router::new()
        .route("/api/wings", get(api::list_wings))
        .route("/api/wings/:wing_id/rooms", get(api::list_rooms))
        .route("/api/rooms/:room_id/drawers", get(api::list_drawers))
        .route("/api/drawers/:id", get(api::get_drawer))
        .route("/api/graph", get(api::graph))
        .route("/api/audit", get(api::audit))
        .route("/api/audit/export", get(api::audit_export))
        .route("/api/heatmap", get(api::heatmap))
        .route("/api/health", get(api::health))
        .route("/api/search", get(api::search))
        .route("/api/sessions", get(api::sessions))
        .route("/api/stats", get(api::stats))
        .route("/api/tokens", get(api::list_tokens).post(api::create_token))
        .route("/api/tokens/:id", axum::routing::delete(api::revoke_token))
        .route("/api/tokens/:id/grants", axum::routing::post(api::add_grant))
        .route_layer(middleware::from_fn_with_state(state.clone(), auth::require_scope));

    Router::new()
        .route("/", get(|| async { Redirect::permanent("/ui") }))
        .route("/ui", get(serve_index))
        .route("/ui/", get(serve_index))
        .route("/ui/*path", get(serve_asset))
        .merge(api)
        .with_state(state)
}

async fn serve_index() -> Response {
    asset_response("index.html")
}

async fn serve_asset(Path(path): Path<String>) -> Response {
    asset_response(&path)
}

/// Serve an embedded asset, falling back to `index.html` for unknown paths so the
/// SPA can own client-side routing.
fn asset_response(path: &str) -> Response {
    if let Some(content) = Assets::get(path) {
        let mime = mime_guess::from_path(path).first_or_octet_stream();
        return (
            [(header::CONTENT_TYPE, mime.to_string())],
            content.data.into_owned(),
        )
            .into_response();
    }
    match Assets::get("index.html") {
        Some(content) => (
            [(header::CONTENT_TYPE, "text/html; charset=utf-8".to_string())],
            content.data.into_owned(),
        )
            .into_response(),
        None => (StatusCode::NOT_FOUND, "dashboard assets not embedded").into_response(),
    }
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("shutdown signal received");
}

/// Best-effort browser open; failure is non-fatal (e.g. headless servers).
fn open_in_browser(url: &str) {
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("cmd").args(["/C", "start", "", url]).spawn();
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = std::process::Command::new("xdg-open").arg(url).spawn();

    if let Err(e) = result {
        tracing::debug!("could not open browser: {e}");
    }
}
