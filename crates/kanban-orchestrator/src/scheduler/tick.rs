use std::sync::Arc;
use std::path::PathBuf;

use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::config::Workflow;
use crate::dispatcher::gate::Gate;
use crate::dispatcher::PhaseExecutor;
use crate::jira::JiraClient;
use crate::notifier::Notifier;

pub struct OrchestratorContext {
    pub pool: SqlitePool,
    pub workflow: Workflow,
    pub repo_root: PathBuf,
    pub jira: Arc<JiraClient>,
    pub gate: Arc<Gate>,
    pub executor: Arc<dyn PhaseExecutor>,
    pub notifier: Arc<Notifier>,
    pub project_id: Uuid,
}

impl OrchestratorContext {
    pub fn clone_for_spawn(&self) -> Self {
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

pub async fn do_tick(ctx: &OrchestratorContext) -> crate::Result<()> {
    // 1) Poll Jira
    let issues = ctx.jira.search(&ctx.workflow.sync.jira.jql).await?;

    // 2) Reconcile each issue
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
                .notify(
                    "New card",
                    &format!("{}: {}", issue.key, issue.fields.summary),
                )
                .await;
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
            let actions =
                crate::reconciler::decide_action(&diff, &ctx.workflow.reconciliation);
            for a in actions {
                use crate::reconciler::ReconcileAction;
                match a {
                    ReconcileAction::Stop => {
                        crate::reconciler::mark_archived(&ctx.pool, outcome.task_id).await?;
                    }
                    ReconcileAction::QueueInject(blob) => {
                        crate::reconciler::enqueue_inject(
                            &ctx.pool,
                            outcome.task_id,
                            &blob,
                        )
                        .await?;
                    }
                    ReconcileAction::Notify(ev) => {
                        ctx.notifier
                            .notify(
                                &ev,
                                &format!("{}: {}", issue.key, issue.fields.summary),
                            )
                            .await;
                    }
                    ReconcileAction::UpdateSnapshot => {}
                }
            }
        }
    }

    // 3) Dispatch idle cards
    dispatch_round(ctx).await?;
    Ok(())
}

pub async fn dispatch_round(ctx: &OrchestratorContext) -> crate::Result<()> {
    use sqlx::Row;
    let project_id_bytes = ctx.project_id.as_bytes().to_vec();
    let rows = sqlx::query(
        "SELECT id, kanban_phase FROM tasks \
         WHERE phase_state = 'idle' AND kanban_phase IS NOT NULL AND project_id = ?",
    )
    .bind(project_id_bytes)
    .fetch_all(&ctx.pool)
    .await?;

    for row in rows {
        let id_bytes: Vec<u8> = row.try_get("id").unwrap();
        let task_id = Uuid::from_slice(&id_bytes).unwrap();
        let column_name: String = row.try_get("kanban_phase").unwrap();

        let Some(column) = ctx.workflow.column(&column_name) else {
            continue;
        };
        if column.agent.is_none() {
            continue;
        }
        let Some(_permit) = ctx.gate.try_acquire(&column_name).await else {
            continue;
        };

        let ctx_clone = ctx.clone_for_spawn();
        tokio::spawn(async move {
            if let Err(e) = run_one_phase(&ctx_clone, task_id).await {
                tracing::error!(?e, "phase failed for task {}", task_id);
            }
        });
    }
    Ok(())
}

