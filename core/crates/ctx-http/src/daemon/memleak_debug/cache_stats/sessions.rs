use ctx_resource_utilization::memleak_debug::json_bytes;

use crate::daemon::AppState;

use super::SessionCacheStats;

pub(super) async fn collect_session_cache_stats(state: &AppState) -> SessionCacheStats {
    let head_cache = state.sessions.session_head_cache.lock().await;
    let head_cache_entries = head_cache.len();
    let head_cache_keys = head_cache.values().map(|entry| entry.value.len()).sum();
    let mut session_head_cache_bytes = 0;
    let mut session_head_cache_max_bytes = 0;
    for entry in head_cache.values() {
        for head in entry.value.values() {
            let bytes = json_bytes(head);
            session_head_cache_bytes += bytes;
            if bytes > session_head_cache_max_bytes {
                session_head_cache_max_bytes = bytes;
            }
        }
    }
    drop(head_cache);

    let session_meta_cache = state.sessions.session_meta_cache.lock().await;
    let session_meta_cache_entries = session_meta_cache.len();
    let mut session_meta_cache_bytes = 0;
    for entry in session_meta_cache.values() {
        session_meta_cache_bytes += json_bytes(&entry.value);
    }
    drop(session_meta_cache);

    let session_event_heads = state.sessions.session_event_heads.lock().await.len();
    let schedulers = state.sessions.schedulers.lock().await.len();
    let broadcasters_guard = state.sessions.broadcasters.lock().await;
    let broadcasters = broadcasters_guard.len();
    let mut broadcast_buffer_total = 0;
    let mut broadcast_buffer_max = 0;
    let mut broadcast_receivers_total = 0;
    let mut broadcast_receivers_max = 0;
    for entry in broadcasters_guard.values() {
        let sender = &entry.value;
        let len = sender.len();
        broadcast_buffer_total += len;
        if len > broadcast_buffer_max {
            broadcast_buffer_max = len;
        }
        let receivers = sender.receiver_count();
        broadcast_receivers_total += receivers;
        if receivers > broadcast_receivers_max {
            broadcast_receivers_max = receivers;
        }
    }
    drop(broadcasters_guard);
    let running_sessions = state.sessions.running_sessions.lock().await.len();
    let active_task_refreshes = state.sessions.active_task_refreshes.lock().await.len();

    SessionCacheStats {
        session_head_cache_entries: head_cache_entries,
        session_head_cache_keys: head_cache_keys,
        session_head_cache_bytes,
        session_head_cache_max_bytes,
        session_meta_cache_entries,
        session_meta_cache_bytes,
        session_event_heads,
        schedulers,
        broadcasters,
        broadcast_buffer_total,
        broadcast_buffer_max,
        broadcast_receivers_total,
        broadcast_receivers_max,
        running_sessions,
        active_task_refreshes,
    }
}
