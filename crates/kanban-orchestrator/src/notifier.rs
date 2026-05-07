//! Notifier — sends macOS / desktop notifications for kanban events.

pub struct Notifier {
    enabled: bool,
    spawner: Box<dyn Fn(&str, &str, &str) + Send + Sync>,
}

impl Notifier {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            spawner: Box::new(|title, key, summary| {
                if !cfg!(target_os = "macos") && std::env::var("KANBAN_FORCE_NOTIFY").is_err() {
                    return;
                }
                let body = format!("{} - {}", key, summary);
                let _ = std::process::Command::new("osascript")
                    .args([
                        "-e",
                        &format!(
                            r#"display notification "{}" with title "vibe-kanban" subtitle "{}""#,
                            escape_osascript(&body),
                            escape_osascript(title),
                        ),
                    ])
                    .spawn();
            }),
        }
    }

    #[cfg(any(test, feature = "test-utils"))]
    pub fn capture() -> (
        Self,
        std::sync::Arc<std::sync::Mutex<Vec<(String, String, String)>>>,
    ) {
        let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let l2 = log.clone();
        (
            Self {
                enabled: true,
                spawner: Box::new(move |t, k, s| {
                    l2.lock().unwrap().push((t.into(), k.into(), s.into()));
                }),
            },
            log,
        )
    }

    pub fn notify(&self, title: &str, key: &str, summary: &str) {
        if !self.enabled {
            return;
        }
        (self.spawner)(title, key, summary);
    }
}

impl Default for Notifier {
    fn default() -> Self {
        Self::new(false)
    }
}

fn escape_osascript(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}
