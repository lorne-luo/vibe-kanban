use std::sync::OnceLock;

use db::DBService;
use uuid::Uuid;

use crate::scheduler::ManualTrigger;

static KANBAN_TRIGGER: OnceLock<ManualTrigger> = OnceLock::new();

/// Handle returned by [`start`] that allows callers to trigger immediate polls.
#[derive(Clone)]
pub struct RuntimeHandle {
    trigger: ManualTrigger,
}

impl RuntimeHandle {
    pub fn trigger(&self) -> ManualTrigger {
        self.trigger.clone()
    }
}

/// Start the kanban orchestrator: run startup recovery, create the scheduler,
/// and fire an immediate first poll.
///
/// This function is intentionally non-fatal from the caller's perspective:
/// `main.rs` wraps the call in `if let Err(err) = start(...).await { warn! }`.
pub async fn start(db: DBService) -> crate::Result<RuntimeHandle> {
    crate::recovery::reset_running(&db.pool).await?;

    let trigger = ManualTrigger::new();
    let _ = KANBAN_TRIGGER.set(trigger.clone());

    let scheduler = crate::scheduler::Scheduler::new_from_db(db, trigger.clone());
    tokio::spawn(async move {
        scheduler.run().await;
    });

    trigger.fire();
    Ok(RuntimeHandle { trigger })
}

/// Fire the global trigger to schedule an immediate poll (no-op if not started).
pub async fn trigger_poll_now(_db: &DBService) -> crate::Result<()> {
    if let Some(trigger) = KANBAN_TRIGGER.get() {
        trigger.fire();
    }
    Ok(())
}

/// Re-sync a specific task from Jira on the next poll cycle.
pub async fn sync_task_now(db: &DBService, task_id: Uuid) -> crate::Result<()> {
    crate::scheduler::tick::sync_task_now(db, task_id).await
}

/// Run a single Jira poll cycle for the given project key.
/// Used by the `kanban_poll_once` binary.
pub async fn poll_once(db: &DBService, project: String) -> crate::Result<()> {
    tracing::info!(project, "running one kanban poll tick");
    // For poll_once we scan the DB for the project's working dir and load the workflow.
    use sqlx::Row;
    let rows = sqlx::query(
        "SELECT id, default_agent_working_dir FROM projects \
         WHERE default_agent_working_dir IS NOT NULL",
    )
    .fetch_all(&db.pool)
    .await?;

    for row in rows {
        let id_bytes: Vec<u8> = match row.try_get("id") {
            Ok(b) => b,
            Err(_) => continue,
        };
        let project_id = match Uuid::from_slice(&id_bytes) {
            Ok(id) => id,
            Err(_) => continue,
        };
        let working_dir: String = match row.try_get("default_agent_working_dir") {
            Ok(d) => d,
            Err(_) => continue,
        };
        let repo_root = std::path::PathBuf::from(&working_dir);
        let workflow = match crate::config::load_workflow(&repo_root, &project) {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!(?e, project, "workflow not found, skipping");
                continue;
            }
        };
        crate::scheduler::tick::do_tick_for_project(db, project_id, &repo_root, workflow).await?;
    }
    Ok(())
}

pub fn set_trigger(trigger: ManualTrigger) {
    let _ = KANBAN_TRIGGER.set(trigger);
}
