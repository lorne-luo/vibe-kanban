# Kanban Workflow Readiness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Surface kanban workflow agent readiness as a badge on the project list and kanban page, with an immediate validation at project creation.

**Architecture:** Backend recomputes per-project workflow status on every API call (no DB persistence). The kanban-orchestrator's existing `load_workflow` is refactored to accept an env-presence probe so the readiness service can run B-level validation (YAML schema + agent prompt file existence) without depending on env vars. A new wrapper response type `ProjectWithStatus` carries the status into the existing project endpoints.

**Tech Stack:** Rust (axum, sqlx, ts-rs, tokio), serde_yaml, React + TypeScript, lucide-react, Radix tooltip/dialog, sonner toast.

**Spec:** `docs/superpowers/specs/2026-05-15-kanban-workflow-readiness-design.md`

---

## File Structure

**Created:**
- `crates/kanban-orchestrator/src/workflow_status.rs` — types + `compute_for_project` (lives here, not in `services`, to avoid a cycle: `kanban-orchestrator` already depends on `services`)
- `crates/kanban-orchestrator/tests/workflow_status_test.rs` — service unit tests
- `frontend/src/components/projects/WorkflowStatusBadge.tsx` — UI component
- `frontend/src/components/projects/WorkflowStatusBadge.test.tsx` — vitest snapshot
- `frontend/src/components/projects/WorkflowStatusDialog.tsx` — details modal (kanban page)

**Modified:**
- `crates/kanban-orchestrator/src/config.rs` — add `load_workflow_with_env_probe`, refactor `load_workflow`
- `crates/services/src/services/mod.rs` — register new module
- `crates/services/Cargo.toml` — add `kanban-orchestrator` dependency
- `crates/server/src/routes/projects.rs` — wrap responses in `ProjectWithStatus`
- `crates/server/src/bin/generate_types.rs` — export new types
- `frontend/src/lib/api.ts` (or wherever `projectsApi` is) — update return types if needed
- `frontend/src/components/projects/ProjectCard.tsx` — render badge
- `frontend/src/pages/ProjectTasks.tsx` — header pill + Alert + dialog wiring
- `frontend/src/components/dialogs/projects/ProjectFormDialog.tsx` (or the actual create dialog file) — toast on creation result

---

## Task 1: Refactor `load_workflow` to accept an env probe

**Files:**
- Modify: `crates/kanban-orchestrator/src/config.rs:178-198`
- Test: `crates/kanban-orchestrator/tests/config_test.rs` (extend)

- [ ] **Step 1: Add new failing test for env-probe variant**

Append to `crates/kanban-orchestrator/tests/config_test.rs`:

```rust
#[test]
fn load_workflow_with_env_probe_skips_env_check() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".agents/kanban-workflows")).unwrap();
    std::fs::create_dir_all(dir.path().join(".agents/agent")).unwrap();
    let yml = std::fs::read_to_string("tests/fixtures/valid_workflow.yml").unwrap();
    std::fs::write(dir.path().join(".agents/kanban-workflows/AP.yml"), &yml).unwrap();
    for n in ["analyzer", "coder", "reviewer"] {
        std::fs::write(
            dir.path().join(format!(".agents/agent/{}.md", n)),
            "---\nname: x\n---\nbody",
        )
        .unwrap();
    }

    // env probe returning false should NOT fail when supplied explicitly.
    let w = kanban_orchestrator::config::load_workflow_with_env_probe(
        dir.path(),
        "AP",
        &|_| true,
    )
    .unwrap();
    assert_eq!(w.project, "AP");

    // env probe returning false bubbles up env-not-set error.
    let err = kanban_orchestrator::config::load_workflow_with_env_probe(
        dir.path(),
        "AP",
        &|_| false,
    )
    .unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("env var"), "got: {msg}");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p kanban-orchestrator load_workflow_with_env_probe_skips_env_check -- --nocapture`
Expected: FAIL — `load_workflow_with_env_probe` not found.

- [ ] **Step 3: Refactor `load_workflow`**

Replace `crates/kanban-orchestrator/src/config.rs:178-198` with:

