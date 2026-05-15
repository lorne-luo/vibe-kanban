use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use thiserror::Error;
use ts_rs::TS;
use uuid::Uuid;

use crate::config::load_workflow_with_env_probe;

#[derive(Debug, Error)]
pub enum WorkflowStatusError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ProjectWorkflowState {
    Ready,
    NotConfigured,
    Invalid,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum RepoWorkflowState {
    Ready,
    NotConfigured,
    Invalid,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowEntryState {
    Ok,
    InvalidYaml,
    MissingAgentFile,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct WorkflowEntry {
    pub name: String,
    pub state: WorkflowEntryState,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct RepoWorkflowStatus {
    pub repo_id: Uuid,
    pub repo_name: String,
    pub workflows_dir: String,
    pub state: RepoWorkflowState,
    pub workflows: Vec<WorkflowEntry>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ProjectWorkflowStatus {
    pub state: ProjectWorkflowState,
    pub repos: Vec<RepoWorkflowStatus>,
    #[ts(type = "Date")]
    pub checked_at: DateTime<Utc>,
}

/// Compute readiness for one project. Scans every linked repo's
/// `.agents/kanban-workflows/*.yml` and aggregates the result.
///
/// The env-presence probe is stubbed to `true` so missing env vars (a C-level
/// concern) do not flip a workflow to `Invalid` here.
pub async fn compute_for_project(
    pool: &SqlitePool,
    project_id: Uuid,
) -> Result<ProjectWorkflowStatus, WorkflowStatusError> {
    let repos =
        db::models::project_repo::ProjectRepo::find_repos_for_project(pool, project_id).await?;
    let mut repo_statuses = Vec::with_capacity(repos.len());
    for repo in repos {
        repo_statuses.push(compute_for_repo(&repo).await);
    }
    let state = aggregate_project_state(&repo_statuses);
    Ok(ProjectWorkflowStatus {
        state,
        repos: repo_statuses,
        checked_at: Utc::now(),
    })
}

pub async fn compute_for_repo(repo: &db::models::repo::Repo) -> RepoWorkflowStatus {
    let workflows_dir = repo.path.join(".agents/kanban-workflows");
    let workflows_dir_str = workflows_dir.display().to_string();

    if !repo.path.exists() || !repo.path.is_dir() {
        return RepoWorkflowStatus {
            repo_id: repo.id,
            repo_name: repo.display_name.clone(),
            workflows_dir: workflows_dir_str,
            state: RepoWorkflowState::Invalid,
            workflows: Vec::new(),
            error: Some(format!("repo path inaccessible: {}", repo.path.display())),
        };
    }

    if !workflows_dir.exists() {
        return RepoWorkflowStatus {
            repo_id: repo.id,
            repo_name: repo.display_name.clone(),
            workflows_dir: workflows_dir_str,
            state: RepoWorkflowState::NotConfigured,
            workflows: Vec::new(),
            error: None,
        };
    }

    let entries = match scan_workflow_files(&workflows_dir, &repo.path) {
        Ok(v) => v,
        Err(e) => {
            return RepoWorkflowStatus {
                repo_id: repo.id,
                repo_name: repo.display_name.clone(),
                workflows_dir: workflows_dir_str,
                state: RepoWorkflowState::Invalid,
                workflows: Vec::new(),
                error: Some(e),
            };
        }
    };

    let state = if entries.is_empty() {
        RepoWorkflowState::NotConfigured
    } else if entries
        .iter()
        .all(|e| matches!(e.state, WorkflowEntryState::Ok))
    {
        RepoWorkflowState::Ready
    } else {
        RepoWorkflowState::Invalid
    };

    RepoWorkflowStatus {
        repo_id: repo.id,
        repo_name: repo.display_name.clone(),
        workflows_dir: workflows_dir_str,
        state,
        workflows: entries,
        error: None,
    }
}

fn scan_workflow_files(
    workflows_dir: &Path,
    repo_root: &Path,
) -> Result<Vec<WorkflowEntry>, String> {
    let read_dir =
        std::fs::read_dir(workflows_dir).map_err(|e| format!("read workflows dir: {}", e))?;
    let mut entries = Vec::new();
    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("yml") {
            continue;
        }
        let name = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        let result = load_workflow_with_env_probe(repo_root, &name, &|_| true);
        entries.push(match result {
            Ok(_) => WorkflowEntry {
                name,
                state: WorkflowEntryState::Ok,
                error: None,
            },
            Err(e) => {
                let msg = e.to_string();
                let state = classify_error(&msg);
                WorkflowEntry {
                    name,
                    state,
                    error: Some(msg),
                }
            }
        });
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

fn classify_error(msg: &str) -> WorkflowEntryState {
    if msg.contains("agent file") {
        WorkflowEntryState::MissingAgentFile
    } else {
        WorkflowEntryState::InvalidYaml
    }
}

pub fn aggregate_project_state(repos: &[RepoWorkflowStatus]) -> ProjectWorkflowState {
    if repos
        .iter()
        .any(|r| matches!(r.state, RepoWorkflowState::Invalid))
    {
        return ProjectWorkflowState::Invalid;
    }
    if repos
        .iter()
        .any(|r| matches!(r.state, RepoWorkflowState::Ready))
    {
        return ProjectWorkflowState::Ready;
    }
    ProjectWorkflowState::NotConfigured
}
