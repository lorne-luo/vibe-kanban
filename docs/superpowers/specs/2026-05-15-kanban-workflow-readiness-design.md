# Kanban Workflow Readiness — Design

**Date:** 2026-05-15
**Status:** Approved

## Problem

A vibe-kanban project added through the UI shows `Workflow not ready` symptoms even when `.agents/kanban-workflows/AP.yml` is configured in the repo. There is no signal in the frontend to tell the user whether the kanban orchestrator can actually load the workflow.

Two underlying causes:

1. The kanban orchestrator runtime filters projects by `projects.default_agent_working_dir`, which is empty for projects created through the current API. Workflow files are never reached.
2. Even if loading was attempted, validation errors are only visible in backend logs.

## Goals

- Each repo manages its own agent: workflow YAMLs live in `<repo>/.agents/kanban-workflows/`. No project-level `default_agent_working_dir` selector in the UI.
- On project creation, immediately read every workflow file and validate it.
- Surface readiness as a badge on the **project list** and **kanban page**.
- Soft-fail: project creation succeeds even when validation fails; the badge communicates the problem.

## Non-Goals

| Out of scope | Reason |
|---|---|
| Filesystem watcher / fs-notify | Strategy A chosen: recompute every request |
| Persisting `workflow_status` to DB | Same as above |
| Environment variable validation (C-level) | Out of scope for this iteration; only B-level (YAML parse + agent file existence) |
| Refactoring `kanban-orchestrator` runtime to per-repo tick | Existing `default_agent_working_dir` single-dir loop continues to work; per-repo dispatch is a follow-up |
| Adding `default_agent_working_dir` field to UI forms | Decision was "each repo manages its own agent"; UI does not expose this |
| Auto-populating `default_agent_working_dir` | Out of scope; new readiness check and existing runtime are two parallel systems for this iteration |

## Design

### Decisions

| Decision | Choice | Why |
|---|---|---|
| Workflow scan scope | Per-repo | Aligns with "each repo manages its own agent" |
| Validation level | B (YAML parse + referenced agent file exists) | Strictest level that is pure FS, no runtime state |
| Storage / refresh | A — recompute on every request | Simplest, no migration, no staleness window |
| Creation failure handling | A — soft warning (200 + status payload) | Doesn't block users who add a project before finishing workflow config |

### Backend

#### Data types

New module: `crates/services/src/services/workflow_status.rs`

```rust
#[derive(Serialize, Deserialize, TS, Debug, Clone)]
#[ts(export)]
pub struct ProjectWorkflowStatus {
    pub state: ProjectWorkflowState,    // Ready | NotConfigured | Invalid
    pub repos: Vec<RepoWorkflowStatus>,
    pub checked_at: DateTime<Utc>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone)]
#[ts(export)]
pub struct RepoWorkflowStatus {
    pub repo_id: Uuid,
    pub repo_name: String,
    pub workflows_dir: String,          // absolute path for tooltips
    pub state: RepoWorkflowState,       // NotConfigured | Ready | Invalid
    pub workflows: Vec<WorkflowEntry>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone)]
#[ts(export)]
pub struct WorkflowEntry {
    pub name: String,                   // file stem (e.g. "AP")
    pub state: WorkflowEntryState,      // Ok | InvalidYaml | MissingAgentFile
    pub error: Option<String>,          // human-readable explanation
}
```

#### Existing validation (verified from source)

`kanban_orchestrator::config::load_workflow(repo_root, project)`:

- Reads `<repo_root>/.agents/kanban-workflows/<project>.yml`
- `serde_yaml::from_str` parse
- Scans agent prompt files from `<repo_root>/.agents/agent/` (singular `agent`, file stems = available agent names)
- Calls `Workflow::validate(&known_agents, &env_present)`:
  - schema (version, exactly one `initial` column, ≥1 terminal column, `next` chain valid)
  - referenced `agent:` exists in `known_agents` (B-level)
  - `sync.jira.auth_env.email` + `sync.jira.auth_env.token` env vars present (C-level)
- Returns `OrchestratorError::Workflow(String)` on any failure, message prefix indicates the cause.

C-level env checking is coupled inside `validate()`. To do B-only without duplicating logic, add a sibling in `kanban_orchestrator::config`:

```rust
pub fn load_workflow_with_env_probe(
    repo_root: &Path,
    project: &str,
    env_present: &dyn Fn(&str) -> bool,
) -> crate::Result<Workflow>
```

Existing `load_workflow` becomes `load_workflow_with_env_probe(repo_root, project, &|n| std::env::var(n).is_ok())`. The readiness service calls it with `&|_| true` so missing env vars do not flip a workflow to `Invalid`.

#### Compute function

```rust
pub async fn compute_for_project(
    db: &PgPool,
    project_id: Uuid,
) -> Result<ProjectWorkflowStatus, Error>
```

Algorithm:

1. Load project's `project_repos` rows.
2. For each repo, set `state` via these rules on `<repo.git_repo_path>/.agents/kanban-workflows/`:
   - repo path does not exist or is not a directory → `Invalid`, error `"repo path inaccessible: <path>"`, `workflows: []`.
   - workflows dir does not exist → `NotConfigured`, `workflows: []`.
   - workflows dir exists but no `*.yml` files → `NotConfigured`, `workflows: []`.
   - workflows dir has yml files: scan each. For each yml, call `load_workflow_with_env_probe(repo_root, file_stem, &|_| true)`:
     - `Ok` → `WorkflowEntry { state: Ok, error: None }`
     - `Err(OrchestratorError::Workflow(msg))` → map by message prefix:
       - starts with `"agent file"` → `MissingAgentFile`
       - all other validate/parse failures → `InvalidYaml`
     - any I/O or unexpected error → `InvalidYaml` with the raw message
   - Repo state then = `Ready` if all entries `Ok`, else `Invalid`.
3. Aggregate to project state:
   - Any workflow entry not `Ok` (i.e. any repo `Invalid`) → `Invalid`
   - All repos `NotConfigured` → `NotConfigured`
   - Otherwise → `Ready`

I/O uses `tokio::fs` for directory scans; the underlying `load_workflow_with_env_probe` is sync (`std::fs`) and runs inside `tokio::task::spawn_blocking` per yml to keep the async runtime clean. Per-repo work runs sequentially (yml count per repo is small).

Error-prefix matching is fragile but pragmatic — the alternative (introducing typed error variants in `OrchestratorError`) is a larger refactor and out of scope. The mapping lives in one helper in `workflow_status.rs` and is unit-tested.

#### API integration

No new endpoint. Status is attached to existing project responses via a wrapper:

```rust
#[derive(Serialize, TS)]
pub struct ProjectWithStatus {
    #[serde(flatten)]
    pub project: Project,
    pub workflow_status: ProjectWorkflowStatus,
}
```

Endpoints changed:

- `GET /api/projects` → `Vec<ProjectWithStatus>`
- `GET /api/projects/:id` → `ProjectWithStatus`
- `POST /api/projects` → `ProjectWithStatus`

`update_project` and other handlers are unchanged.

#### Creation hook

`create_project` handler:

1. Insert project + repos as today.
2. Call `workflow_status::compute_for_project(db, project.id)`.
3. Return `ProjectWithStatus`. Always HTTP 200 on successful insert, regardless of workflow state.

### Frontend

#### Type generation

Run `pnpm run generate-types` once after backend types are in place. `shared/types.ts` gains `ProjectWithStatus`, `ProjectWorkflowStatus`, `RepoWorkflowStatus`, `WorkflowEntry`, and the three state enums. All existing `projectsApi.getAll()` / `getById()` callers will type-error against the wrapper, surfacing the touch points.

#### Component: `WorkflowStatusBadge`

New file: `frontend/src/components/projects/WorkflowStatusBadge.tsx`

Props:

```ts
type Props = {
  status: ProjectWorkflowStatus;
  variant: 'dot' | 'pill';
  onClickDetails?: () => void;
};
```

Visual:

| State | Dot | Pill | Tooltip |
|---|---|---|---|
| `Ready` | green | `🟢 Ready` | `Workflow agent ready (N workflows)` |
| `Invalid` | red | `🔴 Issues` | `Workflow has issues — click for details` |
| `NotConfigured` | gray | `⚪ No workflow` | `No .agents/kanban-workflows/ configured` |