```rust
pub fn load_workflow_with_env_probe(
    repo_root: &std::path::Path,
    project: &str,
    env_present: &dyn Fn(&str) -> bool,
) -> crate::Result<Workflow> {
    let path = repo_root.join(format!(".agents/kanban-workflows/{}.yml", project));
    let s = std::fs::read_to_string(&path).map_err(|e| {
        crate::OrchestratorError::Workflow(format!("read {}: {}", path.display(), e))
    })?;
    let w: Workflow = serde_yaml::from_str(&s)?;
    let agents_dir = repo_root.join(".agents/agent");
    let known: Vec<String> = std::fs::read_dir(&agents_dir)
        .map_err(|e| crate::OrchestratorError::Workflow(format!("read agents dir: {}", e)))?
        .filter_map(|r| r.ok())
        .filter_map(|e| {
            e.file_name()
                .to_str()
                .and_then(|s| s.strip_suffix(".md").map(|s| s.to_string()))
        })
        .collect();
    let known_refs: Vec<&str> = known.iter().map(|s| s.as_str()).collect();
    w.validate(&known_refs, env_present)
        .map_err(crate::OrchestratorError::Workflow)?;
    Ok(w)
}

pub fn load_workflow(repo_root: &std::path::Path, project: &str) -> crate::Result<Workflow> {
    load_workflow_with_env_probe(repo_root, project, &|n| std::env::var(n).is_ok())
}
```

- [ ] **Step 4: Run tests to verify all pass**

Run: `cargo test -p kanban-orchestrator -- --nocapture`
Expected: PASS — including the existing `loads_workflow_from_repo` and the new `load_workflow_with_env_probe_skips_env_check`.

- [ ] **Step 5: Commit**

```bash
git add crates/kanban-orchestrator/src/config.rs crates/kanban-orchestrator/tests/config_test.rs
git commit -m "refactor(kanban): expose load_workflow_with_env_probe for B-only validation"
```

---

## Task 2: Add `workflow_status` service module skeleton

**Files:**
- Create: `crates/kanban-orchestrator/src/workflow_status.rs`
- Modify: `crates/services/src/services/mod.rs`
- Modify: `crates/services/Cargo.toml`

- [ ] **Step 1: Add `kanban-orchestrator` as a dependency**

Open `crates/services/Cargo.toml`, find the `[dependencies]` table, add:

```toml
kanban-orchestrator = { path = "../kanban-orchestrator" }
```

(Alphabetize the dependency list to match existing style.)

- [ ] **Step 2: Create the module file**

Create `crates/kanban-orchestrator/src/workflow_status.rs` with:

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::path::Path;
use thiserror::Error;
use ts_rs::TS;
use uuid::Uuid;

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
    let repos = db::models::project_repo::ProjectRepo::find_repos_for_project(pool, project_id).await?;
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

