#[test]
fn captures_notify_calls() {
    let (n, log) = kanban_orchestrator::notifier::Notifier::capture();
    n.notify("status_changed", "AP-1", "hello");
    let entries = log.lock().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].0, "status_changed");
    assert_eq!(entries[0].1, "AP-1");
}
