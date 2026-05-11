use kanban_orchestrator::dispatcher::exec_adapter::RealExecutor;

#[test]
fn real_executor_can_be_constructed() {
    // Just verify the type exists and the module compiles.
    // Full integration test requires a live executor (e.g. qa-mode feature).
    let _ = std::any::type_name::<RealExecutor>();
}
