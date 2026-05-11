use kanban_orchestrator::dispatcher::gate::Gate;

#[tokio::test]
async fn gate_caps_concurrency() {
    let g = Gate::new(2, 1);
    let p1 = g.try_acquire("Coding").await.unwrap();
    let p2 = g.try_acquire("Reviewing").await.unwrap();
    assert!(
        g.try_acquire("Reviewing").await.is_none(),
        "per-column cap should reject"
    );
    drop(p2);
    let p3 = g.try_acquire("Reviewing").await.unwrap();
    drop(p1);
    drop(p3);
}
