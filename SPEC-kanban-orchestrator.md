# vibe-kanban Orchestrator Spec — Jira Poller + Phase Dispatch Loop

Status: Draft v1 (Revised 2026-05-11)
Date: 2026-05-07
Inspired by: [Symphony](https://github.com/lorne-luo/symphony) (Elixir orchestrator for coding agents)

## 0. Revision Notes (2026-05-11 — repo reset to 0.1.14)

The repo has been reset from the earlier orchestrator branch (HEAD `bad0eeb36`, version 0.1.44)
back to upstream `f9914f57d` (version 0.1.14). Changes vs. the original Draft v1:

- `crates/worktree-manager` no longer exists in the workspace; per-card worktree handling now
  lives inside `crates/git` and `crates/services`. References below have been updated.
- Local store is SQLite only (`crates/db` uses the `sqlite` SQLx feature; `crates/remote` is
  `exclude`d from the workspace). Migration types in §6.1 are now SQLite-native; the Postgres
  variant is deferred to a follow-up alongside `crates/remote`.
- New crate location is **fixed**: `crates/kanban-orchestrator`, auto-started from
  `crates/server/src/main.rs`, control plane mounted at `crates/server/src/routes/kanban.rs`.
  §13 Open Item resolved.
- v1 ships on `local-deployment` only. The `remote` deployment path (Electric + Postgres) is
  out of scope for v1.
- All new Rust enums/structs that cross the API boundary must be regenerated into
  `shared/types.ts` via `crates/server/src/bin/generate_types.rs` (per CLAUDE.md).
- This SPEC targets the `lorne` fork pinned at 0.1.14. Upstream `main` is being sunset (project
  routes moved to export-only on `97123d526`); rebasing onto post-sunset commits is out of scope.

## 1. Purpose

Add a long-running orchestration layer to vibe-kanban that:

1. Periodically pulls tasks from a Jira active sprint into a vibe-kanban project as Kanban cards.
2. Mirrors Jira updates (status, comments, assignee, summary, attachments) to existing cards and
   reacts appropriately (inject context vs. stop running agent).
3. Dispatches a per-column-transition coding agent (`.agents/agent/<name>.md`) when a card enters
   a column whose workflow specifies an agent.
4. Provides explicit `awaiting_review` / `auto` semantics so each phase can require human approval
   before advancing.
5. Notifies the operator on macOS via `osascript` when cards are created or change status.

This spec defines vibe-kanban's responsibilities. The contents of `.agents/agent/<name>.md` are
implementation-defined per-repo; this spec only fixes the **interface** between vibe-kanban and
the agent file (frontmatter fields, completion markers, injected context files).

## 2. Goals and Non-Goals

### 2.1 Goals

- Poll Jira active sprint on a fixed cadence (default 15 min) with bounded concurrency.
- Treat Jira as the **business state-of-truth** and Kanban columns as **workflow phases** —
  the two are intentionally distinct semantic layers.
- Run each column's agent in its own session (state-machine model), with self-loop until completion
  marker or `max_turns_per_phase` (default 5).
- Support both `auto` and `review` gates between columns.
- Survive process restarts without an external coordinator: state is reconstructed from the DB,
  the worktree filesystem, and Jira.
- Provide a repo-owned `.agents/kanban-workflows/<project>.yml` so workflow policy is versioned
  alongside agent definitions.
- Provide a CLI `--once` mode for testing.
- macOS desktop notifications via `osascript` for card lifecycle events.

### 2.2 Non-Goals

- Two-way Jira ↔ Kanban automatic state sync. (Cards can drive Jira via explicit
  `jira_transition`; humans changing Jira drive Kanban via reconciliation. There is no
  symmetrical auto-mirror of Kanban column → Jira status.)
- A full workflow engine. The state machine is intentionally restricted to linear column
  transitions plus review gating.
- Replacing existing vibe-kanban executor abstractions. The new orchestrator dispatches into
  `crates/executors` unchanged.
- Multi-tracker support in v1 (Jira only; design leaves room for adapters later).
- Cross-card dependencies / blocking relationships.
- Remote deployment (`crates/remote`, Electric/Postgres). v1 runs only under
  `crates/local-deployment` with a SQLite store.
- Rebase compatibility with upstream `main` past the project-routes sunset (`97123d526`).

## 3. Layered Semantics

> **Kanban column = agent workflow phase.**
> **Jira status = business state.**

Mapping rules:

- **Inbound** (Jira → Kanban): a Jira issue's status determines whether a card is created and
  in which `initial: true` column it lands. After creation, Jira status changes only trigger
  reconciliation actions (see §6.2), they do **not** automatically move cards.
- **Outbound** (Kanban → Jira): when a card advances into a column with `jira_transition`,
  vibe-kanban executes the named transition on the Jira issue. This is the **only** way
  Kanban writes Jira status. Phase-internal turns never write Jira status; agents may write
  comments via their own tools (out of scope for vibe-kanban).
- **Comments**: vibe-kanban mirrors Jira comments into the card activity stream and may inject
  the latest unread comment into the agent's next turn. vibe-kanban does **not** post comments
  to Jira; agents do that themselves.

This layering avoids the conflict-resolution complexity of bidirectional sync.

## 4. System Overview

```
┌─────────────────────────────────────────────────────┐
│  vibe-kanban backend (Rust, tokio)                  │
│                                                     │
│  ┌──────────────────┐   every 15m  ┌─────────────┐  │
│  │ KanbanScheduler  │─────────────►│ JiraPoller  │  │
│  │ (tokio interval) │              │             │  │
│  │ + manual trigger │              └──────┬──────┘  │
│  └────────┬─────────┘                     │         │
│           │                               ▼         │
│           │                        ┌──────────────┐ │
│           │                        │ Reconciler   │ │
│           │                        └──────┬───────┘ │
│           ▼                               │         │
│  ┌──────────────────┐                     │         │
│  │ Dispatcher       │◄────────────────────┘         │
│  │ (concurrency     │                               │
│  │  gates + state   │      ┌─────────────────────┐  │
│  │  machine)        │─────►│ AgentExecutor       │  │
│  └────────┬─────────┘      │ (existing crate)    │  │
│           │                └─────────┬───────────┘  │
│           ▼                          │              │
│  ┌──────────────────┐                ▼              │
│  │ NotifierService  │      ┌─────────────────────┐  │
│  │ (macOS osascript)│      │ WorktreeManager     │  │
│  └──────────────────┘      │ (existing)          │  │
│                            └─────────────────────┘  │
└─────────────────────────────────────────────────────┘
                  ▲
                  │ reads
┌──────────────────────────────────────────┐
│ repo: .agents/                           │
│   kanban-workflows/<project>.yml         │
│   agent/<name>.md                        │
└──────────────────────────────────────────┘
```

### 4.1 New Components (proposed crate `crates/kanban-orchestrator`)

| Component | Responsibility |
|---|---|
| `KanbanScheduler` | Owns the `tokio::time::interval` loop and the `poll_now` trigger channel. |
| `JiraPoller` | Calls Jira REST `search` with the workflow's JQL; normalizes issues to a stable shape. |
| `Reconciler` | Diffs each issue against `tasks.jira_snapshot`; emits reconciliation events. |
| `Dispatcher` | Applies concurrency gates, owns the per-card `tokio::task` for phase execution, parses agent markers. |
| `NotifierService` | Wraps `osascript`; no-op on non-macOS hosts. |
| `WorkflowLoader` | Reads and validates `.agents/kanban-workflows/<project>.yml` at startup and on demand. |

### 4.2 Reused Components

- `crates/executors` — agent invocation (Claude/Codex/Cursor/etc.) unchanged; `qa_mock` used in tests.
- `crates/services` — per-card worktree/container lifecycle (formerly `worktree-manager`).
- `crates/git` — git operations.
- `crates/db` — SQLx + SQLite migrations.
- `crates/server` — HTTP API surface; new endpoints in §7, mounted via
  `crates/server/src/routes/kanban.rs`, scheduler boot from `crates/server/src/main.rs`.
- `crates/local-deployment` — process-level wiring; the orchestrator only runs here in v1.

## 5. Configuration Contract

### 5.1 `.agents/kanban-workflows/<project>.yml`

```yaml
version: 1
project: AP                          # vibe-kanban project slug

sync:
  source: jira
  jira:
    site: syfe.atlassian.net
    project_key: AP
    jql: "sprint in openSprints() AND assignee in (currentUser())"
    poll_interval: 15m
    auth_env:
      email: JIRA_EMAIL
      token: JIRA_API_TOKEN

columns:
  - name: Todo
    jira_status: ["To Do", "Open", "Backlog"]   # inbound mapping
    initial: true                                # new cards land here

  - name: Analyzing
    agent: analyzer
    on_complete: review                          # auto | review
    next: Coding
    jira_transition: "Start Progress"            # executed on entry to this column

  - name: Coding
    agent: coder
    on_complete: review
    next: Reviewing

  - name: Reviewing
    agent: reviewer
    on_complete: auto
    next: Done
    jira_transition: "Done"

  - name: Done
    terminal: true

defaults:
  workspace_strategy: per_issue_worktree
  max_concurrent_dispatches: 3
  max_per_column: 1
  max_turns_per_phase: 5
  retry:
    max_attempts: 3
    backoff: exponential
    base_ms: 1000
  notifications:
    macos: true
    events: [card_created, status_changed, awaiting_review, error]

reconciliation:
  on_jira_comment: inject_next_turn        # inject_next_turn | display_only
  on_jira_status_terminal: stop_immediately
  on_jira_assignee_change: stop_immediately
  on_jira_summary_change: inject_next_turn

hooks:                                     # optional shell commands
  pre_dispatch: "pnpm install --frozen-lockfile"
  post_complete: ""
  on_error: ""
```

### 5.2 Validation Rules (fail-fast at load)

- `version` MUST equal `1`.
- Exactly one column with `initial: true`.
- At least one column with `terminal: true`.
- For every non-terminal column, `next` MUST resolve to an existing column name.
- Every `agent` value MUST resolve to an existing `.agents/agent/<name>.md`.
- `on_complete` MUST be `auto` or `review`.
- `auth_env` env vars MUST be set in the process environment.
- `jira_transition` names are NOT validated against Jira at load time (avoids hard dependency
  on connectivity); failures are reported at runtime per §8.1.

### 5.3 `.agents/agent/<name>.md` Interface

Frontmatter consumed by vibe-kanban (any other fields are passed through to the agent runtime):

```markdown
---
name: coder
description: Phase agent for Kanban Coding column
review_required: true               # advisory; workflow's on_complete wins
allowed_tools: [edit, bash]
max_turns: 5                        # advisory; workflow's max_turns_per_phase wins
completion_marker: "<<KANBAN_PHASE_COMPLETE>>"
failure_marker: "<<KANBAN_PHASE_FAILED>>"
---

# Prompt body (free-form)
```

Markers default to `<<KANBAN_PHASE_COMPLETE>>` / `<<KANBAN_PHASE_FAILED reason="...">>` if
omitted.

### 5.4 Injected Context (written to worktree before each turn)

```
.kanban-context/
  task.json          # full card snapshot incl. jira_snapshot
  phase.md           # current column / previous column / expected next
  history.jsonl      # one line per completed phase (summary written by Dispatcher)
  reconcile.md       # diff of Jira changes since last turn (omitted if empty)
  review_feedback.md # operator's request_changes feedback (only after request_changes)
```

vibe-kanban guarantees:
- `task.json` and `reconcile.md` are refreshed before every turn.
- `history.jsonl` is appended to (not rewritten) when a phase completes.
- `review_feedback.md` is removed when the phase next succeeds.

The agent prompt is responsible for instructing the model to read these files; vibe-kanban
does not inject them into the conversation directly.

## 6. Data Model and State Machine

### 6.1 DB Migrations (additions to `tasks`)

Local store is SQLite. JSON columns use `TEXT` with `json()` validation; timestamps use
RFC3339 `TEXT` (matching existing migrations in `crates/db/migrations`); UUIDs use `BLOB` to
match the existing `tasks` schema. The Postgres equivalent (JSONB/TIMESTAMPTZ/UUID) is deferred
to the future `crates/remote` adaptation.

| Column | Type (SQLite) | Notes |
|---|---|---|
| `jira_key` | TEXT | Unique index, e.g. `AP-358` |
| `jira_snapshot` | TEXT (json) | Full mirror |
| `jira_synced_at` | TEXT (rfc3339) | Last successful reconcile |
| `kanban_phase` | TEXT | Current column name |
| `phase_state` | TEXT (enum) | `idle / running / awaiting_review / error / archived` |
| `current_turn` | INTEGER | Turn count within current phase |
| `last_executor_session_id` | BLOB NULL | UUID; joins to executors session log |
| `review_pending_since` | TEXT (rfc3339) NULL | UI sort key |
| `error_info` | TEXT (json) NULL | `{reason, attempts, last_error_at}` |
| `pending_inject` | TEXT (json) NULL | Queued reconciliation diff for next turn |

New table `task_events`:

| Column | Type (SQLite) |
|---|---|
| `id` | BLOB PK (UUID) |
| `task_id` | BLOB FK |
| `event_type` | TEXT (e.g. `phase_advanced`, `turn_started`, `marker_complete`, `error`, `review_approved`) |
| `from_phase` | TEXT NULL |
| `to_phase` | TEXT NULL |
| `actor` | TEXT (`scheduler`, `dispatcher`, `human:<user>`, `agent`) |
| `payload` | TEXT (json) |
| `ts` | TEXT (rfc3339) |

`phase_state` and `event_type` are Rust enums in `crates/kanban-orchestrator/src/...`; both
MUST be `#[derive(TS)]` and regenerated into `shared/types.ts` via
`crates/server/src/bin/generate_types.rs` (`pnpm run generate-types`).

### 6.2 Reconciliation Matrix

| Jira-side change | Default behavior | Configurable as |
|---|---|---|
| Issue appears in JQL result, no card | Create card in `initial` column; emit `card_created` notify | (not configurable) |
| Issue still present, comment added | Append to `jira_snapshot.comments`; queue inject for next turn | `on_jira_comment` |
| Issue still present, status moved to terminal (Done/Cancelled) | Set `phase_state=archived`; cancel any running phase at next turn boundary | `on_jira_status_terminal` |
| Issue still present, assignee changed away | Same as terminal | `on_jira_assignee_change` |
| Issue still present, summary/description changed | Update snapshot; queue inject | `on_jira_summary_change` |
| Issue no longer in JQL result | Set `phase_state=archived`; do NOT delete card | (not configurable) |
| Any status change (transition observed) | Emit `status_changed` notify | (not configurable) |

`stop_immediately` means: signal cooperative cancel; the next turn boundary check exits.
vibe-kanban does NOT hard-kill the executor process.

### 6.3 Card State Machine

```
       ┌────────────── (any state) ──────────────┐
       │                                          │
       ▼                                          │
      idle ── dispatch ──► running                │
                            │                     │
                            ├── complete_marker ──┼──► next column.idle
                            │                     │      (executes jira_transition if set)
                            │                     │
                            ├── max_turns ───────►│ awaiting_review
                            │   (if on_complete=  │      │
                            │   review)           │      ├─ approve ──► next column.idle
                            │                     │      └─ request_changes ──► running
                            │                     │
                            ├── fail_marker ─────►│ error (stays in current column)
                            │                     │
                            └── reconcile_stop ──►│ archived
```

Notes:
- `on_complete=auto` skips `awaiting_review`; the marker (or max_turns) advances directly.
- `error` requires explicit operator action (retry / move) via the API.
- `archived` is terminal but the card row is preserved for audit.

### 6.4 Concurrency Gates

| Gate | Default | Scope |
|---|---|---|
| `max_concurrent_dispatches` | 3 | Across all projects in the vibe-kanban instance |
| `max_per_column` | 1 | Per (project, column) |
| `max_turns_per_phase` | 5 | Per phase execution |

Cards exceeding gates remain `idle` and are reconsidered on the next tick.

## 7. Control Plane API

| Method + Path | Purpose |
|---|---|
| `POST /api/projects/:project_id/poll_now` | Trigger an immediate poll tick (bypasses 15m schedule) |
| `POST /api/tasks/:task_id/sync_now` | Reconcile a single card without polling the rest |
| `POST /api/tasks/:task_id/approve` | Resolve `awaiting_review`; advance to `next`; run `jira_transition` if set |
| `POST /api/tasks/:task_id/request_changes` body `{ feedback: string }` | Write `review_feedback.md`, set `phase_state=running`, reset `current_turn=0`, redispatch |
| `POST /api/tasks/:task_id/cancel` | Cooperative cancel; set `phase_state=archived` |
| `POST /api/tasks/:task_id/retry_error` | Reset `error_info`, `current_turn=0`, `phase_state=idle` |
| `GET /api/tasks/:task_id/events` | Stream `task_events` for the activity timeline |

CLI subcommand:

```
vibe-kanban kanban poll --once --project <slug>
```

Runs a single poll tick + dispatch wave synchronously, then exits. Used for tests and ad-hoc
reconciliation.

## 8. Error Handling

### 8.1 Error Classes

| Class | Behavior |
|---|---|
| Jira API transient (429/503/timeout) | Skip this tick; retry next tick. After 3 consecutive ticks failing, emit `error` notify. |
| Jira auth failure (401/403) | Pause scheduler for the affected project; UI flags credential error; notify. |
| Workflow yml load failure at startup | Project fails to start; vibe-kanban refuses to schedule it until fixed. |
| Workflow yml load failure on reload | Keep the previously-loaded version; surface a warning in the UI. |
| Worktree/executor startup failure | Mark card `error`, populate `error_info`, schedule retry per `defaults.retry`. |
| Executor crash / turn timeout | Same as above; turn count increments. |
| Agent emits failure marker | No retry. Card → `error`, notify. |
| `jira_transition` API failure on advance | Card still advances locally; `error_info.jira_transition_failed` set; UI warning; flow continues. |
| Concurrency gate saturation | Card stays `idle`; reconsidered next tick. |

### 8.2 Retry Policy

Exponential backoff per `defaults.retry`: attempt N waits `base_ms * 5^(N-1)` ms (default
1s, 5s, 25s) before re-dispatch. After `max_attempts`, the card stays in `error` and requires
operator action.

### 8.3 Cooperative Cancellation

Long-running turns are not killed mid-flight. Each turn boundary checks:
1. Is the card cancelled?
2. Does Jira reconciliation request stop?

If yes, the dispatcher returns without spawning the next turn and transitions the card to
`archived` (or whatever the cancel reason dictates).

## 9. Observability

### 9.1 Structured Logs (`tracing`)

Every event includes: `project, jira_key, kanban_phase, phase_state, turn, session_id, event`.

### 9.2 Per-Card Log Files

`<vibe_kanban_data>/logs/<jira_key>/phase-<column>-turn-<N>.log` retains full agent
stdout/stderr. Linked from the task detail UI.

### 9.3 Metrics (Prometheus, if enabled)

- `kanban_poll_duration_seconds` histogram
- `kanban_dispatched_total{column, outcome}` counter
- `kanban_phase_turns{column}` histogram
- `kanban_jira_api_errors_total{endpoint, code}` counter
- `kanban_concurrent_dispatches` gauge

### 9.4 Audit Stream

`task_events` table (§6.1) drives the UI activity timeline.

### 9.5 macOS Notifications

```rust
fn notify(title: &str, card: &Card) {
    if !cfg!(target_os = "macos") { return; }
    if !workflow.notifications.macos { return; }
    let body = format!("{} - {}", card.jira_key, card.summary);
    Command::new("osascript")
        .args(["-e", &format!(
            r#"display notification "{}" with title "vibe-kanban" subtitle "{}""#,
            escape(&body), escape(title))])
        .spawn().ok();
}
```

Triggered for events listed in `notifications.events`. Failures to spawn are swallowed.

## 10. Restart Recovery

State is reconstructed from DB + worktree filesystem + Jira (no separate persistence required):

1. On startup, load all enabled projects and validate each `.agents/kanban-workflows/*.yml`.
2. Set every `phase_state=running` card to `idle` and reset `current_turn=0`. The interrupted
   turn's stdout/stderr remains on disk for audit but is not replayed; the next dispatch starts
   the phase from turn 1. (Rationale: a partially-executed turn cannot be safely resumed without
   knowing how far the agent got; restarting the phase is simpler and bounded by `max_turns_per_phase`.)
3. Cards in `awaiting_review` and `error` are left untouched (operator-driven).
4. Worktrees and `.kanban-context/` directories are preserved; the next turn rewrites
   `task.json` and `reconcile.md` as needed.
5. Trigger an immediate poll tick (do not wait 15 min) to reconcile any Jira changes that
   happened during downtime.

This mirrors Symphony's "tracker + filesystem driven recovery" — no in-flight task state is
persisted, because it would inevitably go stale.

## 11. Testing Strategy

| Layer | Focus |
|---|---|
| Unit | yml schema parse + validation, state-machine transitions, Jira diff algorithm, marker parser, concurrency gate logic |
| Integration (Rust) | `wiremock`-backed Jira; full poll → dispatch → complete cycle using `crates/executors/qa_mock.rs`; verify `.kanban-context/` files |
| Restart recovery | Spawn dispatcher, kill mid-turn, restart, assert `idle` + re-dispatch + reconcile delta |
| End-to-end (`--once` CLI) | Single tick against a Jira sandbox; gated behind a `JIRA_E2E=1` env var in CI |
| Notifications | macOS-only test asserts `osascript` is invoked with expected args; non-macOS skips |

## 12. Borrowed from Symphony (Summary)

Adopted:
- Per-issue (per-card) workspace via existing worktree/container services (`crates/services` + `crates/git`).
- Bounded concurrency with backpressure.
- Tracker + filesystem driven restart recovery; no in-flight state persistence.
- Reconciliation that stops ineligible work cooperatively at turn boundaries.
- Repo-owned policy (`.agents/kanban-workflows/`) instead of DB-owned configuration.
- `--once` CLI mode for tests and manual reconciliation.
- Workflow validation at load time (fail-fast).
- Workspace lifecycle hooks (`pre_dispatch`, `post_complete`, `on_error`).

Not adopted:
- Erlang `disk_log` style persistence — vibe-kanban uses `tracing` file appenders and
  per-card directories instead.
- The Codex AppServer JSON-RPC bridge — vibe-kanban already has a richer `executors` abstraction
  and only needs stdout marker scanning on top.
- Single-session multi-phase agent — vibe-kanban uses a state-machine of independent agents,
  one per column edge, by design.

## 13. Open Items (deferred to plan / implementation)

- ~~Exact location of the new code~~ → **resolved**: new `crates/kanban-orchestrator` crate,
  auto-started from `crates/server/src/main.rs`, control plane in
  `crates/server/src/routes/kanban.rs`.
- Migration ordering for `tasks` columns and `task_events` table — sequence into a single
  new file `crates/db/migrations/<date>_add_kanban_orchestrator.sql` after the latest
  existing migration.
- UI surface for `awaiting_review`, `error_info`, and the activity timeline —
  `frontend/src/components/tasks/` (task card + detail panel). v1 may ship CLI-only and
  defer UI to a follow-up.
- Multi-tracker adapter trait shape (Linear, GitHub Issues, …) — out of scope for v1, but the
  `sync.source` field is reserved.
- Postgres / `crates/remote` parity for the new columns and `task_events` — separate spec.