pub(crate) async fn compute_for_repo(repo: &db::models::repo::Repo) -> RepoWorkflowStatus {
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

fn scan_workflow_files(workflows_dir: &Path, repo_root: &Path) -> Result<Vec<WorkflowEntry>, String> {
    let read_dir = std::fs::read_dir(workflows_dir)
        .map_err(|e| format!("read workflows dir: {}", e))?;
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
        let result = kanban_orchestrator::config::load_workflow_with_env_probe(
            repo_root,
            &name,
            &|_| true,
        );
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

pub(crate) fn aggregate_project_state(repos: &[RepoWorkflowStatus]) -> ProjectWorkflowState {
    if repos.iter().any(|r| matches!(r.state, RepoWorkflowState::Invalid)) {
        return ProjectWorkflowState::Invalid;
    }
    if repos.iter().any(|r| matches!(r.state, RepoWorkflowState::Ready)) {
        return ProjectWorkflowState::Ready;
    }
    ProjectWorkflowState::NotConfigured
}
```

- [ ] **Step 3: Register the module**

Open `crates/services/src/services/mod.rs`, add (alphabetical position, between `repo` and `workspace_manager`):

```rust
pub mod workflow_status;
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo check -p services`
Expected: success. If `db` import path is wrong, fix to match the existing pattern used elsewhere in services (`use db::models::...`).

- [ ] **Step 5: Commit**

```bash
git add crates/services/Cargo.toml crates/services/src/services/mod.rs crates/kanban-orchestrator/src/workflow_status.rs
git commit -m "feat(services): scaffold workflow_status module"
```

---

## Task 3: Service unit tests

**Files:**
- Create: `crates/kanban-orchestrator/tests/workflow_status_test.rs`

Each subtest builds a tempdir-backed `Repo` via direct struct construction (we don't need DB for `compute_for_repo`) and asserts a specific outcome.

- [ ] **Step 1: Write failing tests**

Create `crates/kanban-orchestrator/tests/workflow_status_test.rs`:

```rust
use db::models::repo::Repo;
use kanban_orchestrator::workflow_status::{
    aggregate_project_state, compute_for_repo, ProjectWorkflowState, RepoWorkflowState,
    WorkflowEntryState,
};
use std::path::{Path, PathBuf};

fn make_repo(path: PathBuf, name: &str) -> Repo {
    Repo {
        id: uuid::Uuid::new_v4(),
        path,
        name: name.to_string(),
        display_name: name.to_string(),
        setup_script: None,
        cleanup_script: None,
        archive_script: None,
        copy_files: None,
        parallel_setup_script: false,
        dev_server_script: None,
        default_target_branch: None,
        default_working_dir: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}

fn write_agents(dir: &Path, names: &[&str]) {
    std::fs::create_dir_all(dir.join(".agents/agent")).unwrap();
    for n in names {
        std::fs::write(
            dir.join(format!(".agents/agent/{}.md", n)),
            "---\nname: x\n---\nbody",
        )
        .unwrap();
    }
}

fn write_workflow(dir: &Path, name: &str, fixture: &str) {
    std::fs::create_dir_all(dir.join(".agents/kanban-workflows")).unwrap();
    std::fs::copy(
        format!("../kanban-orchestrator/tests/fixtures/{fixture}"),
        dir.join(format!(".agents/kanban-workflows/{name}.yml")),
    )
    .unwrap();
}

#[tokio::test]
async fn repo_without_agents_dir_is_not_configured() {
    let dir = tempfile::tempdir().unwrap();
    let repo = make_repo(dir.path().to_path_buf(), "r1");
    let status = compute_for_repo(&repo).await;
    assert!(matches!(status.state, RepoWorkflowState::NotConfigured));
    assert!(status.workflows.is_empty());
}

#[tokio::test]
async fn repo_with_workflows_dir_but_no_yml_is_not_configured() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".agents/kanban-workflows")).unwrap();
    let repo = make_repo(dir.path().to_path_buf(), "r1");
    let status = compute_for_repo(&repo).await;
    assert!(matches!(status.state, RepoWorkflowState::NotConfigured));
    assert!(status.workflows.is_empty());
}

#[tokio::test]
async fn nonexistent_repo_path_is_invalid() {
    let repo = make_repo(PathBuf::from("/nonexistent/__definitely_not_a_dir__"), "r1");
    let status = compute_for_repo(&repo).await;
    assert!(matches!(status.state, RepoWorkflowState::Invalid));
    assert!(status.error.as_deref().unwrap().contains("repo path inaccessible"));
}

#[tokio::test]
async fn valid_workflow_is_ready() {
    let dir = tempfile::tempdir().unwrap();
    write_agents(dir.path(), &["analyzer", "coder", "reviewer"]);
    write_workflow(dir.path(), "AP", "valid_workflow.yml");
    let repo = make_repo(dir.path().to_path_buf(), "r1");
    let status = compute_for_repo(&repo).await;
    assert!(matches!(status.state, RepoWorkflowState::Ready));
    assert_eq!(status.workflows.len(), 1);
    assert!(matches!(status.workflows[0].state, WorkflowEntryState::Ok));
}

#[tokio::test]
async fn yml_with_missing_agent_file_is_missing_agent() {
    let dir = tempfile::tempdir().unwrap();
    // only analyzer.md, but valid_workflow.yml references coder + reviewer
    write_agents(dir.path(), &["analyzer"]);
    write_workflow(dir.path(), "AP", "valid_workflow.yml");
    let repo = make_repo(dir.path().to_path_buf(), "r1");
    let status = compute_for_repo(&repo).await;
    assert!(matches!(status.state, RepoWorkflowState::Invalid));
    assert_eq!(status.workflows.len(), 1);
    assert!(matches!(
        status.workflows[0].state,
        WorkflowEntryState::MissingAgentFile
    ));
}

#[tokio::test]
async fn yml_with_dangling_next_is_invalid_yaml() {
    let dir = tempfile::tempdir().unwrap();
    write_agents(dir.path(), &["analyzer", "coder", "reviewer"]);
    write_workflow(dir.path(), "BROKEN", "dangling_next.yml");
    let repo = make_repo(dir.path().to_path_buf(), "r1");
    let status = compute_for_repo(&repo).await;
    assert!(matches!(status.state, RepoWorkflowState::Invalid));
    assert!(matches!(
        status.workflows[0].state,
        WorkflowEntryState::InvalidYaml
    ));
}

