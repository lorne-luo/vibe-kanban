use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;

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
        // Fire immediately on startup
        (self.tick)().await;

        let mut ticker = tokio::time::interval(self.interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // Consume the first tick (already fired above)
        ticker.tick().await;

        loop {
            tokio::select! {
                _ = ticker.tick() => { (self.tick)().await; }
                _ = self.trigger.0.notified() => { (self.tick)().await; }
            }
        }
    }
}
