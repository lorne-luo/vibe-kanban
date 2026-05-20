# Kanban Orchestrator Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a Jira-driven poll + per-column-agent dispatch loop to vibe-kanban as specified in `SPEC-kanban-orchestrator.md`.

**Architecture:** New `crates/kanban-orchestrator` crate housing scheduler, Jira poller, reconciler, dispatcher, workflow loading, API helpers, and an adapter to existing notification infrastructure. Reuses existing `crates/executors`, `crates/services` (worktree/container lifecycle is in `crates/services/src/services/worktree_manager.rs` and container services; `crates/worktree-manager` was removed in the 0.1.14 reset), `crates/git`, and `crates/db`. SQLite migrations extend `tasks` and add `task_events`. New axum routes live in `crates/server/src/routes/kanban.rs` and must follow the current `Router<DeploymentImpl>` / `State<DeploymentImpl>` pattern; there is no `AppState` or `startup.rs` in this repo. Startup wiring happens from `crates/server/src/main.rs` after `DeploymentImpl::new()` and before `routes::router(...)`; local-only process dependencies are assembled by `crates/local-deployment`. `kanban poll --once` should be implemented as a dedicated server bin or deferred until a real CLI parser is introduced, because the current `server` binary has no clap subcommand parser. v1 ships locally only; `crates/remote` is out of scope.

**Tech Stack:** Rust 2024, tokio, sqlx (SQLite), axum 0.8, reqwest, serde_yaml, wiremock (tests), tracing, ts-rs.

---

## Pre-Flight

### Task 0: Recon existing structures (read-only, no commit)

**Goal:** Confirm the exact APIs the plan will integrate with. No code changes.

- [ ] **Step 1: Read existing task model**
  - File: `crates/db/src/models/task.rs`
  - Confirm: `Task` struct fields, `TaskStatus` enum variants (`Todo, InProgress, InReview, Done, Cancelled`), SqlitePool usage.

- [ ] **Step 2: Read executor entry points**
  - Files: `crates/executors/src/lib.rs`, `crates/executors/src/executors/mod.rs`, `crates/executors/src/executors/qa_mock.rs`
  - Confirm: how to invoke an agent given a worktree path + prompt, how to read stdout, how `qa_mock` simulates agents.
  - Important: `qa_mock` is behind the `qa-mode` feature (`server` exposes `qa-mode = ["services/qa-mode", "executors/qa-mode"]`). Any test that imports `QaMockExecutor` must run with `--features qa-mode` or live in a crate whose dev dependency enables `executors/qa-mode`.

- [ ] **Step 3: Locate worktree/container lifecycle API**
  - `crates/worktree-manager` no longer exists. Equivalent functionality is in:
    - `crates/services/src/services/worktree_manager.rs` for raw worktree creation/recreation helpers.
    - `crates/services/src/services/workspace_manager.rs` for workspace-level cleanup helpers.
    - `crates/services/src/services/container.rs` plus `crates/local-deployment/src/container.rs` for running execution processes.
    - `crates/git/` for Git operations.
  - Confirm the exact function or service that creates the per-task workspace/worktree before depending on it in Task A1 (`Cargo.toml`).

- [ ] **Step 4: Read server startup and route wiring**
  - Files: `crates/server/src/main.rs`, `crates/server/src/lib.rs`, `crates/server/src/routes/mod.rs`, `crates/local-deployment/src/lib.rs`, `crates/deployment/src/lib.rs`
  - Confirm: `server::DeploymentImpl` aliases `local_deployment::LocalDeployment`; `main.rs` constructs `DeploymentImpl::new()`, runs background startup work, then calls `routes::router(deployment.clone())`; routes use `Router<DeploymentImpl>` and `State<DeploymentImpl>`. There is no `crates/server/src/startup.rs` and no `crate::AppState`.

- [ ] **Step 5: Read server binary shape**
  - File: `crates/server/src/main.rs` and `crates/server/Cargo.toml`
  - Confirm: the current default `server` binary has no clap parser or subcommand dispatch. For `poll --once`, prefer a separate `src/bin/kanban_poll_once.rs` smoke-test binary unless the implementation intentionally adds a real CLI parser.

- [ ] **Step 6: Update this plan inline if recon finds API drift**
  - Do not create `RECON.md`. Keep implementation assumptions in this plan so future workers do not need a temporary side document.

---

## Phase A: Foundation

### Task A1: Create `crates/kanban-orchestrator` crate skeleton

**Files:**
- Create: `crates/kanban-orchestrator/Cargo.toml`
- Create: `crates/kanban-orchestrator/src/lib.rs`
- Modify: `Cargo.toml` (workspace `members`)

- [ ] **Step 1: Add workspace member**

In root `Cargo.toml` under `[workspace] members`:
```toml
"crates/kanban-orchestrator",
```

- [ ] **Step 2: Create `crates/kanban-orchestrator/Cargo.toml`**

```toml
[package]
name = "kanban-orchestrator"
version = "0.1.0"
edition = "2024"

[dependencies]
tokio = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
serde_yaml = "0.9"
sqlx = { workspace = true }
chrono = { workspace = true }
uuid = { workspace = true }
anyhow = { workspace = true }
thiserror = { workspace = true }
tracing = { workspace = true }
reqwest = { workspace = true }
ts-rs = { workspace = true }
db = { path = "../db" }
executors = { path = "../executors" }
services = { path = "../services" }   # replaces removed worktree-manager
git = { path = "../git" }
utils = { path = "../utils" }

[dev-dependencies]
wiremock = "0.6"
tokio = { workspace = true, features = ["test-util", "macros"] }
tempfile = "3"
```

- [ ] **Step 3: Create `crates/kanban-orchestrator/src/lib.rs`**

```rust
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
```

- [ ] **Step 4: Stub each module file**

Create `config.rs`, `jira.rs`, `reconciler.rs`, `dispatcher.rs`, `scheduler.rs`, `notifier.rs`, `context.rs`, `recovery.rs`, `events.rs`, each containing only `//! <name> module` for now.

- [ ] **Step 5: Verify build**

Run: `cargo build -p kanban-orchestrator`
Expected: PASS (empty crate compiles).

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml crates/kanban-orchestrator
git commit -m "feat(kanban): scaffold kanban-orchestrator crate"
```

---

### Task A2: DB migration — extend tasks + add task_events

**Files:**
- Create: `crates/db/migrations/20260511000000_add_kanban_orchestrator.sql`
- Modify: `crates/db/src/models/task.rs`
- Create: `crates/db/src/models/task_event.rs`
- Modify: `crates/db/src/models/mod.rs`

> **Schema conventions (v0.1.14 reset):** UUID primary keys and UUID foreign keys are stored as `BLOB` (16 bytes) — see `tasks.id`, `sessions.id`, `workspaces.id` in `20250617183714_init.sql` / `20251216142123_refactor_task_attempts_to_workspaces_sessions.sql`. Timestamps use `TEXT NOT NULL DEFAULT (datetime('now', 'subsec'))`. Status-like columns use `CHECK (col IN (...))` (see `tasks.status`, `execution_processes.status`). Follow these rules below.

- [ ] **Step 1: Write migration SQL**

```sql
-- Extend tasks for Kanban orchestrator
ALTER TABLE tasks ADD COLUMN jira_key TEXT;
ALTER TABLE tasks ADD COLUMN jira_snapshot TEXT;                    -- JSON
ALTER TABLE tasks ADD COLUMN jira_synced_at TEXT;                   -- RFC3339 / datetime('now','subsec')
ALTER TABLE tasks ADD COLUMN kanban_phase TEXT;
ALTER TABLE tasks ADD COLUMN phase_state TEXT NOT NULL DEFAULT 'idle'
    CHECK (phase_state IN ('idle','running','awaiting_review','error','archived'));
ALTER TABLE tasks ADD COLUMN current_turn INTEGER NOT NULL DEFAULT 0;
ALTER TABLE tasks ADD COLUMN last_executor_session_id BLOB
    REFERENCES sessions(id) ON DELETE SET NULL;                     -- sessions.id is BLOB
ALTER TABLE tasks ADD COLUMN review_pending_since TEXT;
ALTER TABLE tasks ADD COLUMN error_info TEXT;                       -- JSON
ALTER TABLE tasks ADD COLUMN pending_inject TEXT;                   -- JSON

CREATE UNIQUE INDEX idx_tasks_jira_key ON tasks(jira_key) WHERE jira_key IS NOT NULL;
CREATE INDEX idx_tasks_phase_state ON tasks(phase_state);
CREATE INDEX idx_tasks_last_executor_session_id
    ON tasks(last_executor_session_id)
    WHERE last_executor_session_id IS NOT NULL;

CREATE TABLE task_events (
    id           BLOB PRIMARY KEY,
    task_id      BLOB NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    event_type   TEXT NOT NULL,
    from_phase   TEXT,
    to_phase     TEXT,
    actor        TEXT NOT NULL,
    payload      TEXT,                                               -- JSON
    ts           TEXT NOT NULL DEFAULT (datetime('now', 'subsec'))
);

CREATE INDEX idx_task_events_task_id_ts ON task_events(task_id, ts DESC);
```

> **Notes:**
> - `ALTER TABLE ... ADD COLUMN ... REFERENCES` is accepted by SQLite but the FK is only enforced on new rows; that matches existing patterns in this repo.
> - `id` in `task_events` has no default — generate UUIDs in Rust (like every other table).
> - Do not add an `updated_at` trigger; the existing codebase touches `updated_at` from app code.

- [ ] **Step 2: Write the failing test**

Add to `crates/db/tests/migrations.rs` (create if absent):
```rust
#[tokio::test]
async fn kanban_columns_exist() {
    let pool = test_pool().await;
    let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM pragma_table_info('tasks') WHERE name='jira_key'")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(row.0, 1);
    let evt: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='task_events'")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(evt.0, 1);
}
```

`test_pool()` should run migrations to a `:memory:` SQLite. If a helper already exists (check `crates/db/src/lib.rs`), reuse it.

- [ ] **Step 3: Run test (expect fail)**

Run: `cargo test -p db kanban_columns_exist`
Expected: FAIL — column missing.

- [ ] **Step 4: Apply migration & re-run**

Run: `pnpm run prepare-db && cargo test -p db kanban_columns_exist`
Expected: PASS.

- [ ] **Step 5: Extend `Task` struct in `crates/db/src/models/task.rs`**

Add to `Task` struct:
```rust
pub jira_key: Option<String>,
pub jira_snapshot: Option<String>,           // JSON text
pub jira_synced_at: Option<DateTime<Utc>>,
pub kanban_phase: Option<String>,
pub phase_state: PhaseState,
pub current_turn: i64,
pub last_executor_session_id: Option<Uuid>,
pub review_pending_since: Option<DateTime<Utc>>,
pub error_info: Option<String>,              // JSON text
pub pending_inject: Option<String>,          // JSON text
```

Add new enum:
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Type, Serialize, Deserialize, TS, EnumString, Display)]
#[sqlx(rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum PhaseState {
    Idle,
    Running,
    AwaitingReview,
    Error,
    Archived,
}

impl Default for PhaseState { fn default() -> Self { Self::Idle } }
```

Update `find_all` and `find_by_id` queries to include the new columns.

