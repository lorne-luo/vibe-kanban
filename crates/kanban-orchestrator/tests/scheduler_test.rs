use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[tokio::test(start_paused = true)]
async fn scheduler_polls_on_interval_and_on_trigger() {
    let counter = Arc::new(AtomicUsize::new(0));
    let c2 = counter.clone();
    let trigger = kanban_orchestrator::scheduler::ManualTrigger::new();
    let s = kanban_orchestrator::scheduler::Scheduler::new(
        std::time::Duration::from_secs(900),
        trigger.clone(),
        move || {
            c2.fetch_add(1, Ordering::SeqCst);
            async {}
        },
    );
    let h = tokio::spawn(s.run());
    // advance time to fire first tick
    tokio::time::advance(std::time::Duration::from_millis(10)).await;
    tokio::task::yield_now().await;
    // fire manual trigger
    trigger.fire();
    tokio::time::advance(std::time::Duration::from_millis(10)).await;
    tokio::task::yield_now().await;
    // advance past interval
    tokio::time::advance(std::time::Duration::from_secs(900)).await;
    tokio::task::yield_now().await;
    assert!(
        counter.load(Ordering::SeqCst) >= 3,
        "expected ≥3 ticks, got {}",
        counter.load(Ordering::SeqCst)
    );
    h.abort();
}
