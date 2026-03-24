use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceActiveSnapshotStats {
    pub workspace_count: usize,
    pub active_task_count: usize,
    pub active_head_count: usize,
    pub session_replay_sessions: usize,
    pub session_replay_events: usize,
    pub session_replay_event_bytes: usize,
    pub session_replay_event_max_bytes: usize,
    pub workspace_stream_buffer_total: usize,
    pub workspace_stream_buffer_max: usize,
    pub workspace_stream_receivers_total: usize,
    pub workspace_stream_receivers_max: usize,
    pub session_heads_count: usize,
    pub session_heads_bytes: usize,
    pub session_heads_max_bytes: usize,
    pub active_head_index_count: usize,
    pub active_head_bytes: usize,
    pub active_head_max_bytes: usize,
}