- [ ] **Step 6: Create `task_event.rs`**

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use ts_rs::TS;
use uuid::Uuid;

#[derive(Debug, Clone, FromRow, Serialize, Deserialize, TS)]
pub struct TaskEvent {
    pub id: Uuid,
    pub task_id: Uuid,
    pub event_type: String,
    pub from_phase: Option<String>,
    pub to_phase: Option<String>,
    pub actor: String,
    pub payload: Option<String>,    // JSON text
    pub ts: DateTime<Utc>,
}

impl TaskEvent {
    pub async fn insert(pool: &SqlitePool, e: &TaskEvent) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "INSERT INTO task_events (id, task_id, event_type, from_phase, to_phase, actor, payload, ts) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            e.id, e.task_id, e.event_type, e.from_phase, e.to_phase, e.actor, e.payload, e.ts
        ).execute(pool).await?;
        Ok(())
    }

    pub async fn for_task(pool: &SqlitePool, task_id: Uuid) -> Result<Vec<Self>, sqlx::Error> {
        sqlx::query_as!(
            TaskEvent,
            r#"SELECT id as "id!: Uuid", task_id as "task_id!: Uuid", event_type, from_phase, to_phase, actor, payload, ts as "ts!: DateTime<Utc>"
               FROM task_events WHERE task_id = $1 ORDER BY ts DESC"#,
            task_id
        ).fetch_all(pool).await
    }
}
```

Register in `mod.rs`: `pub mod task_event;`.

- [ ] **Step 7: Run `pnpm run prepare-db` to refresh sqlx offline data**

- [ ] **Step 8: Commit**

```bash
git add crates/db
git commit -m "feat(db): add Kanban orchestrator columns and task_events"
```

---

### Task A3: Workflow YAML types

**Files:**
- Create: `crates/kanban-orchestrator/src/config.rs`
- Create: `crates/kanban-orchestrator/tests/config_test.rs`
- Create: `crates/kanban-orchestrator/tests/fixtures/valid_workflow.yml`

- [ ] **Step 1: Create fixture `tests/fixtures/valid_workflow.yml`**

(Copy the example from SPEC §5.1.)

- [ ] **Step 2: Write the failing test in `tests/config_test.rs`**

```rust
use kanban_orchestrator::config::Workflow;

#[test]
fn parses_valid_workflow() {
    let s = std::fs::read_to_string("tests/fixtures/valid_workflow.yml").unwrap();
    let w: Workflow = serde_yaml::from_str(&s).unwrap();
    assert_eq!(w.version, 1);
    assert_eq!(w.project, "AP");
    assert_eq!(w.columns.len(), 5);
    assert!(w.columns.iter().any(|c| c.initial.unwrap_or(false)));
    assert!(w.columns.iter().any(|c| c.terminal.unwrap_or(false)));
}
```

- [ ] **Step 3: Run test to confirm fail**

Run: `cargo test -p kanban-orchestrator parses_valid_workflow`
Expected: FAIL (Workflow type missing).

- [ ] **Step 4: Implement types in `src/config.rs`**

```rust
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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
    pub source: String,           // "jira" in v1
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

fn default_poll() -> Duration { Duration::from_secs(15 * 60) }

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
pub enum OnComplete { Auto, Review }
impl Default for OnComplete { fn default() -> Self { OnComplete::Auto } }

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
    pub backoff: String,    // "exponential"
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
```

Add `humantime-serde = "1"` to Cargo.toml dependencies.

- [ ] **Step 5: Re-run test**

Run: `cargo test -p kanban-orchestrator parses_valid_workflow`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/kanban-orchestrator
git commit -m "feat(kanban): workflow YAML types"
```

---

### Task A4: Workflow validator

**Files:**
- Modify: `crates/kanban-orchestrator/src/config.rs`
- Modify: `crates/kanban-orchestrator/tests/config_test.rs`
- Create: `crates/kanban-orchestrator/tests/fixtures/missing_initial.yml`
- Create: `crates/kanban-orchestrator/tests/fixtures/dangling_next.yml`

- [ ] **Step 1: Add fixtures**

`missing_initial.yml`: copy valid_workflow.yml but remove `initial: true` from Todo.
`dangling_next.yml`: change `next: Coding` on Analyzing to `next: NotAColumn`.

- [ ] **Step 2: Write failing tests**

```rust
#[test]
fn rejects_missing_initial() {
    let s = std::fs::read_to_string("tests/fixtures/missing_initial.yml").unwrap();
    let w: Workflow = serde_yaml::from_str(&s).unwrap();
    let err = w.validate(&["analyzer", "coder", "reviewer"], &|_| true).unwrap_err();
    assert!(err.contains("initial"));
}

#[test]
fn rejects_dangling_next() {
    let s = std::fs::read_to_string("tests/fixtures/dangling_next.yml").unwrap();
    let w: Workflow = serde_yaml::from_str(&s).unwrap();
    let err = w.validate(&["analyzer", "coder", "reviewer"], &|_| true).unwrap_err();
    assert!(err.contains("next"));
}

#[test]
fn rejects_missing_agent_file() {
    let s = std::fs::read_to_string("tests/fixtures/valid_workflow.yml").unwrap();
    let w: Workflow = serde_yaml::from_str(&s).unwrap();
    let err = w.validate(&["analyzer"], &|_| true).unwrap_err();   // coder/reviewer missing
    assert!(err.contains("agent"));
}
```

- [ ] **Step 3: Run tests (expect fail)**

- [ ] **Step 4: Implement `validate` on `Workflow`**

```rust
impl Workflow {
    pub fn validate(
        &self,
        known_agents: &[&str],
        env_present: &dyn Fn(&str) -> bool,
    ) -> std::result::Result<(), String> {
        if self.version != 1 { return Err(format!("unsupported version {}", self.version)); }
        let initials = self.columns.iter().filter(|c| c.initial.unwrap_or(false)).count();
        if initials != 1 { return Err("exactly one column must have initial: true".into()); }
        if !self.columns.iter().any(|c| c.terminal.unwrap_or(false)) {
            return Err("at least one terminal column required".into());
        }
        let names: std::collections::HashSet<_> = self.columns.iter().map(|c| c.name.as_str()).collect();
        for c in &self.columns {
            let is_terminal = c.terminal.unwrap_or(false);
            if !is_terminal {
                let next = c.next.as_deref().ok_or_else(|| format!("column {} missing next", c.name))?;
                if !names.contains(next) {
                    return Err(format!("column {} next={} not found", c.name, next));
                }
            }
            if let Some(a) = &c.agent {
                if !known_agents.contains(&a.as_str()) {
                    return Err(format!("column {} agent file .agents/agent/{}.md missing", c.name, a));
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
```

- [ ] **Step 5: Run tests (expect pass)**

- [ ] **Step 6: Commit**

```bash
git commit -am "feat(kanban): workflow validator with fail-fast checks"
```

---

### Task A5: Loader (workflow.yml + agent frontmatter discovery)

**Files:**
- Modify: `crates/kanban-orchestrator/src/config.rs`
- Modify: `crates/kanban-orchestrator/tests/config_test.rs`

- [ ] **Step 1: Failing test**

```rust
#[test]
fn loads_workflow_from_repo() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".agents/kanban-workflows")).unwrap();
    std::fs::create_dir_all(dir.path().join(".agents/agent")).unwrap();
    let yml = std::fs::read_to_string("tests/fixtures/valid_workflow.yml").unwrap();
    std::fs::write(dir.path().join(".agents/kanban-workflows/AP.yml"), yml).unwrap();
    for n in ["analyzer", "coder", "reviewer"] {
        std::fs::write(
            dir.path().join(format!(".agents/agent/{}.md", n)),
            "---\nname: x\n---\nbody",
        ).unwrap();
    }
    std::env::set_var("JIRA_EMAIL", "x");
    std::env::set_var("JIRA_API_TOKEN", "y");
    let w = kanban_orchestrator::config::load_workflow(dir.path(), "AP").unwrap();
    assert_eq!(w.project, "AP");
}
```

- [ ] **Step 2: Implement `load_workflow`**

```rust
pub fn load_workflow(repo_root: &std::path::Path, project: &str) -> crate::Result<Workflow> {
    let path = repo_root.join(format!(".agents/kanban-workflows/{}.yml", project));
    let s = std::fs::read_to_string(&path)
        .map_err(|e| crate::OrchestratorError::Workflow(format!("read {}: {}", path.display(), e)))?;
    let w: Workflow = serde_yaml::from_str(&s)?;
    let agents_dir = repo_root.join(".agents/agent");
    let known: Vec<String> = std::fs::read_dir(&agents_dir)
        .map_err(|e| crate::OrchestratorError::Workflow(format!("read agents dir: {}", e)))?
        .filter_map(|r| r.ok())
        .filter_map(|e| e.file_name().to_str()
            .and_then(|s| s.strip_suffix(".md").map(|s| s.to_string())))
        .collect();
    let known_refs: Vec<&str> = known.iter().map(|s| s.as_str()).collect();
    w.validate(&known_refs, &|name| std::env::var(name).is_ok())
        .map_err(crate::OrchestratorError::Workflow)?;
    Ok(w)
}
```

- [ ] **Step 3: Test passes**

- [ ] **Step 4: Commit**

```bash
git commit -am "feat(kanban): workflow loader with agent file + env validation"
```

---

## Phase B: Jira Client

### Task B1: Jira REST client (search + transition)

**Files:**
- Create: `crates/kanban-orchestrator/src/jira.rs`
- Create: `crates/kanban-orchestrator/tests/jira_test.rs`

- [ ] **Step 1: Failing test using wiremock**

```rust
use kanban_orchestrator::jira::{JiraClient, JiraIssue};
use wiremock::{matchers::*, Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn search_returns_issues() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/3/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "issues": [{
                "key": "AP-1",
                "fields": {
                    "summary": "Hello",
                    "status": { "name": "To Do" },
                    "assignee": null,
                    "comment": { "comments": [] },
                    "description": null,
                    "labels": [],
                    "priority": null,
                    "attachment": []
                }
            }]
        })))
        .mount(&server).await;

    let client = JiraClient::new(server.uri(), "u@x".into(), "tok".into());
    let issues = client.search("project=AP").await.unwrap();
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].key, "AP-1");
}
```

- [ ] **Step 2: Run (fails)**

- [ ] **Step 3: Implement `JiraClient` in `src/jira.rs`**