async fn run_one_phase(ctx: &OrchestratorContext, task_id: Uuid) -> crate::Result<()> {
    use sqlx::Row;

    // Fetch the task
    let id_bytes = task_id.as_bytes().to_vec();
    let row = sqlx::query(
        "SELECT id, project_id, title, description, status, parent_workspace_id, \
         created_at, updated_at, jira_key, jira_snapshot, jira_synced_at, kanban_phase, \
         phase_state, current_turn, last_executor_session_id, review_pending_since, \
         error_info, pending_inject \
         FROM tasks WHERE id = ?",
    )
    .bind(id_bytes)
    .fetch_optional(&ctx.pool)
    .await?;

    let Some(row) = row else {
        return Err(crate::OrchestratorError::Workflow(format!(
            "task {} not found",
            task_id
        )));
    };

    let kanban_phase: Option<String> = row.try_get("kanban_phase").ok().flatten();
    let pending_inject: Option<String> = row.try_get("pending_inject").ok().flatten();
    let jira_key: Option<String> = row.try_get("jira_key").ok().flatten();
    let title: String = row.try_get("title").unwrap_or_default();

    let column_name = kanban_phase.as_deref().ok_or_else(|| {
        crate::OrchestratorError::Workflow("task has no kanban_phase".into())
    })?;
    let column = ctx
        .workflow
        .column(column_name)
        .ok_or_else(|| {
            crate::OrchestratorError::Workflow(format!("unknown column: {column_name}"))
        })?;
    let agent_name = column.agent.as_deref().ok_or_else(|| {
        crate::OrchestratorError::Workflow("column has no agent".into())
    })?;
    let agent_md = ctx
        .repo_root
        .join(format!(".agents/agent/{}.md", agent_name));

    // Use a temp directory as the worktree for now
    let worktree_dir = ctx
        .repo_root
        .join(format!(".kanban-worktrees/{}", task_id));
    std::fs::create_dir_all(&worktree_dir)?;

    // Mark running
    let id_bytes2 = task_id.as_bytes().to_vec();
    sqlx::query(
        "UPDATE tasks SET phase_state='running', pending_inject=NULL WHERE id=?",
    )
    .bind(id_bytes2)
    .execute(&ctx.pool)
    .await?;
    crate::events::emit(
        &ctx.pool,
        task_id,
        "turn_started",
        "dispatcher",
        kanban_phase.as_deref(),
        None,
        None,
    )
    .await?;

    // Build a minimal Task struct for context writing
    use db::models::task::{PhaseState, Task, TaskStatus};

    let card = Task {
        id: task_id,
        project_id: {
            let b: Vec<u8> = row.try_get("project_id").unwrap_or_default();
            Uuid::from_slice(&b).unwrap_or_else(|_| Uuid::nil())
        },
        title: title.clone(),
        description: row.try_get("description").ok().flatten(),
        status: TaskStatus::Todo,
        parent_workspace_id: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        jira_key: jira_key.clone(),
        jira_snapshot: row.try_get("jira_snapshot").ok().flatten(),
        jira_synced_at: None,
        kanban_phase: kanban_phase.clone(),
        phase_state: PhaseState::Running,
        current_turn: row.try_get::<i64, _>("current_turn").unwrap_or(0),
        last_executor_session_id: None,
        review_pending_since: None,
        error_info: None,
        pending_inject: None,
    };

    let max_turns = column
        .max_turns_per_phase
        .or(ctx.workflow.defaults.max_turns_per_phase)
        .unwrap_or(5);

    let cancel = Arc::new(tokio::sync::Notify::new());
    let inputs = crate::dispatcher::phase::PhaseInputs {
        card: &card,
        column,
        worktree: &worktree_dir,
        agent_md: &agent_md,
        max_turns,
        turn_timeout: std::time::Duration::from_secs(60 * 30),
        reconcile_blob: pending_inject,
        review_feedback: None,
        cancel,
    };
    let outcome =
        crate::dispatcher::phase::run_phase(inputs, ctx.executor.as_ref()).await?;

    use crate::dispatcher::phase::PhaseOutcome;
    match outcome {
        PhaseOutcome::Complete {
            turns_used,
            last_session,
        } => {
            advance_card(ctx, task_id, &kanban_phase, column, turns_used, last_session).await?;
        }
        PhaseOutcome::AwaitingReview {
            turns_used,
            last_session,
        } => {
            let id_bytes = task_id.as_bytes().to_vec();
            let session_bytes = last_session.as_bytes().to_vec();
            sqlx::query(
                "UPDATE tasks SET phase_state='awaiting_review', current_turn=?, \
                 review_pending_since=?, last_executor_session_id=? WHERE id=?",
            )
            .bind(turns_used as i64)
            .bind(Utc::now().to_rfc3339())
            .bind(session_bytes)
            .bind(id_bytes)
            .execute(&ctx.pool)
            .await?;
            let msg = format!(
                "{}: {}",
                jira_key.as_deref().unwrap_or(""),
                title
            );
            ctx.notifier.notify("awaiting_review", &msg).await;
            crate::events::emit(
                &ctx.pool,
                task_id,
                "awaiting_review",
                "dispatcher",
                kanban_phase.as_deref(),
                None,
                None,
            )
            .await?;
        }
        PhaseOutcome::Failed { reason, turns_used } => {
            let info = serde_json::json!({"reason": reason, "turns": turns_used});
            let id_bytes = task_id.as_bytes().to_vec();
            sqlx::query(
                "UPDATE tasks SET phase_state='error', error_info=?, current_turn=? WHERE id=?",
            )
            .bind(info.to_string())
            .bind(turns_used as i64)
            .bind(id_bytes)
            .execute(&ctx.pool)
            .await?;
            let msg = format!(
                "{}: {}",
                jira_key.as_deref().unwrap_or(""),
                title
            );
            ctx.notifier.notify("error", &msg).await;
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
        }
        PhaseOutcome::Cancelled => {
            let id_bytes = task_id.as_bytes().to_vec();
            sqlx::query("UPDATE tasks SET phase_state='archived' WHERE id=?")
                .bind(id_bytes)
                .execute(&ctx.pool)
                .await?;
        }
    }
    Ok(())
}

