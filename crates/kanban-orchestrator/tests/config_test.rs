use kanban_orchestrator::config::Workflow;

#[test]
fn parses_valid_workflow() {
    let s = std::fs::read_to_string("tests/fixtures/valid_workflow.yml").unwrap();
    let w: Workflow = serde_yaml::from_str(&s).unwrap();
    assert_eq!(w.version, 1);
    assert_eq!(w.project, "AP");
    assert_eq!(w.columns.len(), 5);
    assert!(w.columns.iter().any(|c| c.initial.unwrap_or(false)));
    assert!(w.columns.iter().any(|c| c.terminal.unwrap_or(false)));
}
