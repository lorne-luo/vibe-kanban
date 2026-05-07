use crate::config::{Column, OnComplete};
use crate::context::{append_history, write_context};
use crate::dispatcher::markers::{parse_markers, MarkerOutcome};
use crate::dispatcher::{PhaseExecutor, TurnOutcome};
use db::models::task::Task;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;

#[derive(Debug)]
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
    write_context(
        inputs.worktree,
        inputs.card,
        inputs.reconcile_blob.as_deref(),
        inputs.review_feedback.as_deref(),
    )?;

    let mut last_session = uuid::Uuid::nil();
    for turn in 1..=inputs.max_turns {
        if is_cancelled(&inputs.cancel) {
            return Ok(PhaseOutcome::Cancelled);
        }
        let out: TurnOutcome = exec
            .run_turn(inputs.worktree, inputs.agent_md, inputs.turn_timeout)
            .await?;
        last_session = out.session_id;
        match parse_markers(&out.stdout) {
            MarkerOutcome::Complete => {
                let _ = append_history(
                    inputs.worktree,
                    &serde_json::json!({
                        "phase": inputs.column.name,
                        "turns": turn,
                        "session": out.session_id.to_string(),
                    }),
                );
                return Ok(PhaseOutcome::Complete { turns_used: turn, last_session });
            }
            MarkerOutcome::Failed(reason) => {
                return Ok(PhaseOutcome::Failed { reason, turns_used: turn });
            }
            MarkerOutcome::Continue => {}
        }
    }

    if inputs.column.on_complete == OnComplete::Review {
        Ok(PhaseOutcome::AwaitingReview {
            turns_used: inputs.max_turns,
            last_session,
        })
    } else {
        Ok(PhaseOutcome::Complete {
            turns_used: inputs.max_turns,
            last_session,
        })
    }
}

fn is_cancelled(n: &Arc<Notify>) -> bool {
    use std::future::Future;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    let waker = futures::task::noop_waker();
    let mut cx = Context::from_waker(&waker);
    let notified = n.notified();
    let mut fut = Box::pin(notified);
    matches!(Pin::as_mut(&mut fut).poll(&mut cx), Poll::Ready(_))
}
