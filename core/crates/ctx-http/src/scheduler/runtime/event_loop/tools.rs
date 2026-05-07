use ctx_core::models::{SessionEvent, SessionEventType};
use ctx_session_tools::order_seq::read_order_seq;
use ctx_session_tools::{
    build_tool_ops_meta_from_normalized, build_turn_tool_update, merge_tool_update,
    sanitize_normalized_tool_event_payload, tool_count_deltas, NormalizedToolEvent,
};
use ctx_store::store::SessionTurnToolCountDeltas;
use serde_json::{json, Value};

use crate::daemon::AppState;
use crate::ops_events::OpsEvent;

use super::super::tool_runtime::{self, cwd_outside_worktree, maybe_spool_tool_output};
use super::state::EventLoopRuntimeState;
use super::TurnEventLoop;

pub(super) async fn prepare_tool_event_payload(
    ctx: &TurnEventLoop,
    state: &AppState,
    event_type: &SessionEventType,
    tool_event: &NormalizedToolEvent,
) -> Value {
    if matches!(event_type, SessionEventType::ToolCall) {
        emit_tool_call_ops(ctx, state, tool_event);
    }

    let output_artifact = if matches!(event_type, SessionEventType::ToolResult) {
        maybe_spool_tool_output(
            state,
            &ctx.store,
            tool_event,
            tool_runtime::ToolOutputArtifactScope {
                session_id: ctx.session_id,
                task_id: ctx.task_id,
                workspace_id: ctx.workspace_id,
                worktree_id: ctx.worktree_id,
                turn_id: ctx.turn_id,
            },
        )
        .await
    } else {
        None
    };

    sanitize_normalized_tool_event_payload(tool_event, output_artifact.as_ref())
}

fn emit_tool_call_ops(ctx: &TurnEventLoop, state: &AppState, tool_event: &NormalizedToolEvent) {
    let tool_meta = build_tool_ops_meta_from_normalized(tool_event);
    let mut meta = serde_json::Map::new();
    if let Some(tool_call_id) = tool_meta.tool_call_id.clone() {
        meta.insert("tool_call_id".to_string(), json!(tool_call_id));
    }
    if let Some(title) = tool_meta.title.clone() {
        meta.insert("title".to_string(), json!(title));
    }
    if let Some(status) = tool_meta.status.clone() {
        meta.insert("status".to_string(), json!(status));
    }
    if let Some(input_preview) = tool_meta.input_preview.clone() {
        meta.insert("input".to_string(), input_preview);
    }

    let mut event = OpsEvent::new("info", "tool_exec");
    event.session_id = Some(ctx.session_id.0.to_string());
    event.worktree_id = Some(ctx.worktree_id.0.to_string());
    event.run_id = Some(ctx.run_id.0.to_string());
    event.turn_id = Some(ctx.turn_id.0.to_string());
    event.provider_id = Some(ctx.provider_id.clone());
    event.tool_kind = tool_meta.tool_kind.clone();
    event.cwd = tool_meta.cwd.clone();
    event.worktree_root = Some(ctx.workdir_str.clone());
    event.meta = if meta.is_empty() {
        None
    } else {
        Some(Value::Object(meta))
    };
    state.telemetry.ops_events.emit(event);

    if let Some(cwd) = tool_meta.cwd.as_deref() {
        if cwd_outside_worktree(cwd, &ctx.workdir_root, ctx.workdir_canonical.as_ref()) {
            let mut warn_event = OpsEvent::new("warn", "tool_exec_anomaly");
            warn_event.session_id = Some(ctx.session_id.0.to_string());
            warn_event.worktree_id = Some(ctx.worktree_id.0.to_string());
            warn_event.run_id = Some(ctx.run_id.0.to_string());
            warn_event.turn_id = Some(ctx.turn_id.0.to_string());
            warn_event.provider_id = Some(ctx.provider_id.clone());
            warn_event.tool_kind = tool_meta.tool_kind.clone();
            warn_event.cwd = Some(cwd.to_string());
            warn_event.worktree_root = Some(ctx.workdir_str.clone());
            warn_event.meta = Some(json!({
                "reason": "cwd_outside_worktree",
                "tool_call_id": tool_meta.tool_call_id,
            }));
            state.telemetry.ops_events.emit(warn_event);
        }
    }
}

pub(super) async fn handle_persisted_tool_event(
    ctx: &TurnEventLoop,
    runtime: &mut EventLoopRuntimeState,
    event: &SessionEvent,
    tool_event: &NormalizedToolEvent,
) {
    let order_seq = read_order_seq(&event.payload_json);
    let Some(update) = build_turn_tool_update(tool_event, order_seq) else {
        return;
    };

    let prev = if matches!(&event.event_type, SessionEventType::ToolCallUpdate) {
        runtime.tool_cache.get(&update.tool_call_id).cloned()
    } else if let Some(cached) = runtime.tool_cache.get(&update.tool_call_id).cloned() {
        Some(cached)
    } else {
        ctx.store
            .get_session_turn_tool(ctx.session_id, &update.tool_call_id)
            .await
            .ok()
            .flatten()
    };

    let Some(merged) = merge_tool_update(
        prev.as_ref(),
        update,
        ctx.session_id,
        ctx.turn_id,
        event.seq,
        event.created_at,
    ) else {
        return;
    };

    if matches!(&event.event_type, SessionEventType::ToolCallUpdate) {
        runtime
            .tool_cache
            .insert(merged.tool_call_id.clone(), merged);
        return;
    }

    let (delta_total, delta_pending, delta_running, delta_completed, delta_failed) =
        tool_count_deltas(prev.as_ref(), &merged);
    let _ = ctx.store.upsert_session_turn_tool(merged.clone()).await;
    if delta_total != 0
        || delta_pending != 0
        || delta_running != 0
        || delta_completed != 0
        || delta_failed != 0
    {
        let _ = ctx
            .store
            .update_session_turn_tool_counts(
                ctx.session_id,
                ctx.turn_id,
                SessionTurnToolCountDeltas {
                    total: delta_total,
                    pending: delta_pending,
                    running: delta_running,
                    completed: delta_completed,
                    failed: delta_failed,
                },
                event.created_at,
            )
            .await;
    }
    runtime
        .tool_cache
        .insert(merged.tool_call_id.clone(), merged);
}
