#[tokio::test]
async fn hook_failure_is_propagated() {
    // run_hook with a failing command should return Err
    let dir = tempfile::tempdir().unwrap();
    let result: kanban_orchestrator::Result<()> =
        kanban_orchestrator::scheduler::tick::run_hook_for_test("exit 1", dir.path()).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn hook_success_returns_ok() {
    let dir = tempfile::tempdir().unwrap();
    let result: kanban_orchestrator::Result<()> =
        kanban_orchestrator::scheduler::tick::run_hook_for_test("true", dir.path()).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn empty_hook_is_noop() {
    let dir = tempfile::tempdir().unwrap();
    let result: kanban_orchestrator::Result<()> =
        kanban_orchestrator::scheduler::tick::run_hook_for_test("", dir.path()).await;
    assert!(result.is_ok());
}
