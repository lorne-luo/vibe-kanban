use kanban_orchestrator::notifier::Notifier;

#[tokio::test]
async fn captures_notify_calls_without_spawning_processes() {
    let (notifier, log) = Notifier::capture();
    notifier.status_changed("AP-1", "Coding").await;

    let log = log.lock().unwrap();
    assert_eq!(log[0].0, "Kanban status changed");
    assert_eq!(log[0].1, "AP-1: Coding");
}
