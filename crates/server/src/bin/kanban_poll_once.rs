use deployment::Deployment;
use server::DeploymentImpl;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenv::dotenv();

    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .ok();

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let project = std::env::args()
        .skip(1)
        .find_map(|arg| arg.strip_prefix("--project=").map(str::to_owned));

    let project = match project {
        Some(p) if !p.is_empty() => p,
        _ => anyhow::bail!("usage: kanban_poll_once --project=AP"),
    };

    tracing::info!(project, "running one kanban poll tick");

    let deployment = DeploymentImpl::new().await?;
    kanban_orchestrator::runtime::poll_once(deployment.db(), project)
        .await
        .map_err(|e| anyhow::anyhow!("kanban poll failed: {e}"))?;

    tracing::info!("poll complete");
    Ok(())
}