async fn advance_card(
    ctx: &OrchestratorContext,
    task_id: Uuid,
    kanban_phase: &Option<String>,
    column: &crate::config::Column,
    _turns_used: u32,
    last_session: Uuid,
) -> crate::Result<()> {
    let next = column.next.as_deref().ok_or_else(|| {
        crate::OrchestratorError::Workflow("non-terminal column missing next".into())
    })?;

    // Try Jira transition if configured
    if let Some(t) = column.jira_transition.as_deref() {
        use sqlx::Row;
        let id_bytes = task_id.as_bytes().to_vec();
        if let Ok(Some(row)) =
            sqlx::query("SELECT jira_key FROM tasks WHERE id=?")
                .bind(id_bytes)
                .fetch_optional(&ctx.pool)
                .await
        {
            if let Ok(Some(key)) = row.try_get::<Option<String>, _>("jira_key") {
                if let Err(e) = ctx.jira.transition(&key, t).await {
                    let info = serde_json::json!({
                        "jira_transition_failed": e.to_string(),
                        "transition": t
                    });
                    let id_bytes2 = task_id.as_bytes().to_vec();
                    let _ = sqlx::query("UPDATE tasks SET error_info=? WHERE id=?")
                        .bind(info.to_string())
                        .bind(id_bytes2)
                        .execute(&ctx.pool)
                        .await;
                }
            }
        }
    }

    let id_bytes = task_id.as_bytes().to_vec();
    let session_bytes = last_session.as_bytes().to_vec();
    sqlx::query(
        "UPDATE tasks SET kanban_phase=?, phase_state='idle', current_turn=0, \
         last_executor_session_id=? WHERE id=?",
    )
    .bind(next)
    .bind(session_bytes)
    .bind(id_bytes)
    .execute(&ctx.pool)
    .await?;

    crate::events::emit(
        &ctx.pool,
        task_id,
        "phase_advanced",
        "dispatcher",
        kanban_phase.as_deref(),
        Some(next),
        None,
    )
    .await?;
    Ok(())
}