```rust
use serde::{Deserialize, Serialize};

pub struct JiraClient {
    base: String,
    email: String,
    token: String,
    http: reqwest::Client,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JiraIssue {
    pub key: String,
    pub fields: JiraFields,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JiraFields {
    pub summary: String,
    pub status: JiraNamed,
    #[serde(default)]
    pub assignee: Option<JiraUser>,
    #[serde(default)]
    pub description: Option<serde_json::Value>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub priority: Option<JiraNamed>,
    #[serde(default)]
    pub comment: JiraComments,
    #[serde(default)]
    pub attachment: Vec<JiraAttachment>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JiraNamed { pub name: String }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JiraUser {
    #[serde(rename = "accountId")] pub account_id: String,
    #[serde(rename = "displayName")] pub display_name: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct JiraComments {
    #[serde(default)]
    pub comments: Vec<JiraComment>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JiraComment {
    pub id: String,
    pub body: serde_json::Value,
    #[serde(rename = "updated")] pub updated: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JiraAttachment {
    pub id: String,
    pub filename: String,
}

impl JiraClient {
    pub fn new(base: String, email: String, token: String) -> Self {
        Self { base, email, token, http: reqwest::Client::new() }
    }

    pub async fn search(&self, jql: &str) -> crate::Result<Vec<JiraIssue>> {
        #[derive(Deserialize)]
        struct Resp { issues: Vec<JiraIssue> }
        let url = format!("{}/rest/api/3/search", self.base);
        let resp: Resp = self.http.get(&url)
            .basic_auth(&self.email, Some(&self.token))
            .query(&[("jql", jql), ("maxResults", "200")])
            .send().await
            .map_err(|e| crate::OrchestratorError::Jira(e.to_string()))?
            .error_for_status()
            .map_err(|e| crate::OrchestratorError::Jira(e.to_string()))?
            .json().await
            .map_err(|e| crate::OrchestratorError::Jira(e.to_string()))?;
        Ok(resp.issues)
    }

    pub async fn transition(&self, key: &str, transition_name: &str) -> crate::Result<()> {
        // 1) GET /rest/api/3/issue/{key}/transitions to find id by name
        // 2) POST /rest/api/3/issue/{key}/transitions with {"transition":{"id":...}}
        let list_url = format!("{}/rest/api/3/issue/{}/transitions", self.base, key);
        #[derive(Deserialize)]
        struct TList { transitions: Vec<TItem> }
        #[derive(Deserialize)]
        struct TItem { id: String, name: String }
        let list: TList = self.http.get(&list_url)
            .basic_auth(&self.email, Some(&self.token))
            .send().await.map_err(|e| crate::OrchestratorError::Jira(e.to_string()))?
            .error_for_status().map_err(|e| crate::OrchestratorError::Jira(e.to_string()))?
            .json().await.map_err(|e| crate::OrchestratorError::Jira(e.to_string()))?;
        let id = list.transitions.iter().find(|t| t.name == transition_name)
            .ok_or_else(|| crate::OrchestratorError::Jira(format!("transition '{}' not found", transition_name)))?
            .id.clone();
        self.http.post(&list_url)
            .basic_auth(&self.email, Some(&self.token))
            .json(&serde_json::json!({"transition":{"id":id}}))
            .send().await.map_err(|e| crate::OrchestratorError::Jira(e.to_string()))?
            .error_for_status().map_err(|e| crate::OrchestratorError::Jira(e.to_string()))?;
        Ok(())
    }
}
```

- [ ] **Step 4: Test passes**

- [ ] **Step 5: Add transition test (mock both endpoints)**

```rust
#[tokio::test]
async fn transition_resolves_id_then_posts() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/rest/api/3/issue/AP-1/transitions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "transitions":[{"id":"31","name":"Done"}]
        }))).mount(&server).await;
    Mock::given(method("POST")).and(path("/rest/api/3/issue/AP-1/transitions"))
        .respond_with(ResponseTemplate::new(204)).expect(1).mount(&server).await;
    let c = JiraClient::new(server.uri(), "e".into(), "t".into());
    c.transition("AP-1", "Done").await.unwrap();
}
```

- [ ] **Step 6: Commit**

```bash
git commit -am "feat(kanban): Jira REST client (search + transition)"
```

---

### Task B2: Diff snapshot algorithm

**Files:**
- Modify: `crates/kanban-orchestrator/src/jira.rs`
- Modify: `crates/kanban-orchestrator/tests/jira_test.rs`

- [ ] **Step 1: Failing test**

```rust
use kanban_orchestrator::jira::{diff_issue, JiraDiff};

#[test]
fn diff_detects_new_comment_and_status() {
    let mut a = sample_issue();
    let mut b = a.clone();
    b.fields.status.name = "Done".into();
    b.fields.comment.comments.push(sample_comment("c2"));
    let d = diff_issue(&a, &b);
    assert!(d.status_changed);
    assert_eq!(d.new_comments.len(), 1);
    assert_eq!(d.new_comments[0].id, "c2");
    assert!(!d.summary_changed);
    assert!(!d.assignee_changed);
}
```

(Helpers `sample_issue` and `sample_comment` are simple fixtures; include them in the test file.)

- [ ] **Step 2: Implement**

```rust
#[derive(Debug, Default, Clone)]
pub struct JiraDiff {
    pub status_changed: bool,
    pub status_terminal: bool,
    pub assignee_changed: bool,
    pub summary_changed: bool,
    pub new_comments: Vec<JiraComment>,
    pub description_changed: bool,
}

pub const TERMINAL_STATUSES: &[&str] = &["Done", "Closed", "Cancelled", "Resolved"];

pub fn diff_issue(prev: &JiraIssue, curr: &JiraIssue) -> JiraDiff {
    let prev_ids: std::collections::HashSet<&str> =
        prev.fields.comment.comments.iter().map(|c| c.id.as_str()).collect();
    let new_comments: Vec<JiraComment> = curr.fields.comment.comments.iter()
        .filter(|c| !prev_ids.contains(c.id.as_str())).cloned().collect();
    JiraDiff {
        status_changed: prev.fields.status.name != curr.fields.status.name,
        status_terminal: TERMINAL_STATUSES.contains(&curr.fields.status.name.as_str()),
        assignee_changed: prev.fields.assignee.as_ref().map(|u| &u.account_id)
            != curr.fields.assignee.as_ref().map(|u| &u.account_id),
        summary_changed: prev.fields.summary != curr.fields.summary,
        new_comments,
        description_changed: prev.fields.description != curr.fields.description,
    }
}
```

- [ ] **Step 3: Test passes; commit**

```bash
git commit -am "feat(kanban): Jira issue diff"
```

---

## Phase C: Reconciler

### Task C1: Reconciliation actions

**Files:**
- Create: `crates/kanban-orchestrator/src/reconciler.rs`
- Create: `crates/kanban-orchestrator/tests/reconciler_test.rs`

- [ ] **Step 1: Failing test**

```rust
use kanban_orchestrator::reconciler::{ReconcileAction, decide_action};
use kanban_orchestrator::jira::JiraDiff;
use kanban_orchestrator::config::Reconciliation;

#[test]
fn terminal_status_stops_immediately() {
    let mut d = JiraDiff::default();
    d.status_changed = true; d.status_terminal = true;
    let cfg = Reconciliation { on_jira_status_terminal: Some("stop_immediately".into()), ..Default::default() };
    let actions = decide_action(&d, &cfg);
    assert!(actions.contains(&ReconcileAction::Stop));
    assert!(actions.contains(&ReconcileAction::Notify("status_changed".into())));
}

#[test]
fn new_comment_queues_inject() {
    let mut d = JiraDiff::default();
    d.new_comments.push(kanban_orchestrator::jira::JiraComment {
        id: "c1".into(), body: serde_json::Value::Null, updated: "2026-05-07T00:00:00Z".into()
    });
    let cfg = Reconciliation { on_jira_comment: Some("inject_next_turn".into()), ..Default::default() };
    let actions = decide_action(&d, &cfg);
    assert!(matches!(actions[0], ReconcileAction::QueueInject(_)));
}
```

- [ ] **Step 2: Implement**

```rust
use crate::config::Reconciliation;
use crate::jira::JiraDiff;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReconcileAction {
    Stop,                                  // cooperative cancel
    QueueInject(String),                   // markdown blob written to reconcile.md next turn
    Notify(String),                        // event name
    UpdateSnapshot,                        // always implied; explicit for tests
}

pub fn decide_action(diff: &JiraDiff, cfg: &Reconciliation) -> Vec<ReconcileAction> {
    let mut out = vec![ReconcileAction::UpdateSnapshot];
    if diff.status_terminal && cfg.on_jira_status_terminal.as_deref() == Some("stop_immediately") {
        out.push(ReconcileAction::Stop);
    }
    if diff.assignee_changed && cfg.on_jira_assignee_change.as_deref() == Some("stop_immediately") {
        out.push(ReconcileAction::Stop);
    }
    if !diff.new_comments.is_empty() && cfg.on_jira_comment.as_deref() == Some("inject_next_turn") {
        let blob = format!("New comments since last turn:\n{}",
            serde_json::to_string_pretty(&diff.new_comments).unwrap_or_default());
        out.push(ReconcileAction::QueueInject(blob));
    }
    if (diff.summary_changed || diff.description_changed)
        && cfg.on_jira_summary_change.as_deref() == Some("inject_next_turn")
    {
        out.push(ReconcileAction::QueueInject("Summary/description changed; re-read task.json.".into()));
    }
    if diff.status_changed { out.push(ReconcileAction::Notify("status_changed".into())); }
    out
}
```

- [ ] **Step 3: Tests pass; commit**

```bash
git commit -am "feat(kanban): reconciliation action decider"
```

---

### Task C2: Apply reconciliation to DB (`upsert_card_from_jira`)

**Files:**
- Modify: `crates/kanban-orchestrator/src/reconciler.rs`
- Modify: `crates/kanban-orchestrator/tests/reconciler_test.rs`

- [ ] **Step 1: Failing test**

```rust
#[tokio::test]
async fn new_issue_creates_card_in_initial_column() {
    let pool = test_pool().await;
    let project_id = create_project(&pool, "AP").await;
    let workflow = load_test_workflow();
    let issue = sample_issue();
    let outcome = kanban_orchestrator::reconciler::upsert_card_from_jira(
        &pool, project_id, &workflow, &issue
    ).await.unwrap();
    assert!(outcome.created);
    assert_eq!(outcome.kanban_phase, Some("Todo".to_string()));
}
```

(`test_pool`, `create_project`, `load_test_workflow`, `sample_issue` are test helpers — define them in a shared `tests/common/mod.rs`.)

- [ ] **Step 2: Implement**