#[test]
fn aggregate_invalid_wins() {
    let make = |s: RepoWorkflowState| {
        kanban_orchestrator::workflow_status::RepoWorkflowStatus {
            repo_id: uuid::Uuid::nil(),
            repo_name: "r".into(),
            workflows_dir: "/tmp".into(),
            state: s,
            workflows: Vec::new(),
            error: None,
        }
    };
    let repos = vec![make(RepoWorkflowState::Ready), make(RepoWorkflowState::Invalid)];
    assert!(matches!(
        aggregate_project_state(&repos),
        ProjectWorkflowState::Invalid
    ));
}

#[test]
fn aggregate_all_not_configured() {
    let make = |s: RepoWorkflowState| {
        kanban_orchestrator::workflow_status::RepoWorkflowStatus {
            repo_id: uuid::Uuid::nil(),
            repo_name: "r".into(),
            workflows_dir: "/tmp".into(),
            state: s,
            workflows: Vec::new(),
            error: None,
        }
    };
    let repos = vec![
        make(RepoWorkflowState::NotConfigured),
        make(RepoWorkflowState::NotConfigured),
    ];
    assert!(matches!(
        aggregate_project_state(&repos),
        ProjectWorkflowState::NotConfigured
    ));
}
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo test -p services workflow_status`
Expected: all 8 tests pass. If `compute_for_repo` / `aggregate_project_state` are not `pub`, change `pub(crate)` to `pub` in `workflow_status.rs`.

- [ ] **Step 3: Commit**

```bash
git add crates/kanban-orchestrator/tests/workflow_status_test.rs crates/kanban-orchestrator/src/workflow_status.rs
git commit -m "test(services): workflow_status coverage for repo states and aggregation"
```

---

## Task 4: Register types with ts-rs and regenerate

**Files:**
- Modify: `crates/server/src/bin/generate_types.rs`

- [ ] **Step 1: Inspect the current exports list**

Run: `grep -n "Project\|export" crates/server/src/bin/generate_types.rs | head -30`
Note the existing pattern (likely calls to `export_to!` or `Type::export`).

- [ ] **Step 2: Add the new types**

In `crates/server/src/bin/generate_types.rs`, add exports for each of:

- `kanban_orchestrator::workflow_status::ProjectWorkflowStatus`
- `kanban_orchestrator::workflow_status::RepoWorkflowStatus`
- `kanban_orchestrator::workflow_status::WorkflowEntry`
- `kanban_orchestrator::workflow_status::ProjectWorkflowState`
- `kanban_orchestrator::workflow_status::RepoWorkflowState`
- `kanban_orchestrator::workflow_status::WorkflowEntryState`

…using the same export macro/function the file already uses (mirror an existing line, e.g. the `Project` export).

- [ ] **Step 3: Regenerate types**

Run: `pnpm run generate-types`
Expected: `shared/types.ts` updates without errors.

- [ ] **Step 4: Verify the generated file looks right**

Run: `grep -n "ProjectWorkflowStatus\|WorkflowEntry" shared/types.ts`
Expected: all six new types present.

- [ ] **Step 5: Commit**

```bash
git add crates/server/src/bin/generate_types.rs shared/types.ts
git commit -m "chore(types): export workflow_status types"
```

---

## Task 5: Wrap project responses in `ProjectWithStatus`

**Files:**
- Modify: `crates/server/src/routes/projects.rs` (get_all, get_by_id near `pub async fn`, create_project at line 83)
- Modify: `crates/server/src/bin/generate_types.rs` (add export for `ProjectWithStatus`)
- Modify: `shared/types.ts` (regenerated)

- [ ] **Step 1: Define the wrapper**

Add near the top of `crates/server/src/routes/projects.rs` (after imports):

```rust
use kanban_orchestrator::workflow_status::{self, ProjectWorkflowStatus};

