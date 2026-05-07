use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    pub version: u32,
    pub project: String,
    pub sync: Sync,
    pub columns: Vec<Column>,
    #[serde(default)]
    pub defaults: Defaults,
    #[serde(default)]
    pub reconciliation: Reconciliation,
    #[serde(default)]
    pub hooks: Hooks,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sync {
    pub source: String,
    pub jira: JiraConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JiraConfig {
    pub site: String,
    pub project_key: String,
    pub jql: String,
    #[serde(default = "default_poll", with = "humantime_serde")]
    pub poll_interval: Duration,
    pub auth_env: AuthEnv,
}

fn default_poll() -> Duration {
    Duration::from_secs(15 * 60)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthEnv {
    pub email: String,
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Column {
    pub name: String,
    #[serde(default)]
    pub jira_status: Vec<String>,
    pub initial: Option<bool>,
    pub terminal: Option<bool>,
    pub agent: Option<String>,
    #[serde(default)]
    pub on_complete: OnComplete,
    pub next: Option<String>,
    pub jira_transition: Option<String>,
    pub max_turns_per_phase: Option<u32>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OnComplete {
    Auto,
    Review,
}

impl Default for OnComplete {
    fn default() -> Self {
        OnComplete::Auto
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Defaults {
    pub workspace_strategy: Option<String>,
    pub max_concurrent_dispatches: Option<u32>,
    pub max_per_column: Option<u32>,
    pub max_turns_per_phase: Option<u32>,
    pub retry: Option<RetryConfig>,
    pub notifications: Option<NotifyConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    pub max_attempts: u32,
    pub backoff: String,
    pub base_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotifyConfig {
    pub macos: bool,
    #[serde(default)]
    pub events: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Reconciliation {
    pub on_jira_comment: Option<String>,
    pub on_jira_status_terminal: Option<String>,
    pub on_jira_assignee_change: Option<String>,
    pub on_jira_summary_change: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Hooks {
    pub pre_dispatch: Option<String>,
    pub post_complete: Option<String>,
    pub on_error: Option<String>,
}