```rust
use db::models::task::{PhaseState, Task};
use sqlx::SqlitePool;
use uuid::Uuid;

pub struct UpsertOutcome {
    pub task_id: Uuid,
    pub created: bool,
    pub kanban_phase: Option<String>,
    pub diff: Option<crate::jira::JiraDiff>,
}

pub async fn upsert_card_from_jira(
    pool: &SqlitePool,
    project_id: Uuid,
    workflow: &crate::config::Workflow,
    issue: &crate::jira::JiraIssue,
) -> crate::Result<UpsertOutcome> {
    let snapshot_json = serde_json::to_string(issue).unwrap();
    let now = chrono::Utc::now();

    // Look up existing
    let existing: Option<(Uuid, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT id as \"id!: Uuid\", kanban_phase, jira_snapshot FROM tasks WHERE jira_key = ?"
    ).bind(&issue.key).fetch_optional(pool).await?;

    if let Some((id, phase, prev_snap)) = existing {
        let prev: crate::jira::JiraIssue = serde_json::from_str(&prev_snap.unwrap_or("null".into()))
            .unwrap_or_else(|_| issue.clone());
        let diff = crate::jira::diff_issue(&prev, issue);
        sqlx::query("UPDATE tasks SET jira_snapshot = ?, jira_synced_at = ? WHERE id = ?")
            .bind(&snapshot_json).bind(now).bind(id).execute(pool).await?;
        return Ok(UpsertOutcome { task_id: id, created: false, kanban_phase: phase, diff: Some(diff) });
    }

    // Create new
    let id = Uuid::new_v4();
    let initial = workflow.initial_column().ok_or_else(|| crate::OrchestratorError::Workflow("no initial column".into()))?;
    sqlx::query(
        "INSERT INTO tasks (id, project_id, title, description, status, jira_key, jira_snapshot, jira_synced_at, kanban_phase, phase_state, current_turn, created_at, updated_at) \
         VALUES (?, ?, ?, ?, 'todo', ?, ?, ?, ?, 'idle', 0, ?, ?)"
    )
        .bind(id).bind(project_id)
        .bind(&issue.fields.summary)
        .bind(serde_json::to_string(&issue.fields.description).ok())
        .bind(&issue.key).bind(&snapshot_json).bind(now)
        .bind(&initial.name).bind(now).bind(now)
        .execute(pool).await?;
    Ok(UpsertOutcome { task_id: id, created: true, kanban_phase: Some(initial.name.clone()), diff: None })
}

pub async fn enqueue_inject(pool: &SqlitePool, task_id: Uuid, blob: &str) -> crate::Result<()> {
    sqlx::query("UPDATE tasks SET pending_inject = ? WHERE id = ?")
        .bind(blob).bind(task_id).execute(pool).await?;
    Ok(())
}

pub async fn mark_archived(pool: &SqlitePool, task_id: Uuid) -> crate::Result<()> {
    sqlx::query("UPDATE tasks SET phase_state = 'archived' WHERE id = ?")
        .bind(task_id).execute(pool).await?;
    Ok(())
}
```

- [ ] **Step 3: Tests pass; commit**

```bash
git commit -am "feat(kanban): upsert + inject queueing"
```

---

## Phase D: Dispatcher

### Task D1: Marker parser

**Files:**
- Create: `crates/kanban-orchestrator/src/dispatcher/markers.rs`
- Create: `crates/kanban-orchestrator/tests/markers_test.rs`

- [ ] **Step 1: Failing test**

```rust
use kanban_orchestrator::dispatcher::markers::{parse_markers, MarkerOutcome};

#[test]
fn detects_complete_marker() {
    let s = "doing work\n<<KANBAN_PHASE_COMPLETE>>\nbye";
    assert_eq!(parse_markers(s), MarkerOutcome::Complete);
}

#[test]
fn detects_failure_with_reason() {
    let s = "<<KANBAN_PHASE_FAILED reason=\"build broke\">>";
    match parse_markers(s) {
        MarkerOutcome::Failed(r) => assert_eq!(r, "build broke"),
        _ => panic!()
    }
}

#[test]
fn no_marker_means_continue() {
    assert_eq!(parse_markers("nothing special"), MarkerOutcome::Continue);
}
```

- [ ] **Step 2: Implement**

```rust
#[derive(Debug, PartialEq, Eq)]
pub enum MarkerOutcome { Complete, Failed(String), Continue }

pub fn parse_markers(stdout: &str) -> MarkerOutcome {
    if let Some(idx) = stdout.find("<<KANBAN_PHASE_FAILED") {
        let tail = &stdout[idx..];
        let reason = tail.find("reason=\"").and_then(|a| {
            let start = a + "reason=\"".len();
            tail[start..].find('"').map(|e| tail[start..start+e].to_string())
        }).unwrap_or_default();
        return MarkerOutcome::Failed(reason);
    }
    if stdout.contains("<<KANBAN_PHASE_COMPLETE>>") { return MarkerOutcome::Complete; }
    MarkerOutcome::Continue
}
```

- [ ] **Step 3: Tests pass; commit**

```bash
git commit -am "feat(kanban): completion/failure marker parser"
```

---

### Task D2: Context file writer

**Files:**
- Create: `crates/kanban-orchestrator/src/context.rs`
- Create: `crates/kanban-orchestrator/tests/context_test.rs`

- [ ] **Step 1: Failing test**

```rust
#[test]
fn writes_task_json_phase_md() {
    let dir = tempfile::tempdir().unwrap();
    let card = sample_card();
    kanban_orchestrator::context::write_context(dir.path(), &card, Some("reconcile-blob"), None).unwrap();
    let task = std::fs::read_to_string(dir.path().join(".kanban-context/task.json")).unwrap();
    assert!(task.contains("\"jira_key\""));
    let rec = std::fs::read_to_string(dir.path().join(".kanban-context/reconcile.md")).unwrap();
    assert_eq!(rec, "reconcile-blob");
    assert!(dir.path().join(".kanban-context/phase.md").exists());
}
```

(`sample_card` builds a minimal `Task`.)

- [ ] **Step 2: Implement**

```rust
use db::models::task::Task;
use std::path::Path;

pub fn write_context(
    worktree: &Path,
    card: &Task,
    reconcile_blob: Option<&str>,
    review_feedback: Option<&str>,
) -> crate::Result<()> {
    let dir = worktree.join(".kanban-context");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("task.json"), serde_json::to_string_pretty(card).unwrap())?;
    let phase_md = format!(
        "# Current phase\n\nColumn: {}\nTurn: {}\n",
        card.kanban_phase.as_deref().unwrap_or("(none)"),
        card.current_turn,
    );
    std::fs::write(dir.join("phase.md"), phase_md)?;
    let recpath = dir.join("reconcile.md");
    match reconcile_blob {
        Some(s) => std::fs::write(&recpath, s)?,
        None => { let _ = std::fs::remove_file(&recpath); }
    }
    let revpath = dir.join("review_feedback.md");
    match review_feedback {
        Some(s) => std::fs::write(&revpath, s)?,
        None => { let _ = std::fs::remove_file(&revpath); }
    }
    Ok(())
}

pub fn append_history(worktree: &Path, entry: &serde_json::Value) -> crate::Result<()> {
    let path = worktree.join(".kanban-context/history.jsonl");
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(f, "{}", serde_json::to_string(entry).unwrap())?;
    Ok(())
}
```

- [ ] **Step 3: Tests pass; commit**

```bash
git commit -am "feat(kanban): .kanban-context/ writer"
```

---

### Task D3: Concurrency gate

**Files:**
- Create: `crates/kanban-orchestrator/src/dispatcher/gate.rs`
- Create: `crates/kanban-orchestrator/tests/gate_test.rs`

- [ ] **Step 1: Failing test**

```rust
use kanban_orchestrator::dispatcher::gate::Gate;

#[tokio::test]
async fn gate_caps_concurrency() {
    let g = Gate::new(2, 1);
    let p1 = g.try_acquire("Coding").await.unwrap();
    let p2 = g.try_acquire("Reviewing").await.unwrap();
    assert!(g.try_acquire("Reviewing").await.is_none(), "per-column cap should reject");
    drop(p2);
    let p3 = g.try_acquire("Reviewing").await.unwrap();
    drop(p1); drop(p3);
}
```

- [ ] **Step 2: Implement**

```rust
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct Gate {
    inner: Arc<Mutex<GateState>>,
    max_global: usize,
    max_per_column: usize,
}

struct GateState { global: usize, per_col: HashMap<String, usize> }

pub struct Permit { gate: Arc<Mutex<GateState>>, column: String }

impl Drop for Permit {
    fn drop(&mut self) {
        let g = self.gate.clone();
        let col = self.column.clone();
        // Synchronous decrement using try_lock; if contended, spawn
        if let Ok(mut s) = g.try_lock() {
            s.global = s.global.saturating_sub(1);
            if let Some(v) = s.per_col.get_mut(&col) { *v = v.saturating_sub(1); }
        } else {
            tokio::spawn(async move {
                let mut s = g.lock().await;
                s.global = s.global.saturating_sub(1);
                if let Some(v) = s.per_col.get_mut(&col) { *v = v.saturating_sub(1); }
            });
        }
    }
}

impl Gate {
    pub fn new(max_global: usize, max_per_column: usize) -> Self {
        Self { inner: Arc::new(Mutex::new(GateState { global: 0, per_col: HashMap::new() })),
               max_global, max_per_column }
    }
    pub async fn try_acquire(&self, column: &str) -> Option<Permit> {
        let mut s = self.inner.lock().await;
        if s.global >= self.max_global { return None; }
        let cur = *s.per_col.get(column).unwrap_or(&0);
        if cur >= self.max_per_column { return None; }
        s.global += 1;
        s.per_col.insert(column.to_string(), cur + 1);
        Some(Permit { gate: self.inner.clone(), column: column.to_string() })
    }
}
```

- [ ] **Step 3: Tests pass; commit**

```bash
git commit -am "feat(kanban): concurrency gate"
```

---

### Task D4: Phase execution loop

**Files:**
- Create: `crates/kanban-orchestrator/src/dispatcher/phase.rs`
- Modify: `crates/kanban-orchestrator/src/dispatcher.rs` to expose `run_phase`
- Create: `crates/kanban-orchestrator/tests/phase_test.rs`

This task ties together markers, context, gate, and the existing `executors` crate.

- [ ] **Step 1: Define the Executor trait alias**

In `dispatcher.rs`:
```rust
pub mod gate;
pub mod markers;
pub mod phase;

#[async_trait::async_trait]
pub trait PhaseExecutor: Send + Sync + 'static {
    async fn run_turn(
        &self,
        worktree: &std::path::Path,
        agent_md_path: &std::path::Path,
        timeout: std::time::Duration,
    ) -> crate::Result<TurnOutcome>;
}

pub struct TurnOutcome {
    pub stdout: String,
    pub stderr: String,
    pub session_id: uuid::Uuid,
}
```

(Add `async-trait = "0.1"` to Cargo.toml.)

A separate adapter (Task D5) will implement `PhaseExecutor` over the real `executors` crate.

- [ ] **Step 2: Failing test using a mock executor**

```rust
use kanban_orchestrator::dispatcher::{PhaseExecutor, TurnOutcome};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

struct CompleteOnTurn(usize, AtomicUsize);
#[async_trait::async_trait]
impl PhaseExecutor for CompleteOnTurn {
    async fn run_turn(&self, _: &Path, _: &Path, _: std::time::Duration) -> kanban_orchestrator::Result<TurnOutcome> {
        let n = self.1.fetch_add(1, Ordering::SeqCst) + 1;
        let stdout = if n >= self.0 { "<<KANBAN_PHASE_COMPLETE>>".into() } else { "still working".into() };
        Ok(TurnOutcome { stdout, stderr: String::new(), session_id: uuid::Uuid::new_v4() })
    }
}

#[tokio::test]
async fn phase_completes_on_marker() {
    let exec = CompleteOnTurn(2, AtomicUsize::new(0));
    let outcome = run_test_phase(&exec, /*max_turns=*/5).await;
    assert!(matches!(outcome, kanban_orchestrator::dispatcher::phase::PhaseOutcome::Complete { turns_used: 2, .. }));
}

#[tokio::test]
async fn phase_review_when_max_turns_and_review_required() {
    let exec = CompleteOnTurn(99, AtomicUsize::new(0));
    let outcome = run_test_phase_with_review(&exec, /*max_turns=*/3).await;
    assert!(matches!(outcome, kanban_orchestrator::dispatcher::phase::PhaseOutcome::AwaitingReview { turns_used: 3, .. }));
}
```

(Helpers `run_test_phase` etc. wire up tempdir worktree, fake card, dummy agent.md.)

