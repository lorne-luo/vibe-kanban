pub mod api;
pub mod config;
pub mod context;
pub mod dispatcher;
pub mod events;
pub mod jira;
pub mod notifier;
pub mod reconciler;
pub mod recovery;
pub mod runtime;
pub mod scheduler;
pub mod workflow_status;

#[derive(Debug, thiserror::Error)]
pub enum OrchestratorError {
    #[error("workflow load: {0}")]
    Workflow(String),
    #[error("jira: {0}")]
    Jira(String),
    #[error("db: {0}")]
    Db(#[from] sqlx::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("yaml: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("other: {0}")]
    Other(#[from] anyhow::Error),
    #[error("executor: {0}")]
    Executor(String),
    #[error("executor timeout after {0:?}")]
    ExecutorTimeout(std::time::Duration),
}

pub type Result<T> = std::result::Result<T, OrchestratorError>;
