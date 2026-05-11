use std::sync::{Arc, Mutex};

use services::services::notification::NotificationService;

#[derive(Clone)]
pub enum Notifier {
    Service(NotificationService),
    Capture(Arc<Mutex<Vec<(String, String)>>>),
    Disabled,
}

impl Notifier {
    pub fn service(service: NotificationService) -> Self {
        Self::Service(service)
    }

    pub fn disabled() -> Self {
        Self::Disabled
    }

    /// Alias for `disabled()` — convenience constructor for use in tests and
    /// contexts where a real `NotificationService` is not available.
    pub fn new() -> Self {
        Self::Disabled
    }

    pub fn capture() -> (Self, Arc<Mutex<Vec<(String, String)>>>) {
        let log = Arc::new(Mutex::new(Vec::new()));
        (Self::Capture(log.clone()), log)
    }

    pub async fn notify(&self, title: &str, message: &str) {
        match self {
            Self::Service(service) => service.notify(title, message).await,
            Self::Capture(log) => {
                log.lock()
                    .unwrap()
                    .push((title.to_string(), message.to_string()));
            }
            Self::Disabled => {}
        }
    }

    pub async fn card_created(&self, jira_key: &str, summary: &str) {
        self.notify("Kanban card created", &format!("{jira_key}: {summary}"))
            .await;
    }

    pub async fn status_changed(&self, jira_key: &str, phase: &str) {
        self.notify("Kanban status changed", &format!("{jira_key}: {phase}"))
            .await;
    }

    pub async fn awaiting_review(&self, jira_key: &str) {
        self.notify("Kanban awaiting review", jira_key).await;
    }

    pub async fn error(&self, jira_key: &str, message: &str) {
        self.notify("Kanban error", &format!("{jira_key}: {message}"))
            .await;
    }
}

impl Default for Notifier {
    fn default() -> Self {
        Self::Disabled
    }
}
