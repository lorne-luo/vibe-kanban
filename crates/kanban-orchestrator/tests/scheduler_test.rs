use kanban_orchestrator::scheduler::{ManualTrigger, Scheduler};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[tokio::test]
async fn manual_trigger_fires_tick() {
    let counter = Arc::new(AtomicUsize::new(0));
    let c2 = counter.clone();
    let trigger = ManualTrigger::new();
    let scheduler = Scheduler::new(
        Duration::from_secs(3600), // very long interval
        trigger.clone(),
        move || {
            let c = c2.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
            }
        },
    );
    let handle = tokio::spawn(scheduler.run());
    // Wait for startup tick
    tokio::time::sleep(Duration::from_millis(50)).await;
    let after_startup = counter.load(Ordering::SeqCst);
    // Fire manual trigger
    trigger.fire();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let after_trigger = counter.load(Ordering::SeqCst);
    handle.abort();
    assert!(after_startup >= 1, "startup tick should fire");
    assert!(after_trigger >= 2, "manual trigger should fire");
}
