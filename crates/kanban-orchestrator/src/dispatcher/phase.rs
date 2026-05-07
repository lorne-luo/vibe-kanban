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

        // Write per-turn log
        if let Some(jira_key) = inputs.card.jira_key.as_deref() {
            let log_dir = inputs.worktree.join(format!("logs/{}", jira_key));
            let _ = std::fs::create_dir_all(&log_dir);
            let log_path = log_dir.join(format!(
                "phase-{}-turn-{}.log",
                inputs.column.name.replace(' ', "_"),
                turn
            ));
            let content = format!(
                "=== STDOUT ===\n{}\n=== STDERR ===\n{}\n",
                out.stdout, out.stderr
            );
            let _ = std::fs::write(log_path, content);
        }

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
    use std::{
        future::Future,
        pin::Pin,
        task::{Context, Poll},
    };

    let waker = futures::task::noop_waker();
    let mut cx = Context::from_waker(&waker);
    let notified = n.notified();
    let mut fut = Box::pin(notified);
    matches!(Pin::as_mut(&mut fut).poll(&mut cx), Poll::Ready(_))
}