- [ ] **Step 3: Implement `phase.rs`**

```rust
use crate::config::{Column, OnComplete};
use crate::context::{append_history, write_context};
use crate::dispatcher::markers::{parse_markers, MarkerOutcome};
use crate::dispatcher::{PhaseExecutor, TurnOutcome};
use db::models::task::Task;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;

pub enum PhaseOutcome {
    Complete { turns_used: u32, last_session: uuid::Uuid },
    AwaitingReview { turns_used: u32, last_session: uuid::Uuid },
    Failed { reason: String, turns_used: u32 },
    Cancelled,
}

pub struct PhaseInputs<'a> {
    pub card: &'a Task,
    pub column: &'a Column,
    pub worktree: &'a Path,
    pub agent_md: &'a Path,
    pub max_turns: u32,
    pub turn_timeout: Duration,
    pub reconcile_blob: Option<String>,
    pub review_feedback: Option<String>,
    pub cancel: Arc<Notify>,
}

pub async fn run_phase(
    inputs: PhaseInputs<'_>,
    exec: &dyn PhaseExecutor,
) -> crate::Result<PhaseOutcome> {
    write_context(inputs.worktree, inputs.card, inputs.reconcile_blob.as_deref(), inputs.review_feedback.as_deref())?;

    let mut last_session = uuid::Uuid::nil();
    for turn in 1..=inputs.max_turns {
        if cancelled(&inputs.cancel) { return Ok(PhaseOutcome::Cancelled); }
        let out: TurnOutcome = exec.run_turn(inputs.worktree, inputs.agent_md, inputs.turn_timeout).await?;
        last_session = out.session_id;
        match parse_markers(&out.stdout) {
            MarkerOutcome::Complete => {
                append_history(inputs.worktree, &serde_json::json!({
                    "phase": inputs.column.name, "turns": turn, "session": out.session_id
                }))?;
                return Ok(PhaseOutcome::Complete { turns_used: turn, last_session });
            }
            MarkerOutcome::Failed(reason) => {
                return Ok(PhaseOutcome::Failed { reason, turns_used: turn });
            }
            MarkerOutcome::Continue => continue,
        }
    }
    if inputs.column.on_complete == OnComplete::Review {
        Ok(PhaseOutcome::AwaitingReview { turns_used: inputs.max_turns, last_session })
    } else {
        // auto-advance even though no marker was emitted
        Ok(PhaseOutcome::Complete { turns_used: inputs.max_turns, last_session })
    }
}

fn cancelled(n: &Arc<Notify>) -> bool {
    // non-blocking check using try_recv-style pattern
    use tokio::sync::futures::Notified;
    let mut fut = std::pin::pin!(n.notified());
    matches!(futures::poll!(fut.as_mut()), std::task::Poll::Ready(_))
}
```

(Add `futures = "0.3"` to dev-dependencies; cancellation polling is best-effort. If `futures::poll!` is awkward, use an `AtomicBool` shared between dispatcher and reconciler instead — the test verifies behavior, the implementation may differ.)

- [ ] **Step 4: Tests pass; commit**

```bash
git commit -am "feat(kanban): phase execution loop with marker + review handling"
```

---

### Task D5: Adapter from `crates/executors` to `PhaseExecutor`

**Files:**
- Create: `crates/kanban-orchestrator/src/dispatcher/exec_adapter.rs`
- Modify: `crates/kanban-orchestrator/src/dispatcher.rs`
- Create: `crates/kanban-orchestrator/tests/exec_adapter_test.rs`

The current executor API is `StandardCodingAgentExecutor::spawn(current_dir, prompt, env) -> SpawnedChild`; there is no `Executor::run`. The adapter should read the agent markdown body as the prompt, call the selected `CodingAgent`, and collect process stdout/stderr from the spawned child.

- [ ] **Step 1: Add adapter error conversion**

Extend `OrchestratorError` in `crates/kanban-orchestrator/src/lib.rs`:

```rust
#[error("executor: {0}")]
Executor(#[from] executors::executors::ExecutorError),
#[error("executor timeout after {0:?}")]
ExecutorTimeout(std::time::Duration),
```

- [ ] **Step 2: Implement adapter**

Create `crates/kanban-orchestrator/src/dispatcher/exec_adapter.rs`:

```rust
use std::{path::Path, time::Duration};

use executors::{
    env::{ExecutionEnv, RepoContext},
    executors::{CodingAgent, StandardCodingAgentExecutor},
};
use tokio::io::AsyncReadExt;

use crate::dispatcher::{PhaseExecutor, TurnOutcome};

#[derive(Clone)]
pub struct RealExecutor {
    agent: CodingAgent,
    env: ExecutionEnv,
}

impl RealExecutor {
    pub fn new(agent: CodingAgent, workspace_root: std::path::PathBuf, repo_names: Vec<String>) -> Self {
        let env = ExecutionEnv::new(RepoContext::new(workspace_root, repo_names), false, String::new());
        Self { agent, env }
    }

    fn prompt_from_agent_md(agent_md: &Path) -> crate::Result<String> {
        let text = std::fs::read_to_string(agent_md)?;
        if let Some(rest) = text.strip_prefix("---") {
            if let Some((_, body)) = rest.split_once("---") {
                return Ok(body.trim().to_string());
            }
        }
        Ok(text)
    }
}

#[async_trait::async_trait]
impl PhaseExecutor for RealExecutor {
    async fn run_turn(
        &self,
        worktree: &Path,
        agent_md: &Path,
        timeout: Duration,
    ) -> crate::Result<TurnOutcome> {
        let prompt = Self::prompt_from_agent_md(agent_md)?;
        let mut spawned = self.agent.spawn(worktree, &prompt, &self.env).await?;

        let mut stdout = String::new();
        let mut stderr = String::new();
        if let Some(mut out) = spawned.child.inner().stdout.take() {
            out.read_to_string(&mut stdout).await?;
        }
        if let Some(mut err) = spawned.child.inner().stderr.take() {
            err.read_to_string(&mut stderr).await?;
        }

        let status = tokio::time::timeout(timeout, spawned.child.wait())
            .await
            .map_err(|_| crate::OrchestratorError::ExecutorTimeout(timeout))??;

        if !status.success() && stdout.trim().is_empty() {
            stdout = format!("<<KANBAN_PHASE_FAILED reason=\"executor exited with {status}\">>");
        }

        Ok(TurnOutcome {
            stdout,
            stderr,
            session_id: uuid::Uuid::new_v4(),
        })
    }
}
```

If `command_group::AsyncGroupChild` exposes stdout/stderr through a different accessor in the installed crate version, adapt only the three lines that use `spawned.child.inner().stdout/stderr`; keep the rest of the adapter contract unchanged.

- [ ] **Step 3: Export adapter module**

Modify `crates/kanban-orchestrator/src/dispatcher.rs`:

```rust
pub mod exec_adapter;
```

- [ ] **Step 4: Add QA-mode smoke test**

Create `crates/kanban-orchestrator/tests/exec_adapter_test.rs`:

```rust
#![cfg(feature = "qa-mode")]

use executors::executors::{CodingAgent, qa_mock::QaMockExecutor};
use kanban_orchestrator::dispatcher::{PhaseExecutor, exec_adapter::RealExecutor};

#[tokio::test]
async fn qa_mock_adapter_collects_stdout() {
    let dir = tempfile::tempdir().unwrap();
    let agent_md = dir.path().join("agent.md");
    std::fs::write(&agent_md, "---\nname: qa\n---\nSay hello").unwrap();

    let executor = RealExecutor::new(
        CodingAgent::QaMock(QaMockExecutor),
        dir.path().to_path_buf(),
        Vec::new(),
    );

    let outcome = executor
        .run_turn(dir.path(), &agent_md, std::time::Duration::from_secs(20))
        .await
        .unwrap();

    assert!(!outcome.stdout.is_empty());
}
```

Run:

```bash
cargo test -p kanban-orchestrator --features qa-mode qa_mock_adapter_collects_stdout
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/kanban-orchestrator/src/lib.rs crates/kanban-orchestrator/src/dispatcher.rs crates/kanban-orchestrator/src/dispatcher/exec_adapter.rs crates/kanban-orchestrator/tests/exec_adapter_test.rs
git commit -m "feat(kanban): adapt executors for phase turns"
```

---

## Phase E: Scheduler

### Task E1: Tokio interval + manual trigger

**Files:**
- Create: `crates/kanban-orchestrator/src/scheduler.rs`
- Create: `crates/kanban-orchestrator/tests/scheduler_test.rs`

- [ ] **Step 1: Failing test**

```rust
#[tokio::test(start_paused = true)]
async fn scheduler_polls_on_interval_and_on_trigger() {
    let counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let c2 = counter.clone();
    let trigger = kanban_orchestrator::scheduler::ManualTrigger::new();
    let s = kanban_orchestrator::scheduler::Scheduler::new(
        std::time::Duration::from_secs(900), trigger.clone(),
        move || { c2.fetch_add(1, std::sync::atomic::Ordering::SeqCst); async {} }
    );
    let h = tokio::spawn(s.run());
    tokio::time::advance(std::time::Duration::from_millis(10)).await;  // first immediate tick
    trigger.fire();
    tokio::time::advance(std::time::Duration::from_millis(10)).await;
    tokio::time::advance(std::time::Duration::from_secs(900)).await;
    assert!(counter.load(std::sync::atomic::Ordering::SeqCst) >= 3);
    h.abort();
}
```

- [ ] **Step 2: Implement**

```rust
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;

#[derive(Clone)]
pub struct ManualTrigger(Arc<Notify>);
impl ManualTrigger {
    pub fn new() -> Self { Self(Arc::new(Notify::new())) }
    pub fn fire(&self) { self.0.notify_one(); }
}

type TickFn = Box<dyn Fn() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

pub struct Scheduler {
    interval: Duration,
    trigger: ManualTrigger,
    tick: TickFn,
}

impl Scheduler {
    pub fn new<F, Fut>(interval: Duration, trigger: ManualTrigger, f: F) -> Self
    where F: Fn() -> Fut + Send + Sync + 'static, Fut: Future<Output = ()> + Send + 'static {
        let tick: TickFn = Box::new(move || Box::pin(f()));
        Self { interval, trigger, tick }
    }

    pub async fn run(self) {
        let mut ticker = tokio::time::interval(self.interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = ticker.tick() => { (self.tick)().await; }
                _ = self.trigger.0.notified() => { (self.tick)().await; }
            }
        }
    }
}
```

- [ ] **Step 3: Tests pass; commit**

```bash
git commit -am "feat(kanban): tokio scheduler with manual trigger"
```

---

### Task E2: Tick body — wire poller + reconciler + dispatcher

**Files:**
- Modify: `crates/kanban-orchestrator/src/scheduler.rs`
- Create: `crates/kanban-orchestrator/src/scheduler/tick.rs`

- [ ] **Step 1: Implement `do_tick`**

