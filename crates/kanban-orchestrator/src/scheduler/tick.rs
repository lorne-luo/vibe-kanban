//! Scheduler tick body — Jira poll + reconcile + phase dispatch.
//!
//! `OrchestratorContext` is the single owned bundle of resources passed to
//! every tick.  It is cheaply clonable so that `tokio::spawn`ed per-card
//! tasks each get their own copy.

use std::{path::PathBuf, sync::Arc, time::Duration};

use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{
    config::Workflow,
    dispatcher::{
        PhaseExecutor,
        gate::Gate,
        phase::{PhaseInputs, PhaseOutcome, run_phase},
    },
    notifier::Notifier,
};

// ---------------------------------------------------------------------------
// Context
// ---------------------------------------------------------------------------

/// All shared resources for one orchestrator project.
///
/// Every field either derives `Clone` or is wrapped in `Arc`, so `clone()` is
/// cheap — just pointer bumps and a string copy for `repo_root`.
pub struct OrchestratorContext {
    pub pool: SqlitePool,
    pub workflow: Workflow,
    pub repo_root: PathBuf,
    pub jira: crate::jira::JiraClient,
    pub gate: Arc<Gate>,
    pub executor: Arc<dyn PhaseExecutor>,
    pub notifier: Arc<Notifier>,
    pub project_id: Uuid,
}

impl Clone for OrchestratorContext {
    fn clone(&self) -> Self {
        Self {
            pool: self.pool.clone(),
            workflow: self.workflow.clone(),
            repo_root: self.repo_root.clone(),
            jira: self.jira.clone(),
            gate: self.gate.clone(),
            executor: self.executor.clone(),
            notifier: self.notifier.clone(),
            project_id: self.project_id,
        }
    }
}

// ---------------------------------------------------------------------------
// do_tick — top-level entry called by Scheduler every interval
// ---------------------------------------------------------------------------

