use std::{collections::HashMap, sync::Arc};

use tokio::sync::Mutex;

pub struct Gate {
    inner: Arc<Mutex<GateState>>,
    max_global: usize,
    max_per_column: usize,
}

struct GateState {
    global: usize,
    per_col: HashMap<String, usize>,
}

pub struct Permit {
    gate: Arc<Mutex<GateState>>,
    column: String,
}

impl Drop for Permit {
    fn drop(&mut self) {
        let g = self.gate.clone();
        let col = self.column.clone();
        if let Ok(mut s) = g.try_lock() {
            s.global = s.global.saturating_sub(1);
            if let Some(v) = s.per_col.get_mut(&col) {
                *v = v.saturating_sub(1);
            }
        } else {
            tokio::spawn(async move {
                let mut s = g.lock().await;
                s.global = s.global.saturating_sub(1);
                if let Some(v) = s.per_col.get_mut(&col) {
                    *v = v.saturating_sub(1);
                }
            });
        }
    }
}

impl Gate {
    pub fn new(max_global: usize, max_per_column: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(GateState {
                global: 0,
                per_col: HashMap::new(),
            })),
            max_global,
            max_per_column,
        }
    }

    pub async fn try_acquire(&self, column: &str) -> Option<Permit> {
        let mut s = self.inner.lock().await;
        if s.global >= self.max_global {
            return None;
        }
        let cur = *s.per_col.get(column).unwrap_or(&0);
        if cur >= self.max_per_column {
            return None;
        }
        s.global += 1;
        s.per_col.insert(column.to_string(), cur + 1);
        Some(Permit {
            gate: self.inner.clone(),
            column: column.to_string(),
        })
    }
}