```rust
pub struct OrchestratorContext {
    pub pool: sqlx::SqlitePool,
    pub workflow: crate::config::Workflow,
    pub repo_root: std::path::PathBuf,
    pub jira: crate::jira::JiraClient,
    pub gate: std::sync::Arc<crate::dispatcher::gate::Gate>,
    pub executor: std::sync::Arc<dyn crate::dispatcher::PhaseExecutor>,
    pub notifier: std::sync::Arc<crate::notifier::Notifier>,
    pub project_id: uuid::Uuid,
}

pub async fn do_tick(ctx: &OrchestratorContext) -> crate::Result<()> {
    // 1) poll
    let issues = ctx.jira.search(&ctx.workflow.sync.jira.jql).await?;
    // 2) reconcile each
    for issue in &issues {
        let outcome = crate::reconciler::upsert_card_from_jira(
            &ctx.pool, ctx.project_id, &ctx.workflow, issue
        ).await?;
        if outcome.created {
            ctx.notifier.notify("New card", &issue.key, &issue.fields.summary);
            crate::events::emit(&ctx.pool, outcome.task_id, "card_created", "scheduler", None, None, None).await?;
        }
        if let Some(diff) = outcome.diff {
            let actions = crate::reconciler::decide_action(&diff, &ctx.workflow.reconciliation);
            for a in actions {
                use crate::reconciler::ReconcileAction;
                match a {
                    ReconcileAction::Stop => crate::reconciler::mark_archived(&ctx.pool, outcome.task_id).await?,
                    ReconcileAction::QueueInject(blob) => crate::reconciler::enqueue_inject(&ctx.pool, outcome.task_id, &blob).await?,
                    ReconcileAction::Notify(ev) => ctx.notifier.notify(&ev, &issue.key, &issue.fields.summary),
                    ReconcileAction::UpdateSnapshot => {}
                }
            }
        }
    }
    // 3) dispatch idle cards with an agent column
    crate::scheduler::tick::dispatch_round(ctx).await?;
    Ok(())
}
```

- [ ] **Step 2: Implement `dispatch_round` in `tick.rs`**

```rust
pub async fn dispatch_round(ctx: &super::OrchestratorContext) -> crate::Result<()> {
    let rows: Vec<(uuid::Uuid, String)> = sqlx::query_as(
        "SELECT id as \"id!: Uuid\", kanban_phase FROM tasks \
         WHERE phase_state = 'idle' AND kanban_phase IS NOT NULL AND project_id = ?"
    ).bind(ctx.project_id).fetch_all(&ctx.pool).await?;

    for (task_id, column_name) in rows {
        let Some(column) = ctx.workflow.column(&column_name) else { continue; };
        if column.agent.is_none() { continue; }
        let Some(_permit) = ctx.gate.try_acquire(&column_name).await else { continue; };

        // Spawn the phase task; permit moves into the spawned future via Arc clone pattern.
        let ctx_clone = ctx.clone_for_spawn();
        tokio::spawn(async move {
            if let Err(e) = run_one_phase(&ctx_clone, task_id).await {
                tracing::error!(?e, "phase failed");
            }
        });
    }
    Ok(())
}

async fn run_one_phase(_ctx: &super::OrchestratorContext, task_id: uuid::Uuid) -> crate::Result<()> {
    Err(crate::OrchestratorError::Workflow(format!(
        "run_one_phase skeleton reached for task {task_id}; Task E3 replaces this body"
    )))
}
```

`OrchestratorContext::clone_for_spawn` constructs a cheap clone (pool/jira/gate/executor/notifier are `Arc`-friendly).

- [ ] **Step 3: Commit (compile-clean skeleton; Task E3 replaces `run_one_phase`)**

```bash
git commit -am "feat(kanban): scheduler tick wiring (skeleton)"
```

---

### Task E3: `run_one_phase` — DB transitions per PhaseOutcome

**Files:**
- Modify: `crates/kanban-orchestrator/src/scheduler/tick.rs`
- Create: `crates/kanban-orchestrator/tests/tick_test.rs`

- [ ] **Step 1: Failing test**

End-to-end-ish: seed a card in `idle/Coding`, mock executor returns complete marker, run dispatch_round, assert card moved to `Reviewing` (next column) and `phase_state=idle` and `task_events` has `phase_advanced`.

- [ ] **Step 2: Implement**

```rust
async fn run_one_phase(ctx: &super::OrchestratorContext, task_id: uuid::Uuid) -> crate::Result<()> {
    let card: db::models::task::Task = db::models::task::Task::find_by_id(&ctx.pool, task_id).await?
        .ok_or_else(|| crate::OrchestratorError::Other(anyhow::anyhow!("task gone")))?;
    let column = ctx.workflow.column(card.kanban_phase.as_deref().unwrap_or(""))
        .ok_or_else(|| crate::OrchestratorError::Workflow("unknown column".into()))?;
    let agent_name = column.agent.as_deref()
        .ok_or_else(|| crate::OrchestratorError::Workflow("column has no agent".into()))?;
    let agent_md = ctx.repo_root.join(format!(".agents/agent/{}.md", agent_name));
    let worktree = worktree_manager::ensure(&ctx.pool, &card).await?;   // signature TBD per recon

    // mark running
    sqlx::query("UPDATE tasks SET phase_state='running' WHERE id=?")
        .bind(task_id).execute(&ctx.pool).await?;
    crate::events::emit(&ctx.pool, task_id, "turn_started", "dispatcher",
        card.kanban_phase.as_deref(), None, None).await?;

    let max_turns = column.max_turns_per_phase
        .or(ctx.workflow.defaults.max_turns_per_phase)
        .unwrap_or(5);

    let pending_inject = card.pending_inject.clone();
    sqlx::query("UPDATE tasks SET pending_inject=NULL WHERE id=?").bind(task_id).execute(&ctx.pool).await?;

    let cancel = std::sync::Arc::new(tokio::sync::Notify::new());
    let inputs = crate::dispatcher::phase::PhaseInputs {
        card: &card, column, worktree: &worktree, agent_md: &agent_md,
        max_turns, turn_timeout: std::time::Duration::from_secs(60 * 30),
        reconcile_blob: pending_inject, review_feedback: None, cancel,
    };
    let outcome = crate::dispatcher::phase::run_phase(inputs, ctx.executor.as_ref()).await?;

    use crate::dispatcher::phase::PhaseOutcome;
    match outcome {
        PhaseOutcome::Complete { turns_used, last_session } => {
            advance_card(ctx, &card, column, turns_used, last_session).await?;
        }
        PhaseOutcome::AwaitingReview { turns_used, last_session } => {
            sqlx::query("UPDATE tasks SET phase_state='awaiting_review', current_turn=?, review_pending_since=?, last_executor_session_id=? WHERE id=?")
                .bind(turns_used as i64).bind(chrono::Utc::now()).bind(last_session).bind(task_id)
                .execute(&ctx.pool).await?;
            ctx.notifier.notify("awaiting_review", card.jira_key.as_deref().unwrap_or(""), &card.title);
            crate::events::emit(&ctx.pool, task_id, "awaiting_review", "dispatcher",
                card.kanban_phase.as_deref(), None, None).await?;
        }
        PhaseOutcome::Failed { reason, turns_used } => {
            let info = serde_json::json!({"reason":reason,"turns":turns_used});
            sqlx::query("UPDATE tasks SET phase_state='error', error_info=?, current_turn=? WHERE id=?")
                .bind(info.to_string()).bind(turns_used as i64).bind(task_id).execute(&ctx.pool).await?;
            ctx.notifier.notify("error", card.jira_key.as_deref().unwrap_or(""), &card.title);
            crate::events::emit(&ctx.pool, task_id, "error", "agent", None, None, Some(&info)).await?;
        }
        PhaseOutcome::Cancelled => {
            sqlx::query("UPDATE tasks SET phase_state='archived' WHERE id=?")
                .bind(task_id).execute(&ctx.pool).await?;
        }
    }
    Ok(())
}

async fn advance_card(
    ctx: &super::OrchestratorContext,
    card: &db::models::task::Task,
    column: &crate::config::Column,
    turns_used: u32, last_session: uuid::Uuid,
) -> crate::Result<()> {
    let next = column.next.as_deref().ok_or_else(|| crate::OrchestratorError::Workflow("non-terminal column missing next".into()))?;
    if let Some(t) = column.jira_transition.as_deref() {
        if let Some(key) = card.jira_key.as_deref() {
            if let Err(e) = ctx.jira.transition(key, t).await {
                let info = serde_json::json!({"jira_transition_failed": e.to_string(), "transition": t});
                sqlx::query("UPDATE tasks SET error_info=? WHERE id=?")
                    .bind(info.to_string()).bind(card.id).execute(&ctx.pool).await?;
            }
        }
    }
    sqlx::query("UPDATE tasks SET kanban_phase=?, phase_state='idle', current_turn=0, last_executor_session_id=? WHERE id=?")
        .bind(next).bind(last_session).bind(card.id).execute(&ctx.pool).await?;
    crate::events::emit(&ctx.pool, card.id, "phase_advanced", "dispatcher",
        card.kanban_phase.as_deref(), Some(next), None).await?;
}
```

- [ ] **Step 3: Tests pass; commit**

```bash
git commit -am "feat(kanban): full phase outcome handling"
```

---

## Phase F: API + CLI

### Task F1: HTTP routes using current `DeploymentImpl` state

**Files:**
- Create: `crates/server/src/routes/kanban.rs`
- Modify: `crates/server/src/routes/mod.rs` (register routes)
- Modify: `crates/server/src/error.rs` (convert orchestrator errors into API errors)
- Modify: `crates/server/Cargo.toml` (add `kanban-orchestrator = { path = "../kanban-orchestrator" }`)

- [ ] **Step 1: Add API helper surface in orchestrator**

Add this module to `crates/kanban-orchestrator/src/lib.rs`:

```rust
pub mod api;
```

Create `crates/kanban-orchestrator/src/api.rs`:

```rust
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::scheduler::ManualTrigger;

#[derive(Clone)]
pub struct KanbanHandle {
    trigger: ManualTrigger,
}

impl KanbanHandle {
    pub fn new(trigger: ManualTrigger) -> Self {
        Self { trigger }
    }

    pub fn poll_now(&self) {
        self.trigger.fire();
    }
}

#[derive(Debug, Deserialize)]
pub struct RequestChangesBody {
    pub feedback: String,
}

#[derive(Debug, Serialize)]
pub struct TaskEventResponse {
    pub id: Uuid,
    pub event_type: String,
    pub from_phase: Option<String>,
    pub to_phase: Option<String>,
    pub actor: String,
    pub payload: Option<String>,
    pub ts: String,
}

pub async fn approve(pool: &SqlitePool, task_id: Uuid) -> crate::Result<()> {
    sqlx::query("UPDATE tasks SET phase_state = 'idle', current_turn = 0 WHERE id = ?")
        .bind(task_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn request_changes(pool: &SqlitePool, task_id: Uuid, feedback: &str) -> crate::Result<()> {
    let payload = serde_json::json!({ "feedback": feedback }).to_string();
    sqlx::query("UPDATE tasks SET phase_state = 'running', current_turn = 0, pending_inject = ? WHERE id = ?")
        .bind(payload)
        .bind(task_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn cancel(pool: &SqlitePool, task_id: Uuid) -> crate::Result<()> {
    sqlx::query("UPDATE tasks SET phase_state = 'archived' WHERE id = ?")
        .bind(task_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn retry_error(pool: &SqlitePool, task_id: Uuid) -> crate::Result<()> {
    sqlx::query("UPDATE tasks SET phase_state = 'idle', error_info = NULL WHERE id = ?")
        .bind(task_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn events(pool: &SqlitePool, task_id: Uuid) -> crate::Result<Vec<TaskEventResponse>> {
    let rows = sqlx::query_as!(
        TaskEventResponse,
        r#"SELECT
            id AS "id!: Uuid",
            event_type,
            from_phase,
            to_phase,
            actor,
            payload,
            ts
        FROM task_events
        WHERE task_id = ?
        ORDER BY ts DESC"#,
        task_id
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}
```

