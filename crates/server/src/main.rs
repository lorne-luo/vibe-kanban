use anyhow::{self, Error as AnyhowError};
use axum::Router;
use deployment::{Deployment, DeploymentError};
use server::{
    DeploymentImpl, middleware::origin::validate_origin, routes, runtime::relay_registration,
};
use services::services::container::ContainerService;
use sqlx::Error as SqlxError;
use strip_ansi_escapes::strip;
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use tower_http::validate_request::ValidateRequestHeaderLayer;
use tracing_subscriber::{EnvFilter, prelude::*};
use utils::{
    assets::asset_dir,
    port_file::write_port_file_with_proxy,
    sentry::{self as sentry_utils, SentrySource, sentry_layer},
};

// ---------------------------------------------------------------------------
// Kanban one-shot poll mode
// ---------------------------------------------------------------------------
//
// Usage: KANBAN_ONCE=1 KANBAN_PROJECT=AP JIRA_EMAIL=x@y JIRA_API_TOKEN=tok ./server
//
// If KANBAN_ONCE=1 and KANBAN_PROJECT=<slug> are set, the server will run a
// single kanban poll tick and exit instead of starting the HTTP server.

async fn run_kanban_once(project: &str) -> anyhow::Result<()> {
    use kanban_orchestrator::config::load_workflow;
    use kanban_orchestrator::dispatcher::exec_adapter::SimpleShellExecutor;
    use kanban_orchestrator::dispatcher::gate::Gate;
    use kanban_orchestrator::jira::JiraClient;
    use kanban_orchestrator::notifier::Notifier;
    use kanban_orchestrator::scheduler::tick::{OrchestratorContext, do_tick};

    let repo_root = std::env::current_dir()?;
    let workflow = load_workflow(&repo_root, project)?;

    let email_val = std::env::var(&workflow.sync.jira.auth_env.email).map_err(|_| {
        anyhow::anyhow!(
            "env var {} not set",
            workflow.sync.jira.auth_env.email
        )
    })?;
    let token_val = std::env::var(&workflow.sync.jira.auth_env.token).map_err(|_| {
        anyhow::anyhow!(
            "env var {} not set",
            workflow.sync.jira.auth_env.token
        )
    })?;

    let jira = JiraClient::new(
        format!("https://{}", workflow.sync.jira.site),
        email_val,
        token_val,
    );

    // Use the vibe-kanban data dir DB (same path as server uses at startup)
    let db_path = asset_dir().join("db.v2.sqlite");
    let pool =
        sqlx::SqlitePool::connect(&format!("sqlite://{}", db_path.display())).await?;

    // Find the project_id by name/slug
    let project_id: Option<(String,)> =
        sqlx::query_as("SELECT id FROM projects WHERE name = ? LIMIT 1")
            .bind(project)
            .fetch_optional(&pool)
            .await?;

    let project_uuid = match project_id {
        Some((id_str,)) => uuid::Uuid::parse_str(&id_str)?,
        None => {
            eprintln!("Project '{}' not found in database", project);
            std::process::exit(1);
        }
    };

    let ctx = OrchestratorContext {
        pool,
        workflow,
        repo_root,
        jira,
        gate: std::sync::Arc::new(Gate::new(3, 1)),
        executor: std::sync::Arc::new(SimpleShellExecutor::new("echo")),
        notifier: std::sync::Arc::new(Notifier::new(true)),
        project_id: project_uuid,
    };

    do_tick(&ctx).await?;
    println!("kanban poll tick complete for project '{}'", project);
    Ok(())
}