Uses `lucide-react` Circle (filled) + Radix Tooltip primitives already in the codebase.

#### Project list (`Projects.tsx` / `ProjectCard.tsx`)

Add `<WorkflowStatusBadge variant="dot" status={project.workflow_status} />` to the right of each project card title. Clicking navigates to `/local-projects/:id` and opens the details dialog.

#### Kanban page (`ProjectTasks.tsx`)

- Header right side: `<WorkflowStatusBadge variant="pill" status={...} onClickDetails={openDialog} />`.
- When `state === 'Invalid'`, render a collapsible Alert near the existing reconnecting-Alert region:
  ```
  ⚠ Workflow agent not ready
    AP.yml: agent file `coder.md` not found  [Show all]
  ```
- Details dialog (`@/components/ui/dialog`):
  ```
  Workflow status — checked at 19:25:14

  ▾ /Users/lorneluo/Workspace/swf/dev/  (repo: dev)
     • AP                    ❌ Invalid YAML: missing field 'agents.coder'
     • B2B                   ❌ Agent file not found: prompts/reviewer.md
     • Maintenance           ✅ Ready
  ```

#### Refresh strategy

Strategy A — no new polling. Existing `useProject()` already revalidates on route change / window focus (project list uses similar hooks). User reloading the page or switching back from another tab refreshes status naturally.

A `Reload now` button inside the details dialog invokes the existing project-query `refetch()` so users can force-refresh after editing yml externally without leaving the page.

#### Creation toast

In the project creation submit handler:

```ts
const created = await projectsApi.create(payload);
toast.success(`Project "${created.name}" created`);

if (created.workflow_status.state === 'Invalid') {
  toast.warning('Workflow agent not ready — check .agents/kanban-workflows/', {
    action: { label: 'Details', onClick: () => navigate(`/local-projects/${created.id}`) },
  });
}
```

Uses existing `sonner` toast library.

### Error handling

- Repo path inaccessible → `RepoWorkflowState::Invalid`, error `"repo path inaccessible: <path>"`. One bad repo does not crash the whole `GET /api/projects`.
- `load_workflow()` returns `Result`; bubble error string into the matching `WorkflowEntryState`.
- Project with zero repos → `ProjectWorkflowState::NotConfigured`, empty repos array.

### Performance

Worst case per list request: `N_projects × N_repos × N_yml` file reads. With N≈10 projects, expected <50 ms on local SSD. No caching this iteration; revisit if N grows past ~50.

### Tests

Backend (`crates/services/src/services/workflow_status.rs`):
- repo without `.agents/` → repo `NotConfigured` → project `NotConfigured`
- repo with `.agents/kanban-workflows/` but zero yml → `NotConfigured`
- repo path does not exist → repo `Invalid` with "repo path inaccessible"
- single valid yml + matching `.agents/agent/<name>.md` → `Ready`
- yml syntax error → `Invalid` with `InvalidYaml` entry
- yml references agent name with no matching `.agents/agent/<name>.md` → `Invalid` with `MissingAgentFile` entry
- yml with `jira.auth_env` env vars unset → still `Ready` (env probe stubbed to true; confirms B-only behaviour)
- multi-repo project: one repo Ready + one repo Invalid → project-level `Invalid`
- multi-repo project: all repos NotConfigured → project `NotConfigured`

Use `tempfile` (already a dep) — same style as `crates/kanban-orchestrator/tests/config_test.rs`.

Frontend:
- Vitest snapshot for `WorkflowStatusBadge` covering the three states.

### Implementation order

1. Add `load_workflow_with_env_probe` in `crates/kanban-orchestrator/src/config.rs`; refactor existing `load_workflow` to delegate. Keep existing test green.
2. New `crates/services/src/services/workflow_status.rs` + unit tests (covers cases listed above)
3. ts-rs derive on new types + `pnpm run generate-types`
4. Update `GET /api/projects`, `GET /api/projects/:id`, `POST /api/projects` handlers to return `ProjectWithStatus`
5. `WorkflowStatusBadge` component + Vitest snapshot
6. Project list integration
7. Kanban page pill + Alert + details dialog
8. Creation toast warning
9. Run `pnpm run check`, `cargo test --workspace`, `pnpm run lint`, `pnpm run format`
