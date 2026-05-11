/// Stub notifier — G1 will replace this with a real implementation.
#[derive(Clone)]
pub struct Notifier;

impl Notifier {
    pub fn new() -> Self {
        Self
    }

    pub async fn notify(&self, _title: &str, _message: &str) {
        // no-op until G1
    }
}

impl Default for Notifier {
    fn default() -> Self {
        Self::new()
    }
}
