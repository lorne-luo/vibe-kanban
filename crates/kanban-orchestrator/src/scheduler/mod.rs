use std::{future::Future, pin::Pin, sync::Arc, time::Duration};

use db::DBService;
use tokio::sync::Notify;

pub mod tick;

#[derive(Clone)]
pub struct ManualTrigger(Arc<Notify>);

impl ManualTrigger {
    pub fn new() -> Self {
        Self(Arc::new(Notify::new()))
    }

    pub fn fire(&self) {
        self.0.notify_one();
    }
}

impl Default for ManualTrigger {
    fn default() -> Self {
        Self::new()
    }
}

type TickFn = Box<dyn Fn() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

pub struct Scheduler {
    interval: Duration,
    trigger: ManualTrigger,
    tick: TickFn,
}

impl Scheduler {
    pub fn new<F, Fut>(interval: Duration, trigger: ManualTrigger, f: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let tick: TickFn = Box::new(move || Box::pin(f()));
        Self {
            interval,
            trigger,
            tick,
        }
    }

    pub async fn run(self) {
        let mut ticker = tokio::time::interval(self.interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = ticker.tick() => { (self.tick)().await; }
                _ = self.trigger.0.notified() => { (self.tick)().await; }
            }
        }
    }

    /// Build a scheduler that discovers and ticks all projects in the DB.
    ///
    /// Uses the workflow's `poll_interval` from the first found workflow, or 5 minutes
    /// as a default if no workflows are yet discovered.
    pub fn new_from_db(db: DBService, trigger: ManualTrigger) -> Self {
        let interval = Duration::from_secs(5 * 60);
        Self::new(interval, trigger, move || {
            let db = db.clone();
            async move {
                if let Err(e) = tick::do_all_projects_tick(&db).await {
                    tracing::warn!(?e, "kanban tick error");
                }
            }
        })
    }
}
