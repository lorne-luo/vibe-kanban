#[derive(Debug, PartialEq, Eq)]
pub enum MarkerOutcome {
    Complete,
    Failed(String),
    Continue,
}

pub fn parse_markers(stdout: &str) -> MarkerOutcome {
    if let Some(idx) = stdout.find("<<KANBAN_PHASE_FAILED") {
        let tail = &stdout[idx..];
        let reason = tail
            .find("reason=\"")
            .and_then(|a| {
                let start = a + "reason=\"".len();
                tail[start..]
                    .find('"')
                    .map(|e| tail[start..start + e].to_string())
            })
            .unwrap_or_default();
        return MarkerOutcome::Failed(reason);
    }
    if stdout.contains("<<KANBAN_PHASE_COMPLETE>>") {
        return MarkerOutcome::Complete;
    }
    MarkerOutcome::Continue
}
