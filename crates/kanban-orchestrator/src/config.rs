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

pub fn load_workflow(repo_root: &std::path::Path, project: &str) -> crate::Result<Workflow> {
    let path = repo_root.join(format!(".agents/kanban-workflows/{}.yml", project));
    let s = std::fs::read_to_string(&path).map_err(|e| {
        crate::OrchestratorError::Workflow(format!("read {}: {}", path.display(), e))
    })?;
    let w: Workflow = serde_yaml::from_str(&s)?;
    let agents_dir = repo_root.join(".agents/agent");
    let known: Vec<String> = std::fs::read_dir(&agents_dir)
        .map_err(|e| {
            crate::OrchestratorError::Workflow(format!("read agents dir: {}", e))
        })?
        .filter_map(|r| r.ok())
        .filter_map(|e| {
            e.file_name()
                .to_str()
                .and_then(|s| s.strip_suffix(".md").map(|s| s.to_string()))
        })
        .collect();
    let known_refs: Vec<&str> = known.iter().map(|s| s.as_str()).collect();
    w.validate(&known_refs, &|name| std::env::var(name).is_ok())
        .map_err(crate::OrchestratorError::Workflow)?;
    Ok(w)
}

impl Workflow {
    pub fn validate(
        &self,
        known_agents: &[&str],
        env_present: &dyn Fn(&str) -> bool,
    ) -> std::result::Result<(), String> {
        if self.version != 1 {
            return Err(format!("unsupported version {}", self.version));
        }
        let initials = self.columns.iter().filter(|c| c.initial.unwrap_or(false)).count();
        if initials != 1 {
            return Err("exactly one column must have initial: true".into());
        }
        if !self.columns.iter().any(|c| c.terminal.unwrap_or(false)) {
            return Err("at least one terminal column required".into());
        }
        let names: std::collections::HashSet<&str> =
            self.columns.iter().map(|c| c.name.as_str()).collect();
        for c in &self.columns {
            let is_terminal = c.terminal.unwrap_or(false);
            if !is_terminal {
                let next = c.next.as_deref().ok_or_else(|| {
                    format!("column '{}' missing next", c.name)
                })?;
                if !names.contains(next) {
                    return Err(format!(
                        "column '{}' next='{}' not found in columns",
                        c.name, next
                    ));
                }
            }
            if let Some(a) = &c.agent {
                if !known_agents.contains(&a.as_str()) {
                    return Err(format!(
                        "column '{}' agent file .agents/agent/{}.md missing",
                        c.name, a
                    ));
                }
            }
        }
        if !env_present(&self.sync.jira.auth_env.email) {
            return Err(format!("env var {} not set", self.sync.jira.auth_env.email));
        }
        if !env_present(&self.sync.jira.auth_env.token) {
            return Err(format!("env var {} not set", self.sync.jira.auth_env.token));
        }
        Ok(())
    }

    pub fn initial_column(&self) -> Option<&Column> {
        self.columns.iter().find(|c| c.initial.unwrap_or(false))
    }

    pub fn column(&self, name: &str) -> Option<&Column> {
        self.columns.iter().find(|c| c.name == name)
    }
}
