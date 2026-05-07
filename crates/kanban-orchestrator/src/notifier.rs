//! Notifier — sends macOS / desktop notifications for kanban events.
//!
//! The full osascript implementation lives in Task G1.  For now this is a
//! lightweight stub that logs at INFO level so the rest of the codebase
//! can call `ctx.notifier.notify(…)` without any conditional compilation.

pub struct Notifier {
    enabled: bool,
}

impl Notifier {
    pub fn new(enabled: bool) -> Self {
        Self { enabled }
    }

    /// Fire a notification.  `event` is a short identifier (e.g. "error"),
    /// `key` is the Jira issue key (may be empty), and `title` is the card
    /// title shown in the alert.
    pub fn notify(&self, event: &str, key: &str, title: &str) {
        if self.enabled {
            tracing::info!(event, key, title, "kanban notification");
        }
    }
}

impl Default for Notifier {
    fn default() -> Self {
        Self::new(false)
    }
}