#[derive(Debug, Error)]
pub enum VibeKanbanError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Sqlx(#[from] SqlxError),
    #[error(transparent)]
    Deployment(#[from] DeploymentError),
    #[error(transparent)]
    Other(#[from] AnyhowError),
}

#[tokio::main]
async fn main() -> Result<(), VibeKanbanError> {
    // Install rustls crypto provider before any TLS operations
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

    sentry_utils::init_once(SentrySource::Backend);

    let log_level = std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string());
    let filter_string = format!(
        "warn,server={level},services={level},db={level},executors={level},deployment={level},local_deployment={level},utils={level},embedded_ssh={level},desktop_bridge={level},relay_hosts={level},relay_client={level},relay_webrtc={level},codex_core=off",
        level = log_level
    );
    let env_filter = EnvFilter::try_new(filter_string).expect("Failed to create tracing filter");
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_filter(env_filter))
        .with(sentry_layer())
        .init();

    // Kanban one-shot poll mode — run a single tick and exit
    if std::env::var("KANBAN_ONCE").as_deref() == Ok("1") {
        let project = std::env::var("KANBAN_PROJECT").unwrap_or_else(|_| "AP".into());
        return run_kanban_once(&project)
            .await
            .map_err(VibeKanbanError::Other);
    }

    // Create asset directory if it doesn't exist
    if !asset_dir().exists() {
        std::fs::create_dir_all(asset_dir())?;
    }

    // Copy old database to new location for safe downgrades
    let old_db = asset_dir().join("db.sqlite");
    let new_db = asset_dir().join("db.v2.sqlite");
    if !new_db.exists() && old_db.exists() {
        tracing::info!(
            "Copying database to new location: {:?} -> {:?}",
            old_db,
            new_db
        );
        std::fs::copy(&old_db, &new_db).expect("Failed to copy database file");
        tracing::info!("Database copy complete");
    }

    let shutdown_token = CancellationToken::new();

    let deployment = DeploymentImpl::new(shutdown_token.clone()).await?;
    deployment.update_sentry_scope().await?;
    deployment
        .container()
        .cleanup_orphan_executions()
        .await
        .map_err(DeploymentError::from)?;
    deployment
        .container()
        .backfill_before_head_commits()
        .await
        .map_err(DeploymentError::from)?;
    deployment
        .container()
        .backfill_repo_names()
        .await
        .map_err(DeploymentError::from)?;
    deployment
        .track_if_analytics_allowed("session_start", serde_json::json!({}))
        .await;
    // Preload global executor options cache for all executors with DEFAULT presets
    tokio::spawn(async move {
        executors::executors::utils::preload_global_executor_options_cache().await;
    });
    let port = std::env::var("BACKEND_PORT")
        .or_else(|_| std::env::var("PORT"))
        .ok()
        .and_then(|s| {
            // Remove any ANSI codes, then turn into String
            let cleaned =
                String::from_utf8(strip(s.as_bytes())).expect("UTF-8 after stripping ANSI");
            cleaned.trim().parse::<u16>().ok()
        })
        .unwrap_or_else(|| {
            tracing::info!("No PORT environment variable set, using port 0 for auto-assignment");
            0
        }); // Use 0 to find free port if no specific port provided

    let proxy_port = std::env::var("PREVIEW_PROXY_PORT")
        .ok()
        .and_then(|s| s.trim().parse::<u16>().ok())
        .unwrap_or(0);

    let host = std::env::var("HOST").unwrap_or_else(|_| "127.0.0.1".to_string());

    let main_listener = tokio::net::TcpListener::bind(format!("{host}:{port}")).await?;
    let actual_main_port = main_listener.local_addr()?.port();

    let proxy_listener = tokio::net::TcpListener::bind(format!("{host}:{proxy_port}")).await?;
    let actual_proxy_port = proxy_listener.local_addr()?.port();

    if let Err(e) = write_port_file_with_proxy(actual_main_port, Some(actual_proxy_port)).await {
        tracing::warn!("Failed to write port file: {}", e);
    }

    tracing::info!(
        "Main server on :{}, Preview proxy on :{}",
        actual_main_port,
        actual_proxy_port
    );

    deployment
        .client_info()
        .set_server_addr(main_listener.local_addr()?)
        .expect("client server address already set");
    deployment
        .client_info()
        .set_preview_proxy_port(actual_proxy_port)
        .expect("client preview proxy port already set");

    let app_router = routes::router(deployment.clone());

    // Production only: open browser
    if !cfg!(debug_assertions) {
        tracing::info!("Opening browser...");
        let browser_port = actual_main_port;
        tokio::spawn(async move {
            if let Err(e) =
                utils::browser::open_browser(&format!("http://127.0.0.1:{browser_port}")).await
            {
                tracing::warn!(
                    "Failed to open browser automatically: {}. Please open http://127.0.0.1:{} manually.",
                    e,
                    browser_port
                );
            }
        });
    }

    let proxy_router: Router = routes::preview::subdomain_router(deployment.clone())
        .layer(ValidateRequestHeaderLayer::custom(validate_origin));

    let main_shutdown = shutdown_token.clone();
    let proxy_shutdown = shutdown_token.clone();

    let main_server = axum::serve(main_listener, app_router)
        .with_graceful_shutdown(async move { main_shutdown.cancelled().await });
    let proxy_server = axum::serve(proxy_listener, proxy_router)
        .with_graceful_shutdown(async move { proxy_shutdown.cancelled().await });

    let main_handle = tokio::spawn(async move {
        if let Err(e) = main_server.await {
            tracing::error!("Main server error: {}", e);
        }
    });
    let proxy_handle = tokio::spawn(async move {
        if let Err(e) = proxy_server.await {
            tracing::error!("Preview proxy error: {}", e);
        }
    });

    relay_registration::spawn_relay(&deployment).await;

    tokio::select! {
        _ = shutdown_signal() => {
            tracing::info!("Shutdown signal received");
        }
        _ = main_handle => {}
        _ = proxy_handle => {}
    }

    shutdown_token.cancel();

    perform_cleanup_actions(&deployment).await;

    Ok(())
}

pub async fn shutdown_signal() {
    // Always wait for Ctrl+C
    let ctrl_c = async {
        if let Err(e) = tokio::signal::ctrl_c().await {
            tracing::error!("Failed to install Ctrl+C handler: {e}");
        }
    };

    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        // Try to install SIGTERM handler, but don't panic if it fails
        let terminate = async {
            if let Ok(mut sigterm) = signal(SignalKind::terminate()) {
                sigterm.recv().await;
            } else {
                tracing::error!("Failed to install SIGTERM handler");
                // Fallback: never resolves
                std::future::pending::<()>().await;
            }
        };

        tokio::select! {
            _ = ctrl_c => {},
            _ = terminate => {},
        }
    }

    #[cfg(not(unix))]
    {
        // Only ctrl_c is available, so just await it
        ctrl_c.await;
    }
}

pub async fn perform_cleanup_actions(deployment: &DeploymentImpl) {
    deployment
        .container()
        .kill_all_running_processes()
        .await
        .expect("Failed to cleanly kill running execution processes");
}
