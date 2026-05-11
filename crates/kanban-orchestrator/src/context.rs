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
        None => {
            let _ = std::fs::remove_file(&recpath);
        }
    }
    let revpath = dir.join("review_feedback.md");
    match review_feedback {
        Some(s) => std::fs::write(&revpath, s)?,
        None => {
            let _ = std::fs::remove_file(&revpath);
        }
    }
    Ok(())
}

pub fn append_history(worktree: &Path, entry: &serde_json::Value) -> crate::Result<()> {
    let path = worktree.join(".kanban-context/history.jsonl");
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(f, "{}", serde_json::to_string(entry).unwrap())?;
    Ok(())
}
