pub mod config;
pub mod jira;
pub mod reconciler;
pub mod dispatcher;
pub mod scheduler;
pub mod notifier;
pub mod context;
pub mod recovery;
pub mod events;

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
}

pub type Result<T> = std::result::Result<T, OrchestratorError>;
