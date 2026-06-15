//! `llm-wiki-server` — the browser/LAN HTTP server entry point.

use std::net::SocketAddr;
use std::path::Path;

use llm_wiki_lib::auth::sessions::Sessions;
use llm_wiki_lib::auth::users::Users;
use llm_wiki_lib::config::ServerConfig;
use llm_wiki_lib::http::{legacy_router, main_router, AppState};
use llm_wiki_lib::storage::session_bus::SessionBus;
use llm_wiki_lib::storage::user_data::UserData;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = ServerConfig::from_env()?;

    ensure_dir(&config.data_root)?;
    ensure_dir(&config.projects_root)?;

    let users_path = config.data_root.join("users.toml");
    if !users_path.exists() {
        eprintln!(
            "no users.toml at {} — create it with at least one user before starting the server",
            users_path.display()
        );
        eprintln!("example:");
        eprintln!("  [users.alice]");
        eprintln!("  password_hash = \"<argon2 hash>\"");
        std::process::exit(2);
    }

    let users = Users::load(&users_path)?;
    let sessions = Sessions::open(&config.data_root.join("sessions"))?;
    let user_data = UserData::new(config.data_root.clone());
    let session_bus = SessionBus::new();
    let llm_client = std::sync::Arc::new(llm_wiki_lib::core::llm_client::LlmClient::new());

    let state = AppState {
        users: std::sync::Arc::new(users),
        sessions,
        user_data,
        session_bus,
        config: std::sync::Arc::new(config.clone()),
        llm_client,
    };

    // Schedule a background task to prune expired session rows from sled.
    // Lazy expiry on `lookup` covers the cookie-hot path; this catches
    // sessions that were created and never looked up again.
    let sessions_for_prune = state.sessions.clone();
    tokio::spawn(async move {
        let day = std::time::Duration::from_secs(60 * 60 * 24);
        loop {
            tokio::time::sleep(day).await;
            match sessions_for_prune.prune_expired() {
                Ok(n) if n > 0 => eprintln!("[sessions] pruned {n} expired"),
                Ok(_) => {}
                Err(e) => eprintln!("[sessions] prune failed: {e}"),
            }
        }
    });

    // Main listener: <bind>:<port> with auth.
    // Default `bind` is 127.0.0.1 (loopback only). Set LLM_WIKI_BIND=0.0.0.0
    // (or a specific LAN IP) to expose to other hosts. Opt-in by design:
    // any authenticated user can forward HTTP through /proxy/raw, so we
    // shouldn't accidentally be the LAN's confused-deputy egress.
    let main_addr: SocketAddr = format!("{}:{}", config.bind, config.port).parse()?;
    let main_listener = tokio::net::TcpListener::bind(&main_addr).await?;
    eprintln!("listening on http://{main_addr}");
    if config.bind == "127.0.0.1" {
        eprintln!("(loopback only; set LLM_WIKI_BIND=0.0.0.0 to expose to LAN)");
    }

    let main_app = main_router(state.clone());
    let main_serve = axum::serve(main_listener, main_app)
        .with_graceful_shutdown(shutdown_signal());

    // Legacy 127.0.0.1:19828 (no auth) — opt-out via config.
    if config.legacy_19828_enabled {
        let legacy_addr: SocketAddr = "127.0.0.1:19828".parse()?;
        let legacy_app = legacy_router(state.clone());
        let legacy_listener = tokio::net::TcpListener::bind(&legacy_addr).await?;
        eprintln!("legacy listener on http://{legacy_addr}");
        let legacy_serve = axum::serve(legacy_listener, legacy_app)
            .with_graceful_shutdown(shutdown_signal());
        let (a, b) = tokio::join!(main_serve, legacy_serve);
        a?;
        b?;
    } else {
        main_serve.await?;
    }

    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    eprintln!("shutdown signal received");
}

fn ensure_dir(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o700);
        std::fs::set_permissions(path, perms)?;
    }
    Ok(())
}