This helper is intentionally DB-only. It must not depend on a global `AppState`; server routes will pass `deployment.db().pool` directly.

- [ ] **Step 2: Add API error conversion**

Modify `crates/server/src/error.rs`:

```rust
impl From<kanban_orchestrator::OrchestratorError> for ApiError {
    fn from(err: kanban_orchestrator::OrchestratorError) -> Self {
        match err {
            kanban_orchestrator::OrchestratorError::Db(err) => ApiError::Database(err),
            kanban_orchestrator::OrchestratorError::Io(err) => ApiError::Io(err),
            other => ApiError::BadRequest(other.to_string()),
        }
    }
}
```

Place the impl near the other `From` impls.

- [ ] **Step 3: Implement routes with the existing route pattern**

Create `crates/server/src/routes/kanban.rs`:

```rust
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::Json as ResponseJson,
    routing::{get, post},
};
use deployment::Deployment;
use kanban_orchestrator::api::{self, RequestChangesBody, TaskEventResponse};
use utils::response::ApiResponse;
use uuid::Uuid;

use crate::{DeploymentImpl, error::ApiError};

pub async fn poll_now(
    State(deployment): State<DeploymentImpl>,
    Path(_project_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    kanban_orchestrator::runtime::trigger_poll_now(deployment.db()).await?;
    Ok(StatusCode::ACCEPTED)
}

pub async fn sync_now(
    State(deployment): State<DeploymentImpl>,
    Path(task_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    kanban_orchestrator::runtime::sync_task_now(deployment.db(), task_id).await?;
    Ok(StatusCode::ACCEPTED)
}

pub async fn approve(
    State(deployment): State<DeploymentImpl>,
    Path(task_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    api::approve(&deployment.db().pool, task_id).await?;
    Ok(StatusCode::ACCEPTED)
}

pub async fn request_changes(
    State(deployment): State<DeploymentImpl>,
    Path(task_id): Path<Uuid>,
    Json(body): Json<RequestChangesBody>,
) -> Result<StatusCode, ApiError> {
    api::request_changes(&deployment.db().pool, task_id, &body.feedback).await?;
    Ok(StatusCode::ACCEPTED)
}

pub async fn cancel(
    State(deployment): State<DeploymentImpl>,
    Path(task_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    api::cancel(&deployment.db().pool, task_id).await?;
    Ok(StatusCode::ACCEPTED)
}

pub async fn retry_error(
    State(deployment): State<DeploymentImpl>,
    Path(task_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    api::retry_error(&deployment.db().pool, task_id).await?;
    Ok(StatusCode::ACCEPTED)
}

pub async fn events(
    State(deployment): State<DeploymentImpl>,
    Path(task_id): Path<Uuid>,
) -> Result<ResponseJson<ApiResponse<Vec<TaskEventResponse>>>, ApiError> {
    let events = api::events(&deployment.db().pool, task_id).await?;
    Ok(ResponseJson(ApiResponse::success(events)))
}

pub fn router(_deployment: &DeploymentImpl) -> Router<DeploymentImpl> {
    Router::new()
        .route("/projects/{project_id}/kanban/poll-now", post(poll_now))
        .route("/tasks/{task_id}/kanban/sync-now", post(sync_now))
        .route("/tasks/{task_id}/kanban/approve", post(approve))
        .route("/tasks/{task_id}/kanban/request-changes", post(request_changes))
        .route("/tasks/{task_id}/kanban/cancel", post(cancel))
        .route("/tasks/{task_id}/kanban/retry-error", post(retry_error))
        .route("/tasks/{task_id}/kanban/events", get(events))
}
```

The route strings are relative to `/api` because `crates/server/src/routes/mod.rs` nests `base_routes` under `/api`.

- [ ] **Step 4: Register route module**

Modify `crates/server/src/routes/mod.rs`:

```rust
pub mod kanban;
```

Add the router to `base_routes` near the existing task routes:

```rust
.merge(kanban::router(&deployment))
```

- [ ] **Step 5: Add runtime no-op stubs before wiring real scheduler handle**

Create `crates/kanban-orchestrator/src/runtime.rs` and export it from `lib.rs`:

```rust
use db::DBService;
use uuid::Uuid;

pub async fn trigger_poll_now(_db: &DBService) -> crate::Result<()> {
    Ok(())
}

pub async fn sync_task_now(_db: &DBService, _task_id: Uuid) -> crate::Result<()> {
    Ok(())
}
```

Later scheduler tasks replace these no-op functions with real handle lookup or direct tick execution. Keeping this stub here lets route wiring compile independently without inventing `AppState`.

- [ ] **Step 6: Run checks**

Run:

```bash
cargo check -p server
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/server/Cargo.toml crates/server/src/routes/mod.rs crates/server/src/routes/kanban.rs crates/server/src/error.rs crates/kanban-orchestrator/src/lib.rs crates/kanban-orchestrator/src/api.rs crates/kanban-orchestrator/src/runtime.rs
git commit -m "feat(server): add kanban control plane routes"
```

---

### Task F2: One-shot poll binary for local smoke testing

**Files:**
- Create: `crates/server/src/bin/kanban_poll_once.rs`
- Modify: `crates/server/Cargo.toml` only if Task F1 has not already added `kanban-orchestrator`

The current `server` binary has no clap parser and should not grow a partial CLI just for this feature. Implement the v1 `--once` path as a dedicated bin. A real `vibe-kanban kanban poll --once` subcommand can be added later if the project introduces a proper CLI command parser.

- [ ] **Step 1: Create the binary**

Create `crates/server/src/bin/kanban_poll_once.rs`:

```rust
use deployment::Deployment;
use server::DeploymentImpl;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .ok();

    let project = std::env::args()
        .skip(1)
        .find_map(|arg| arg.strip_prefix("--project=").map(str::to_owned));

    if project.as_deref().is_none_or(str::is_empty) {
        anyhow::bail!("usage: kanban_poll_once --project=AP");
    }

    let deployment = DeploymentImpl::new().await?;
    kanban_orchestrator::runtime::poll_once(deployment.db(), project.unwrap()).await?;
    Ok(())
}
```

- [ ] **Step 2: Add runtime entrypoint**

Modify `crates/kanban-orchestrator/src/runtime.rs`:

```rust
use db::DBService;
use uuid::Uuid;

pub async fn trigger_poll_now(_db: &DBService) -> crate::Result<()> {
    Ok(())
}

pub async fn sync_task_now(_db: &DBService, _task_id: Uuid) -> crate::Result<()> {
    Ok(())
}

pub async fn poll_once(db: &DBService, project: String) -> crate::Result<()> {
    tracing::info!(project, "running one kanban poll tick");
    crate::scheduler::tick::do_tick_for_project(db, &project).await
}
```

If `do_tick_for_project` has not been introduced yet, add it in Phase E as the real implementation target and keep this binary task blocked until Phase E completes.

- [ ] **Step 3: Manual smoke test**

Run:

```bash
JIRA_EMAIL=... JIRA_API_TOKEN=... cargo run -p server --bin kanban_poll_once -- --project=AP
```

Expected: exits 0 after one poll tick; logs include `running one kanban poll tick` and the project key.

- [ ] **Step 4: Commit**

```bash
git add crates/server/src/bin/kanban_poll_once.rs crates/kanban-orchestrator/src/runtime.rs
git commit -m "feat(kanban): add one-shot poll binary"
```

---

## Phase G: Notifier + Observability

### Task G1: Notifier adapter that reuses existing notification service

**Files:**
- Create: `crates/kanban-orchestrator/src/notifier.rs`
- Create: `crates/kanban-orchestrator/tests/notifier_test.rs`

The repo already has cross-platform notification code in `crates/services/src/services/notification.rs`, including macOS `osascript`. Do not duplicate platform-specific process spawning in the orchestrator crate.

- [ ] **Step 1: Implement adapter**

Create `crates/kanban-orchestrator/src/notifier.rs`:

```rust
use std::sync::{Arc, Mutex};

use services::services::notification::NotificationService;

#[derive(Clone)]
pub enum Notifier {
    Service(NotificationService),
    Capture(Arc<Mutex<Vec<(String, String)>>>),
    Disabled,
}

impl Notifier {
    pub fn service(service: NotificationService) -> Self {
        Self::Service(service)
    }

    pub fn disabled() -> Self {
        Self::Disabled
    }

    #[cfg(test)]
    pub fn capture() -> (Self, Arc<Mutex<Vec<(String, String)>>>) {
        let log = Arc::new(Mutex::new(Vec::new()));
        (Self::Capture(log.clone()), log)
    }

    pub async fn notify(&self, title: &str, message: &str) {
        match self {
            Self::Service(service) => service.notify(title, message).await,
            Self::Capture(log) => log.lock().unwrap().push((title.to_string(), message.to_string())),
            Self::Disabled => {}
        }
    }

    pub async fn card_created(&self, jira_key: &str, summary: &str) {
        self.notify("Kanban card created", &format!("{jira_key}: {summary}")).await;
    }

    pub async fn status_changed(&self, jira_key: &str, phase: &str) {
        self.notify("Kanban status changed", &format!("{jira_key}: {phase}")).await;
    }

    pub async fn awaiting_review(&self, jira_key: &str) {
        self.notify("Kanban awaiting review", jira_key).await;
    }

    pub async fn error(&self, jira_key: &str, message: &str) {
        self.notify("Kanban error", &format!("{jira_key}: {message}")).await;
    }
}
```

Export it from `crates/kanban-orchestrator/src/lib.rs` if not already exported:

```rust
pub mod notifier;
```

- [ ] **Step 2: Test using capture mode**

Create `crates/kanban-orchestrator/tests/notifier_test.rs`:

```rust
use kanban_orchestrator::notifier::Notifier;

#[tokio::test]
async fn captures_notify_calls_without_spawning_processes() {
    let (notifier, log) = Notifier::capture();
    notifier.status_changed("AP-1", "Coding").await;

    let log = log.lock().unwrap();
    assert_eq!(log[0].0, "Kanban status changed");
    assert_eq!(log[0].1, "AP-1: Coding");
}
```

- [ ] **Step 3: Run test**

Run:

```bash
cargo test -p kanban-orchestrator captures_notify_calls_without_spawning_processes
```

Expected: PASS. The test must not invoke `osascript`; platform-specific behavior is already covered by `services::services::notification`.

- [ ] **Step 4: Commit**

```bash
git add crates/kanban-orchestrator/src/lib.rs crates/kanban-orchestrator/src/notifier.rs crates/kanban-orchestrator/tests/notifier_test.rs
git commit -m "feat(kanban): reuse notification service"
```

---

### Task G2: `events` helper

**Files:**
- Modify: `crates/kanban-orchestrator/src/events.rs`

- [ ] **Step 1: Implement**

