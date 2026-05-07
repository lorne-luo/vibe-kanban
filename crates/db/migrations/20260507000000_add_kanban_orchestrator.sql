-- Extend tasks for Kanban orchestrator
ALTER TABLE tasks ADD COLUMN jira_key TEXT;
ALTER TABLE tasks ADD COLUMN jira_snapshot TEXT;             -- JSON
ALTER TABLE tasks ADD COLUMN jira_synced_at TEXT;            -- RFC3339 datetime
ALTER TABLE tasks ADD COLUMN kanban_phase TEXT;
ALTER TABLE tasks ADD COLUMN phase_state TEXT NOT NULL DEFAULT 'idle';
ALTER TABLE tasks ADD COLUMN current_turn INTEGER NOT NULL DEFAULT 0;
ALTER TABLE tasks ADD COLUMN last_executor_session_id TEXT;  -- UUID text
ALTER TABLE tasks ADD COLUMN review_pending_since TEXT;
ALTER TABLE tasks ADD COLUMN error_info TEXT;                -- JSON
ALTER TABLE tasks ADD COLUMN pending_inject TEXT;            -- JSON

CREATE UNIQUE INDEX idx_tasks_jira_key ON tasks(jira_key) WHERE jira_key IS NOT NULL;
CREATE INDEX idx_tasks_phase_state ON tasks(phase_state);

CREATE TABLE task_events (
    id           TEXT PRIMARY KEY,
    task_id      TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    event_type   TEXT NOT NULL,
    from_phase   TEXT,
    to_phase     TEXT,
    actor        TEXT NOT NULL,
    payload      TEXT,                                       -- JSON
    ts           TEXT NOT NULL
);

CREATE INDEX idx_task_events_task_id_ts ON task_events(task_id, ts DESC);
