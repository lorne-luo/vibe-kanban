use kanban_orchestrator::config::Workflow;

#[test]
fn loads_workflow_from_repo() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".agents/kanban-workflows")).unwrap();
    std::fs::create_dir_all(dir.path().join(".agents/agent")).unwrap();
    let yml = std::fs::read_to_string("tests/fixtures/valid_workflow.yml").unwrap();
    std::fs::write(dir.path().join(".agents/kanban-workflows/AP.yml"), yml).unwrap();
    for n in ["analyzer", "coder", "reviewer"] {
        std::fs::write(
            dir.path().join(format!(".agents/agent/{}.md", n)),
            "---\nname: x\n---\nbody",
        ).unwrap();
    }
    // SAFETY: single-threaded test, no concurrent env reads
    unsafe {
        std::env::set_var("JIRA_EMAIL", "x");
        std::env::set_var("JIRA_API_TOKEN", "y");
    }
    let w = kanban_orchestrator::config::load_workflow(dir.path(), "AP").unwrap();
    assert_eq!(w.project, "AP");
}

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

#[test]
fn rejects_missing_initial() {
    let s = std::fs::read_to_string("tests/fixtures/missing_initial.yml").unwrap();
    let w: Workflow = serde_yaml::from_str(&s).unwrap();
    let err = w.validate(&["analyzer", "coder", "reviewer"], &|_| true).unwrap_err();
    assert!(err.contains("initial"), "expected error about 'initial', got: {}", err);
}

#[test]
fn rejects_dangling_next() {
    let s = std::fs::read_to_string("tests/fixtures/dangling_next.yml").unwrap();
    let w: Workflow = serde_yaml::from_str(&s).unwrap();
    let err = w.validate(&["analyzer", "coder", "reviewer"], &|_| true).unwrap_err();
    assert!(err.contains("next"), "expected error about 'next', got: {}", err);
}

#[test]
fn rejects_missing_agent_file() {
    let s = std::fs::read_to_string("tests/fixtures/valid_workflow.yml").unwrap();
    let w: Workflow = serde_yaml::from_str(&s).unwrap();
    // only "analyzer" known — coder/reviewer missing
    let err = w.validate(&["analyzer"], &|_| true).unwrap_err();
    assert!(err.contains("agent"), "expected error about 'agent', got: {}", err);
}
