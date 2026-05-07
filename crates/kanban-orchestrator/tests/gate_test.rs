use kanban_orchestrator::dispatcher::gate::Gate;

#[tokio::test]
async fn gate_caps_global_concurrency() {
    let g = Gate::new(2, 10); // max 2 global, 10 per column
    let p1 = g.try_acquire("Coding").await;
    assert!(p1.is_some());
    let p2 = g.try_acquire("Reviewing").await;
    assert!(p2.is_some());
    // Global cap at 2
    let p3 = g.try_acquire("Done").await;
    assert!(p3.is_none(), "global cap should reject 3rd");
    drop(p1);
    // After dropping one, should succeed
    let p4 = g.try_acquire("Done").await;
    assert!(p4.is_some());
}

#[tokio::test]
async fn gate_caps_per_column() {
    let g = Gate::new(10, 1); // max 10 global, 1 per column
    let p1 = g.try_acquire("Coding").await;
    assert!(p1.is_some());
    // Per-column cap
    let p2 = g.try_acquire("Coding").await;
    assert!(p2.is_none(), "per-column cap should reject 2nd for same column");
    // Different column is fine
    let p3 = g.try_acquire("Reviewing").await;
    assert!(p3.is_some());
    drop(p1);
    // After drop, column slot freed
    let p4 = g.try_acquire("Coding").await;
    assert!(p4.is_some());
}
