use std::{path::Path, sync::Arc, time::Duration};

use db::models::task::Task;
use tokio::sync::Notify;

use crate::{
    config::{Column, OnComplete},
    context::{append_history, write_context},
    dispatcher::{
        PhaseExecutor, TurnOutcome,
        markers::{MarkerOutcome, parse_markers},
    },
};

#[derive(Debug)]
pub enum PhaseOutcome {
    Complete {
        turns_used: u32,
        last_session: uuid::Uuid,
    },
    AwaitingReview {
        turns_used: u32,
        last_session: uuid::Uuid,
    },
    Failed {
        reason: String,
        turns_used: u32,
    },
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
                append_history(
                    inputs.worktree,
                    &serde_json::json!({
                        "phase": inputs.column.name,
                        "turns": turn,
                        "session": out.session_id
                    }),
                )?;
                return Ok(PhaseOutcome::Complete {
                    turns_used: turn,
                    last_session,
                });
            }
            MarkerOutcome::Failed(reason) => {
                return Ok(PhaseOutcome::Failed {
                    reason,
                    turns_used: turn,
                });
            }
            MarkerOutcome::Continue => continue,
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
    use std::{
        sync::Arc as StdArc,
        task::{Context, Poll, Waker},
    };

    struct NoopWaker;
    impl std::task::Wake for NoopWaker {
        fn wake(self: StdArc<Self>) {}
    }
    let waker = Waker::from(StdArc::new(NoopWaker));
    let mut cx = Context::from_waker(&waker);
    let mut fut = std::pin::pin!(n.notified());
    matches!(std::pin::Pin::new(&mut fut).poll(&mut cx), Poll::Ready(_))
}