```rust
use sqlx::SqlitePool;
use uuid::Uuid;

pub async fn emit(
    pool: &SqlitePool,
    task_id: Uuid,
    event_type: &str,
    actor: &str,
    from_phase: Option<&str>,
    to_phase: Option<&str>,
    payload: Option<&serde_json::Value>,
) -> crate::Result<()> {
    let row = db::models::task_event::TaskEvent {
        id: Uuid::new_v4(),
        task_id,
        event_type: event_type.into(),
        from_phase: from_phase.map(String::from),
        to_phase: to_phase.map(String::from),
        actor: actor.into(),
        payload: payload.map(|v| v.to_string()),
        ts: chrono::Utc::now(),
    };
    db::models::task_event::TaskEvent::insert(pool, &row).await?;
    Ok(())
}
```

- [ ] **Step 2: Add unit test that emits and reads back**

- [ ] **Step 3: Commit**

```bash
git commit -am "feat(kanban): event emission helper"
```

---

### Task G3: Per-card log files

**Files:**
- Modify: `crates/kanban-orchestrator/src/dispatcher/phase.rs`

- [ ] **Step 1: After each `run_turn`, write `<vibe_kanban_data>/logs/<jira_key>/phase-<column>-turn-<N>.log`** with the captured stdout/stderr.

- [ ] **Step 2: Test that log file is created**

- [ ] **Step 3: Commit**

```bash
git commit -am "feat(kanban): per-card per-turn log files"
```

---

## Phase H: Recovery + Wiring + E2E

### Task H1: Startup recovery

**Files:**
- Create: `crates/kanban-orchestrator/src/recovery.rs`

- [ ] **Step 1: Failing test**

Seed a card with `phase_state='running', current_turn=3`. Call `recovery::reset_running(&pool)`. Assert card is now `phase_state='idle', current_turn=0`. AwaitingReview/error cards untouched.

- [ ] **Step 2: Implement**

```rust
pub async fn reset_running(pool: &sqlx::SqlitePool) -> crate::Result<u64> {
    let r = sqlx::query("UPDATE tasks SET phase_state='idle', current_turn=0 WHERE phase_state='running'")
        .execute(pool).await?;
    Ok(r.rows_affected())
}
```

- [ ] **Step 3: Commit**

```bash
git commit -am "feat(kanban): startup recovery resets interrupted runs"
```

---

### Task H2: Wire orchestrator into current server startup

**Files:**
- Modify: `crates/server/src/main.rs`
- Modify: `crates/kanban-orchestrator/src/runtime.rs`
- Modify: `crates/kanban-orchestrator/src/lib.rs` if new runtime types need exporting

There is no `crates/server/src/startup.rs` and no `AppState`. Startup wiring must happen in `crates/server/src/main.rs` after `DeploymentImpl::new()` and existing cleanup/backfill work, before `routes::router(deployment.clone())`.

- [ ] **Step 1: Add runtime start handle**

Extend `crates/kanban-orchestrator/src/runtime.rs`:

```rust
use std::sync::OnceLock;

use db::DBService;
use uuid::Uuid;

use crate::scheduler::ManualTrigger;

static KANBAN_TRIGGER: OnceLock<ManualTrigger> = OnceLock::new();

#[derive(Clone)]
pub struct RuntimeHandle {
    trigger: ManualTrigger,
}

impl RuntimeHandle {
    pub fn trigger(&self) -> ManualTrigger {
        self.trigger.clone()
    }
}

pub async fn start(db: DBService) -> crate::Result<RuntimeHandle> {
    crate::recovery::reset_running(&db.pool).await?;

    let trigger = ManualTrigger::new();
    let _ = KANBAN_TRIGGER.set(trigger.clone());

    let scheduler = crate::scheduler::Scheduler::new_from_db(db.clone(), trigger.clone()).await?;
    tokio::spawn(async move {
        if let Err(err) = scheduler.run().await {
            tracing::error!(?err, "kanban scheduler stopped");
        }
    });

    trigger.fire();
    Ok(RuntimeHandle { trigger })
}

pub async fn trigger_poll_now(_db: &DBService) -> crate::Result<()> {
    if let Some(trigger) = KANBAN_TRIGGER.get() {
        trigger.fire();
    }
    Ok(())
}

pub async fn sync_task_now(db: &DBService, task_id: Uuid) -> crate::Result<()> {
    crate::scheduler::tick::sync_task_now(db, task_id).await
}

pub async fn poll_once(db: &DBService, project: String) -> crate::Result<()> {
    tracing::info!(project, "running one kanban poll tick");
    crate::scheduler::tick::do_tick_for_project(db, &project).await
}
```

This uses a process-local `OnceLock` because current routes only receive `DeploymentImpl`. Do not introduce `AppState` just for this feature.

- [ ] **Step 2: Start runtime from `main.rs`**

Modify `crates/server/src/main.rs` after the existing backfill calls and before analytics/session-start tracking:

```rust
if std::env::var("VK_DISABLE_KANBAN_ORCHESTRATOR").is_ok() {
    tracing::info!("Kanban orchestrator disabled by VK_DISABLE_KANBAN_ORCHESTRATOR");
} else if let Err(err) = kanban_orchestrator::runtime::start(deployment.db().clone()).await {
    tracing::warn!(?err, "Kanban orchestrator did not start");
}
```

Keep failure non-fatal in v1 so a missing `.agents/kanban-workflows` directory or Jira credentials does not prevent normal vibe-kanban startup.

- [ ] **Step 3: Add server dependency if not already added**

In `crates/server/Cargo.toml`:

```toml
kanban-orchestrator = { path = "../kanban-orchestrator" }
```

- [ ] **Step 4: Manual smoke test**

Run:

```bash
VK_DISABLE_KANBAN_ORCHESTRATOR=1 cargo run -p server
```

Expected: server starts normally and logs `Kanban orchestrator disabled by VK_DISABLE_KANBAN_ORCHESTRATOR`.

Then run without the env var using a real `.agents/kanban-workflows/AP.yml`:

```bash
JIRA_EMAIL=... JIRA_API_TOKEN=... cargo run -p server
```

Expected: server starts normally; logs include scheduler startup and initial poll trigger. Missing workflow files should warn, not crash the server.

- [ ] **Step 5: Commit**

```bash
git add crates/server/Cargo.toml crates/server/src/main.rs crates/kanban-orchestrator/src/lib.rs crates/kanban-orchestrator/src/runtime.rs
git commit -m "feat(server): boot kanban orchestrator at startup"
```

---

### Task H3: End-to-end integration test

**Files:**
- Create: `crates/kanban-orchestrator/tests/e2e_test.rs`

- [ ] **Step 1: Test scenario**

1. Start `wiremock` Jira returning 1 issue in `To Do`.
2. Build `OrchestratorContext` with `qa_mock` executor scripted to emit `<<KANBAN_PHASE_COMPLETE>>` on turn 1.
3. Call `do_tick`. Assert: card created in `Todo`, dispatch runs analyzer, advances to `Coding`.
4. Call `do_tick` again. Assert: dispatch runs coder, hits `on_complete: review` → card in `awaiting_review`.
5. Call `api::approve`. Assert card moves to `Reviewing`.
6. Call `do_tick`. Assert: reviewer runs, advances to `Done` and `jira_transition: "Done"` was called on Jira mock.

- [ ] **Step 2: Implement test (aim for ~150 LOC)**

- [ ] **Step 3: Commit**

```bash
git commit -am "test(kanban): full e2e poll → dispatch → review → done"
```

---

### Task H4: Hooks (pre_dispatch / post_complete / on_error)

**Files:**
- Modify: `crates/kanban-orchestrator/src/scheduler/tick.rs` (call hooks around `run_phase`)

- [ ] **Step 1: Test that pre_dispatch shell hook runs and its failure aborts the phase**

- [ ] **Step 2: Implement**

```rust
async fn run_hook(cmd: &str, cwd: &std::path::Path) -> crate::Result<()> {
    if cmd.is_empty() { return Ok(()); }
    let status = tokio::process::Command::new("sh").arg("-c").arg(cmd)
        .current_dir(cwd).status().await?;
    if !status.success() {
        return Err(crate::OrchestratorError::Other(anyhow::anyhow!("hook failed: {}", cmd)));
    }
    Ok(())
}
```

Call sites: before `run_phase`, after `Complete`, on `Failed`.

- [ ] **Step 3: Commit**

```bash
git commit -am "feat(kanban): workflow hooks (pre_dispatch / post_complete / on_error)"
```

---

## Self-Review Notes

After completing all tasks:

1. **Spec coverage**: every section in `SPEC-kanban-orchestrator.md` is mapped to at least one task above:
   - §3 Layered Semantics → Task A3 (column has `jira_status`, `jira_transition`)
   - §4 System Overview → Task A1, E1, E2
   - §5 Configuration Contract → Task A3, A4, A5
   - §6 Data Model → Task A2; State Machine → Task D4, E3; Concurrency → Task D3
   - §7 Control Plane API → Task F1, F2
   - §8 Error Handling → Task D4 (failure marker), E3 (DB transitions), H4 (hook errors)
   - §9 Observability → Task G1, G2, G3
   - §10 Restart Recovery → Task H1, H2
   - §11 Testing Strategy → Tasks include unit, integration, e2e
   - §12 Borrowed from Symphony → Hooks (H4), recovery (H1), validation (A4), --once (F2), policy-in-repo (A5)

2. **Placeholder scan**: Remaining implementation-dependent details are called out at integration seams only: the D5 adapter may need a small accessor adjustment for `command_group::AsyncGroupChild`, and F2 is blocked until Phase E exposes `do_tick_for_project`. No task depends on `startup.rs`, `AppState`, or a non-existent clap CLI parser.

3. **Type consistency**: `PhaseState` (db model) and `OnComplete` (config) values agree across tasks. `PhaseOutcome` variants used identically in D4 and E3. `task_events.event_type` strings (`card_created`, `turn_started`, `phase_advanced`, `awaiting_review`, `error`) are referenced consistently.

4. **Open follow-ups (not blocking v1)**:
   - UI for awaiting_review badge / activity timeline — `frontend/src/components/tasks/`
     (the `packages/local-web` layout from the post-0.1.14 branch does not exist in this fork).
   - Prometheus metrics endpoint.
   - Multi-tracker adapter trait (Linear, GitHub).
   - SQLite → Postgres parity for `crates/remote` (currently `exclude`d from workspace).

5. **Current-codebase alignment notes (verified 2026-05-11)**:
   - `crates/worktree-manager` is intentionally not used; worktree/container integration points are `crates/services/src/services/worktree_manager.rs`, `crates/services/src/services/workspace_manager.rs`, `crates/services/src/services/container.rs`, `crates/local-deployment/src/container.rs`, and `crates/git/`.
   - Migration SQL is SQLite-oriented (`TEXT` JSON, RFC3339 timestamps, UUIDs through existing SQLx/ts-rs patterns) and does not use Postgres-only `JSONB` or `TIMESTAMPTZ`.
   - v1 startup and routes are local-server only: `crates/server/src/main.rs`, `crates/server/src/routes/mod.rs`, `crates/server/src/routes/kanban.rs`, and `crates/local-deployment`. `crates/remote` / Electric parity remains a follow-up.