#[derive(Debug, serde::Serialize, ts_rs::TS)]
pub struct ProjectWithStatus {
    #[serde(flatten)]
    pub project: Project,
    pub workflow_status: ProjectWorkflowStatus,
}
```

If imports for `services` / `ts_rs` are not present, add them. `Project` is already in scope.

- [ ] **Step 2: Add an assembler helper**

Also in `projects.rs`:

```rust
async fn with_status(
    pool: &sqlx::SqlitePool,
    project: Project,
) -> Result<ProjectWithStatus, ApiError> {
    let workflow_status = workflow_status::compute_for_project(pool, project.id)
        .await
        .map_err(|e| {
            tracing::error!("workflow status compute failed: {e}");
            ApiError::Internal
        })?;
    Ok(ProjectWithStatus { project, workflow_status })
}
```

If `ApiError::Internal` doesn't exist verbatim, use whatever variant maps to HTTP 500 in this crate (search `ApiError` definitions).

- [ ] **Step 3: Update `get_all_projects` / `get_project` handlers**

Find existing handlers (likely `get_all_projects` and a single-project getter). Change their return type from `ApiResponse<Vec<Project>>` / `ApiResponse<Project>` to `ApiResponse<Vec<ProjectWithStatus>>` / `ApiResponse<ProjectWithStatus>` and call `with_status` on each.

Example for list:

```rust
let projects = deployment.project().find_all(&deployment.db().pool).await?;
let mut results = Vec::with_capacity(projects.len());
for p in projects {
    results.push(with_status(&deployment.db().pool, p).await?);
}
Ok(ResponseJson(ApiResponse::success(results)))
```

- [ ] **Step 4: Update `create_project` handler**

At `crates/server/src/routes/projects.rs:95-108`, change the success branch to wrap the project:

```rust
Ok(project) => {
    deployment
        .track_if_analytics_allowed(
            "project_created",
            serde_json::json!({
                "project_id": project.id.to_string(),
                "repository_count": repo_count,
                "trigger": "manual",
            }),
        )
        .await;

    let response = with_status(&deployment.db().pool, project).await?;
    Ok(ResponseJson(ApiResponse::success(response)))
}
```

- [ ] **Step 5: Export wrapper type, regenerate**

In `crates/server/src/bin/generate_types.rs`, add an export for `crate::routes::projects::ProjectWithStatus` (mirror an existing route-type export). Then:

```bash
pnpm run generate-types
```

- [ ] **Step 6: Verify backend compiles + types regenerated**

Run: `cargo check -p server` and `grep -n ProjectWithStatus shared/types.ts`
Expected: both clean.

- [ ] **Step 7: Run `pnpm run frontend:check` and fix the type errors it raises in `frontend/src`**

These are the touch points the spec predicted. Most callers will need to read `.project.name` instead of `.name`, or unpack `workflow_status` from the response. Fix them mechanically — they're the entry points for later UI tasks.

Run: `pnpm run frontend:check`
Expected: PASS after fixes.

- [ ] **Step 8: Commit**

```bash
git add crates/server/src/routes/projects.rs crates/server/src/bin/generate_types.rs shared/types.ts frontend/src
git commit -m "feat(server): include workflow_status in project responses"
```

---

## Task 6: `WorkflowStatusBadge` component + snapshot test

**Files:**
- Create: `frontend/src/components/projects/WorkflowStatusBadge.tsx`
- Create: `frontend/src/components/projects/WorkflowStatusBadge.test.tsx`

- [ ] **Step 1: Implement the badge**

```tsx
import { Circle } from 'lucide-react';
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui/tooltip';
import type { ProjectWorkflowStatus } from 'shared/types';

type Variant = 'dot' | 'pill';

interface Props {
  status: ProjectWorkflowStatus;
  variant?: Variant;
  onClick?: () => void;
}

const COLOR: Record<ProjectWorkflowStatus['state'], string> = {
  ready: 'text-emerald-500',
  invalid: 'text-red-500',
  not_configured: 'text-muted-foreground',
};

const LABEL: Record<ProjectWorkflowStatus['state'], string> = {
  ready: 'Ready',
  invalid: 'Issues',
  not_configured: 'No workflow',
};

function tooltipText(status: ProjectWorkflowStatus): string {
  const total = status.repos.reduce((n, r) => n + r.workflows.length, 0);
  switch (status.state) {
    case 'ready':
      return `Workflow agent ready (${total} workflow${total === 1 ? '' : 's'})`;
    case 'invalid':
      return 'Workflow has issues — click for details';
    case 'not_configured':
      return 'No .agents/kanban-workflows/ configured';
  }
}

