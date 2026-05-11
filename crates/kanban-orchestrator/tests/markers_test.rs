use kanban_orchestrator::dispatcher::markers::{parse_markers, MarkerOutcome};

#[test]
fn detects_complete_marker() {
    let s = "doing work\n<<KANBAN_PHASE_COMPLETE>>\nbye";
    assert_eq!(parse_markers(s), MarkerOutcome::Complete);
}

#[test]
fn detects_failure_with_reason() {
    let s = "<<KANBAN_PHASE_FAILED reason=\"build broke\">>";
    match parse_markers(s) {
        MarkerOutcome::Failed(r) => assert_eq!(r, "build broke"),
        _ => panic!("expected Failed"),
    }
}

#[test]
fn no_marker_means_continue() {
    assert_eq!(parse_markers("nothing special"), MarkerOutcome::Continue);
}