pub async fn do_tick(ctx: &OrchestratorContext) -> crate::Result<()> {
    // 1) Poll Jira and reconcile each issue
    let issues = ctx.jira.search(&ctx.workflow.sync.jira.jql).await?;

    for issue in &issues {
        let outcome = crate::reconciler::upsert_card_from_jira(
            &ctx.pool,
            ctx.project_id,
            &ctx.workflow,
            issue,
        )
        .await?;

        if outcome.created {
            ctx.notifier
                .notify("card_created", issue.key.as_str(), &issue.fields.summary);
            crate::events::emit(
                &ctx.pool,
                outcome.task_id,
                "card_created",
                "scheduler",
                None,
                None,
                None,
            )
            .await?;
        }

        if let Some(diff) = outcome.diff {
            let actions = crate::reconciler::decide_action(&diff, &ctx.workflow.reconciliation);
            for action in actions {
                use crate::reconciler::ReconcileAction;
                match action {
                    ReconcileAction::Stop => {
                        crate::reconciler::mark_archived(&ctx.pool, outcome.task_id).await?;
                    }
                    ReconcileAction::QueueInject(blob) => {
                        crate::reconciler::enqueue_inject(&ctx.pool, outcome.task_id, &blob)
                            .await?;
                    }
                    ReconcileAction::Notify(ev) => {
                        ctx.notifier
                            .notify(&ev, issue.key.as_str(), &issue.fields.summary);
                    }
                    ReconcileAction::UpdateSnapshot => {}
                }
            }
        }
    }

    // 2) Dispatch idle cards that belong to agent-driven columns
    dispatch_round(ctx).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// dispatch_round — find idle cards and spawn a phase task for each
// ---------------------------------------------------------------------------

async fn dispatch_round(ctx: &OrchestratorContext) -> crate::Result<()> {
    // UUIDs in the orchestrator are stored as TEXT (hyphenated string) to be
    // consistent with the common test infrastructure and the reconciler inserts.
    let project_id_str = ctx.project_id.to_string();
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, kanban_phase FROM tasks \
         WHERE phase_state = 'idle' AND kanban_phase IS NOT NULL AND project_id = ?",
    )
    .bind(&project_id_str)
    .fetch_all(&ctx.pool)
    .await?;

    for (task_id_str, column_name) in rows {
        let task_id = match Uuid::parse_str(&task_id_str) {
            Ok(id) => id,
            Err(_) => continue,
        };

        // Skip columns with no agent
        let Some(column) = ctx.workflow.column(&column_name) else {
            continue;
        };
        if column.agent.is_none() {
            continue;
        }

        // Try to acquire a concurrency permit; skip if at capacity
        let Some(_permit) = ctx.gate.try_acquire(&column_name).await else {
            continue;
        };

        // NOTE: _permit is moved into the spawned task so it lives for the
        // duration of the phase.
        let ctx2 = ctx.clone();
        tokio::spawn(async move {
            // Permit is implicitly dropped when this closure returns, releasing
            // the gate slot.
            let _permit = _permit;
            if let Err(e) = run_one_phase(&ctx2, task_id).await {
                tracing::error!(
                    error = %e,
                    task_id = %task_id,
                    "phase task failed"
                );
            }
        });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// run_one_phase — execute one full phase cycle for a single card
// ---------------------------------------------------------------------------

/// Lightweight row fetched by text UUID to avoid BLOB/TEXT type mismatch.
/// The orchestrator inserts task IDs as TEXT; sqlx's `Task::find_by_id` uses
/// a compiled macro that binds UUID as BLOB, so we use a plain text query.
struct CardRow {
    id: Uuid,
    title: String,
    kanban_phase: Option<String>,
    jira_key: Option<String>,
    current_turn: i64,
    pending_inject: Option<String>,
}

async fn fetch_card_by_text_id(pool: &SqlitePool, task_id: Uuid) -> crate::Result<Option<CardRow>> {
    let id_str = task_id.to_string();
    let row: Option<(
        String,
        String,
        Option<String>,
        Option<String>,
        i64,
        Option<String>,
    )> = sqlx::query_as(
        "SELECT id, title, kanban_phase, jira_key, current_turn, pending_inject \
             FROM tasks WHERE id = ?",
    )
    .bind(&id_str)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(
        |(id_s, title, kanban_phase, jira_key, current_turn, pending_inject)| CardRow {
            id: Uuid::parse_str(&id_s).unwrap_or(task_id),
            title,
            kanban_phase,
            jira_key,
            current_turn,
            pending_inject,
        },
    ))
}

async fn run_one_phase(ctx: &OrchestratorContext, task_id: Uuid) -> crate::Result<()> {
    // Load the card via text-based query (orchestrator stores IDs as TEXT)
    let card = match fetch_card_by_text_id(&ctx.pool, task_id).await? {
        Some(c) => c,
        None => return Ok(()),
    };

    let column_name = match card.kanban_phase.as_deref() {
        Some(n) => n.to_string(),
        None => return Ok(()),
    };
    let column = match ctx.workflow.column(&column_name) {
        Some(c) => c.clone(),
        None => return Ok(()),
    };
    if column.agent.is_none() {
        return Ok(());
    }

    let agent_name = column.agent.as_deref().unwrap();
    let agent_md = ctx
        .repo_root
        .join(format!(".agents/agent/{}.md", agent_name));
    let worktree = ctx.repo_root.join(format!("worktrees/{}", task_id));

    let id_str = task_id.to_string();

    // --- Mark running ---
    sqlx::query("UPDATE tasks SET phase_state = 'running' WHERE id = ?")
        .bind(&id_str)
        .execute(&ctx.pool)
        .await?;
    crate::events::emit(
        &ctx.pool,
        task_id,
        "turn_started",
        "dispatcher",
        card.kanban_phase.as_deref(),
        None,
        None,
    )
    .await?;

    let max_turns = column
        .max_turns_per_phase
        .or(ctx.workflow.defaults.max_turns_per_phase)
        .unwrap_or(5);

    // Consume and clear any queued inject blob
    let pending_inject = card.pending_inject.clone();
    sqlx::query("UPDATE tasks SET pending_inject = NULL WHERE id = ?")
        .bind(&id_str)
        .execute(&ctx.pool)
        .await?;

    // Build a minimal Task struct for PhaseInputs (only fields used by context writer)
    let task_for_phase = make_task_for_phase(&card);

    let cancel = Arc::new(tokio::sync::Notify::new());
    let inputs = PhaseInputs {
        card: &task_for_phase,
        column: &column,
        worktree: &worktree,
        agent_md: &agent_md,
        max_turns,
        turn_timeout: Duration::from_secs(60 * 30),
        reconcile_blob: pending_inject,
        review_feedback: None,
        cancel,
    };

    // Pre-dispatch hook (fatal — abort phase if hook fails)
    if let Some(pre) = ctx.workflow.hooks.pre_dispatch.as_deref() {
        run_hook(pre, &worktree).await?;
    }

    let outcome = run_phase(inputs, ctx.executor.as_ref()).await?;

    match outcome {
        PhaseOutcome::Complete {
            turns_used,
            last_session,
        } => {
            advance_card(ctx, &card, &column, turns_used, last_session).await?;
            // Post-complete hook (non-fatal)
            if let Some(post) = ctx.workflow.hooks.post_complete.as_deref() {
                let _ = run_hook(post, &worktree).await;
            }
        }
        PhaseOutcome::AwaitingReview {
            turns_used,
            last_session,
        } => {
            sqlx::query(
                "UPDATE tasks SET phase_state = 'awaiting_review', current_turn = ?, \
                 review_pending_since = datetime('now'), last_executor_session_id = ? \
                 WHERE id = ?",
            )
            .bind(turns_used as i64)
            .bind(last_session.to_string())
            .bind(&id_str)
            .execute(&ctx.pool)
            .await?;

            ctx.notifier.notify(
                "awaiting_review",
                card.jira_key.as_deref().unwrap_or(""),
                &card.title,
            );
            crate::events::emit(
                &ctx.pool,
                task_id,
                "awaiting_review",
                "dispatcher",
                card.kanban_phase.as_deref(),
                None,
                None,
            )
            .await?;
        }
        PhaseOutcome::Failed { reason, turns_used } => {
            let info = serde_json::json!({ "reason": reason, "turns": turns_used });
            sqlx::query(
                "UPDATE tasks SET phase_state = 'error', error_info = ?, current_turn = ? \
                 WHERE id = ?",
            )
            .bind(info.to_string())
            .bind(turns_used as i64)
            .bind(&id_str)
            .execute(&ctx.pool)
            .await?;

            ctx.notifier
                .notify("error", card.jira_key.as_deref().unwrap_or(""), &card.title);
            crate::events::emit(
                &ctx.pool,
                task_id,
                "error",
                "agent",
                None,
                None,
                Some(&info),
            )
            .await?;
            // On-error hook (non-fatal)
            if let Some(on_err) = ctx.workflow.hooks.on_error.as_deref() {
                let _ = run_hook(on_err, &worktree).await;
            }
        }
        PhaseOutcome::Cancelled => {
            sqlx::query("UPDATE tasks SET phase_state = 'archived' WHERE id = ?")
                .bind(&id_str)
                .execute(&ctx.pool)
                .await?;
        }
    }

    Ok(())
}

/// Build a minimal `db::models::task::Task` from `CardRow` for use in `PhaseInputs`.
/// Only the fields consumed by `write_context` and `append_history` need to be correct.
fn make_task_for_phase(card: &CardRow) -> db::models::task::Task {
    db::models::task::Task {
        id: card.id,
        project_id: Uuid::nil(), // not used by context writer
        title: card.title.clone(),
        description: None,
        status: db::models::task::TaskStatus::InProgress,
        parent_workspace_id: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        jira_key: card.jira_key.clone(),
        jira_snapshot: None,
        jira_synced_at: None,
        kanban_phase: card.kanban_phase.clone(),
        phase_state: db::models::task::PhaseState::Running,
        current_turn: card.current_turn,
        last_executor_session_id: None,
        review_pending_since: None,
        error_info: None,
        pending_inject: None,
    }
}

// ---------------------------------------------------------------------------
// run_hook — execute a shell hook command in the given working directory
// ---------------------------------------------------------------------------

async fn run_hook(cmd: &str, cwd: &std::path::Path) -> crate::Result<()> {
    if cmd.is_empty() {
        return Ok(());
    }
    let status = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(cwd)
        .status()
        .await
        .map_err(crate::OrchestratorError::Io)?;
    if !status.success() {
        return Err(crate::OrchestratorError::Workflow(format!(
            "hook failed: {}",
            cmd
        )));
    }
    Ok(())
}

/// Public test shim so integration tests can call `run_hook` directly.
#[cfg(feature = "test-utils")]
pub async fn run_hook_for_test(cmd: &str, cwd: &std::path::Path) -> crate::Result<()> {
    run_hook(cmd, cwd).await
}

// ---------------------------------------------------------------------------
// advance_card — move card to next column after a successful phase
// ---------------------------------------------------------------------------

async fn advance_card(
    ctx: &OrchestratorContext,
    card: &CardRow,
    column: &crate::config::Column,
    turns_used: u32,
    last_session: Uuid,
) -> crate::Result<()> {
    let next = column.next.as_deref().ok_or_else(|| {
        crate::OrchestratorError::Workflow("non-terminal column missing next".into())
    })?;

    let id_str = card.id.to_string();

    // Attempt Jira transition (log but don't abort on failure)
    if let Some(transition) = column.jira_transition.as_deref() {
        if let Some(key) = card.jira_key.as_deref() {
            if let Err(e) = ctx.jira.transition(key, transition).await {
                let info = serde_json::json!({
                    "jira_transition_failed": e.to_string(),
                    "transition": transition,
                });
                sqlx::query("UPDATE tasks SET error_info = ? WHERE id = ?")
                    .bind(info.to_string())
                    .bind(&id_str)
                    .execute(&ctx.pool)
                    .await?;
            }
        }
    }

    sqlx::query(
        "UPDATE tasks \
         SET kanban_phase = ?, phase_state = 'idle', current_turn = 0, \
             last_executor_session_id = ? \
         WHERE id = ?",
    )
    .bind(next)
    .bind(last_session.to_string())
    .bind(&id_str)
    .execute(&ctx.pool)
    .await?;

    crate::events::emit(
        &ctx.pool,
        card.id,
        "phase_advanced",
        "dispatcher",
        card.kanban_phase.as_deref(),
        Some(next),
        None,
    )
    .await?;

    tracing::info!(
        task_id = %card.id,
        from = card.kanban_phase.as_deref().unwrap_or("?"),
        to = next,
        turns = turns_used,
        "card advanced to next phase"
    );

    Ok(())
}