export function WorkflowStatusBadge({
  status,
  variant = 'dot',
  onClick,
}: Props) {
  const color = COLOR[status.state];
  const tip = tooltipText(status);
  const clickable = !!onClick;

  const content =
    variant === 'pill' ? (
      <span
        className={`inline-flex items-center gap-1.5 px-2 py-0.5 rounded-full text-xs font-medium bg-muted ${color}`}
      >
        <Circle className="h-2.5 w-2.5 fill-current" />
        {LABEL[status.state]}
      </span>
    ) : (
      <Circle className={`h-2.5 w-2.5 fill-current ${color}`} />
    );

  return (
    <TooltipProvider>
      <Tooltip>
        <TooltipTrigger asChild>
          <button
            type="button"
            aria-label={LABEL[status.state]}
            onClick={onClick}
            disabled={!clickable}
            className={clickable ? 'cursor-pointer' : 'cursor-default'}
          >
            {content}
          </button>
        </TooltipTrigger>
        <TooltipContent>{tip}</TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}
```

If `Tooltip*` primitives live at a different path in this repo, fix the import. Confirm with: `grep -rn "from '@/components/ui/tooltip'" frontend/src | head -1`.

- [ ] **Step 2: Add snapshot test**

Create `frontend/src/components/projects/WorkflowStatusBadge.test.tsx`:

```tsx
import { render } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { WorkflowStatusBadge } from './WorkflowStatusBadge';
import type { ProjectWorkflowStatus } from 'shared/types';

const base = {
  checked_at: new Date('2026-05-15T10:00:00Z'),
  repos: [],
} satisfies Partial<ProjectWorkflowStatus>;

describe('WorkflowStatusBadge', () => {
  it.each([
    ['ready', { ...base, state: 'ready' as const }],
    ['invalid', { ...base, state: 'invalid' as const }],
    ['not_configured', { ...base, state: 'not_configured' as const }],
  ])('renders %s state', (_label, status) => {
    const { container } = render(<WorkflowStatusBadge status={status as ProjectWorkflowStatus} />);
    expect(container.firstChild).toMatchSnapshot();
  });
});
```

- [ ] **Step 3: Run tests**

Run: `cd frontend && pnpm vitest run WorkflowStatusBadge`
Expected: PASS, three snapshots created.

(If vitest is not configured in this repo, skip the test file and note it in the commit message. Verify with `cat frontend/package.json | grep vitest`.)

- [ ] **Step 4: Commit**

```bash
git add frontend/src/components/projects/WorkflowStatusBadge.tsx frontend/src/components/projects/WorkflowStatusBadge.test.tsx frontend/src/components/projects/__snapshots__
git commit -m "feat(frontend): WorkflowStatusBadge component"
```

---

## Task 7: Integrate badge into project list

**Files:**
- Modify: `frontend/src/components/projects/ProjectCard.tsx`

- [ ] **Step 1: Locate the title row**

Run: `grep -n "project.name\|displayName\|CardTitle" frontend/src/components/projects/ProjectCard.tsx | head`
Confirm where the title is rendered.

- [ ] **Step 2: Render the badge next to the title**

Import at top:

```tsx
import { WorkflowStatusBadge } from '@/components/projects/WorkflowStatusBadge';
```

Wrap the existing title with a flex row, e.g.:

```tsx
<div className="flex items-center gap-2">
  <span className="truncate">{project.name}</span>
  <WorkflowStatusBadge status={project.workflow_status} />
</div>
```

Adapt to the existing markup — don't restructure the card more than necessary.

- [ ] **Step 3: Verify types + render**

Run: `pnpm run frontend:check`
Then start `pnpm run dev`, open `/local-projects`, eyeball the dot on the card. Hover → tooltip text shows.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/components/projects/ProjectCard.tsx
git commit -m "feat(frontend): workflow readiness dot on project card"
```

---

## Task 8: Workflow status details dialog

**Files:**
- Create: `frontend/src/components/projects/WorkflowStatusDialog.tsx`

- [ ] **Step 1: Create the dialog component**

```tsx
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { CheckCircle2, XCircle } from 'lucide-react';
import type {
  ProjectWorkflowStatus,
  WorkflowEntryState,
} from 'shared/types';

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  status: ProjectWorkflowStatus;
  onReload?: () => void;
}

function entryIcon(state: WorkflowEntryState) {
  return state === 'ok' ? (
    <CheckCircle2 className="h-4 w-4 text-emerald-500" />
  ) : (
    <XCircle className="h-4 w-4 text-red-500" />
  );
}

export function WorkflowStatusDialog({
  open,
  onOpenChange,
  status,
  onReload,
}: Props) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-2xl">
        <DialogHeader>
          <DialogTitle>
            Workflow status — checked at{' '}
            {new Date(status.checked_at).toLocaleTimeString()}
          </DialogTitle>
        </DialogHeader>

        <div className="space-y-4">
          {status.repos.map((r) => (
            <div key={r.repo_id} className="border rounded p-3">
              <div className="font-mono text-sm mb-1">{r.workflows_dir}</div>
              <div className="text-xs text-muted-foreground mb-2">
                repo: {r.repo_name}
              </div>
              {r.error && (
                <div className="text-red-500 text-sm mb-2">{r.error}</div>
              )}
              {r.workflows.length === 0 && !r.error && (
                <div className="text-muted-foreground text-sm italic">
                  no .yml files
                </div>
              )}
              <ul className="space-y-1">
                {r.workflows.map((w) => (
                  <li key={w.name} className="flex items-start gap-2 text-sm">
                    {entryIcon(w.state)}
                    <span className="font-medium">{w.name}</span>
                    {w.error && (
                      <span className="text-muted-foreground">— {w.error}</span>
                    )}
                  </li>
                ))}
              </ul>
            </div>
          ))}
        </div>

        {onReload && (
          <div className="mt-4 flex justify-end">
            <button
              type="button"
              className="text-sm px-3 py-1 rounded border hover:bg-muted"
              onClick={onReload}
            >
              Reload now
            </button>
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}
```

- [ ] **Step 2: Verify imports**

Run: `grep -rn "from '@/components/ui/dialog'" frontend/src | head -1`
Confirm the Dialog path matches. Fix if needed.

- [ ] **Step 3: Verify it compiles**

Run: `pnpm run frontend:check`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/components/projects/WorkflowStatusDialog.tsx
git commit -m "feat(frontend): WorkflowStatusDialog for details view"
```

---

## Task 9: Kanban page integration

**Files:**
- Modify: `frontend/src/pages/ProjectTasks.tsx`

- [ ] **Step 1: Locate the existing reconnecting Alert**

Run: `grep -n "states.reconnecting\|AlertTriangle" frontend/src/pages/ProjectTasks.tsx | head`
Note line numbers around the Alert block (was near line 900 before earlier edits).

- [ ] **Step 2: Pull workflow_status from project context**

The `useProject()` hook now returns a `ProjectWithStatus`-shaped object (its `project` field stays, plus `workflow_status`). Locate the destructure block and add `workflow_status` to it. If the hook needs adjusting, do so where it reads the API response.

Run: `grep -n "useProject\b" frontend/src/contexts/ProjectContext.tsx | head`
Adapt the context to surface `workflow_status` alongside `project`.

- [ ] **Step 3: Add pill + alert + dialog to the kanban page**

Near the top of the rendered tree in `ProjectTasks.tsx`, just above the existing `flex-1 min-h-0` container:

```tsx
import { useState } from 'react';
import { WorkflowStatusBadge } from '@/components/projects/WorkflowStatusBadge';
import { WorkflowStatusDialog } from '@/components/projects/WorkflowStatusDialog';
// ...

const [statusDialogOpen, setStatusDialogOpen] = useState(false);
```

In the JSX:

```tsx
{workflow_status && (
  <>
    <div className="flex items-center justify-end px-4 pt-2">
      <WorkflowStatusBadge
        status={workflow_status}
        variant="pill"
        onClick={() => setStatusDialogOpen(true)}
      />
    </div>
    {workflow_status.state === 'invalid' && (
      <Alert className="mx-4 my-2">
        <AlertTriangle className="h-4 w-4" />
        <AlertTitle>Workflow agent not ready</AlertTitle>
        <AlertDescription>
          {workflow_status.repos
            .flatMap((r) => r.workflows.filter((w) => w.state !== 'ok'))
            .slice(0, 1)
            .map((w) => (
              <span key={w.name}>
                {w.name}: {w.error}
              </span>
            ))}{' '}
          <button
            type="button"
            className="underline ml-2"
            onClick={() => setStatusDialogOpen(true)}
          >
            Show all
          </button>
        </AlertDescription>
      </Alert>
    )}
    <WorkflowStatusDialog
      open={statusDialogOpen}
      onOpenChange={setStatusDialogOpen}
      status={workflow_status}
      onReload={() => {
        // trigger the existing project query refetch; locate it via grep
        // e.g. queryClient.invalidateQueries({ queryKey: ['project', projectId] })
      }}
    />
  </>
)}
```

For the reload callback, find the project-fetch query and call its `refetch` (or `queryClient.invalidateQueries` if react-query is used). Confirm with:
```
grep -rn "useQuery.*project\|projectQuery" frontend/src/contexts/ProjectContext.tsx frontend/src/hooks
```

- [ ] **Step 4: Run frontend check and dev server**

Run: `pnpm run frontend:check`
Then `pnpm run dev` and verify the kanban page shows the pill + dialog opens on click. For testing the invalid case, rename your `.agents/agent/coder.md` temporarily so the AP.yml fails — confirm Alert appears, dialog lists the issue.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/pages/ProjectTasks.tsx frontend/src/contexts/ProjectContext.tsx
git commit -m "feat(frontend): show workflow readiness on kanban page"
```

---

## Task 10: Toast warning on project creation

**Files:**
- Modify: `frontend/src/components/dialogs/projects/ProjectFormDialog.tsx`

- [ ] **Step 1: Locate the create submission handler**

Run: `grep -n "projectsApi.create\|toast" frontend/src/components/dialogs/projects/ProjectFormDialog.tsx`
Note the success branch.

- [ ] **Step 2: Add the conditional warning toast**

After the existing `toast.success` (or equivalent navigation), add:

```tsx
if (created.workflow_status.state === 'invalid') {
  toast.warning('Workflow agent not ready — check .agents/kanban-workflows/', {
    action: {
      label: 'Details',
      onClick: () => navigate(`/local-projects/${created.id}`),
    },
  });
}
```

The exact `created` variable name comes from the existing handler. The toast lib import is `import { toast } from 'sonner';` if sonner is in use (confirm with `grep -n "from 'sonner'" frontend/src | head -1`).

- [ ] **Step 3: Verify**

Run: `pnpm run frontend:check`
Manually exercise: create a project pointing at a repo without `.agents/kanban-workflows/` — no toast (state = `not_configured`, not `invalid`). Then break the AP.yml in your test repo and create again — warning toast appears.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/components/dialogs/projects/ProjectFormDialog.tsx
git commit -m "feat(frontend): warn via toast when newly-created project has invalid workflow"
```

---

## Task 11: Final verification

- [ ] **Step 1: Format**

Run: `pnpm run format`

- [ ] **Step 2: Lint**

Run: `pnpm run lint`
Expected: clean. Fix any warnings introduced.

- [ ] **Step 3: Type check**

Run: `pnpm run check`
Expected: clean for both frontend and `cargo check`.

- [ ] **Step 4: Backend tests**

Run: `cargo test --workspace`
Expected: clean.

- [ ] **Step 5: Frontend tests**

Run: `cd frontend && pnpm vitest run`
Expected: clean (or "no tests found" if vitest isn't wired).

- [ ] **Step 6: End-to-end smoke**

Start `pnpm run dev`. With the existing `dev` project (`/Users/lorneluo/Workspace/swf/dev` with AP.yml):

- Open `/local-projects` → project card shows a green dot.
- Open `/local-projects/<id>/tasks` → pill in header reads "Ready", no Alert.
- Edit `.agents/kanban-workflows/AP.yml` to introduce a syntax error → switch tab/reload → pill turns red, Alert renders, dialog lists the parse error.

- [ ] **Step 7: Commit any format-only churn**

If `pnpm run format` produced diffs, commit:
```bash
git add -u
git commit -m "chore: rustfmt + prettier pass"
```

---

## Notes for Implementers

- The kanban-orchestrator runtime (`crates/kanban-orchestrator/src/runtime.rs`) still drives off `projects.default_agent_working_dir`. This feature is read-only with respect to that field — do not touch it. Aligning runtime to per-repo tick is a follow-up.
- All status computation is per request (strategy A in the spec). Do not add DB columns, caches, or file watchers.
- Error-prefix matching (`"agent file"`) is the chosen pragmatic mapping. If `Workflow::validate`'s error strings change, the test in Task 3 (`yml_with_missing_agent_file_is_missing_agent`) will catch it.
- Frontend state value casing: ts-rs with `#[serde(rename_all = "snake_case")]` emits `'ready' | 'invalid' | 'not_configured'`. Match this everywhere on the frontend.
