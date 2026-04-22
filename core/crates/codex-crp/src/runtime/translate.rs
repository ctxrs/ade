use super::{AppServerSessionState, CrpEventRouter};
use crate::app_server::{
    AgentMessageDeltaNotification, CommandExecutionOutputDeltaNotification,
    ItemLifecycleNotification, ReasoningSummaryPartAddedNotification,
    ReasoningSummaryTextDeltaNotification, ReasoningTextDeltaNotification, ThreadItem,
    ThreadTokenUsageUpdatedNotification, TurnCompletedNotification, TurnStartedNotification,
};
use crate::protocol::{CrpChannel, CrpEvent, CrpToolStatus, CrpTurnError, CrpTurnStatus};
use anyhow::Result;
use serde_json::{json, Value};
use tracing::warn;

use super::io::dispatch_event;

pub(super) fn translate_notification(
    session_state: &mut AppServerSessionState,
    method: &str,
    params: Value,
) -> Result<Vec<(CrpChannel, CrpEvent)>> {
    match method {
        "thread/tokenUsage/updated" => {
            let payload: ThreadTokenUsageUpdatedNotification = serde_json::from_value(params)?;
            if payload.thread_id != session_state.thread_id {
                return Ok(Vec::new());
            }
            let turn_id = session_state
                .turn_aliases
                .ensure_crp_turn_id(payload.turn_id.as_str());
            let context_window = canonical_context_window_from_thread_usage(&payload.token_usage);
            session_state
                .turn_aliases
                .note_token_usage(payload.turn_id.as_str(), payload.token_usage);
            if let Some(context_window) = context_window {
                Ok(vec![(
                    CrpChannel::Control,
                    CrpEvent::TurnContextWindowUpdated {
                        session_id: session_state.tracker.session_id.clone(),
                        turn_id,
                        context_window,
                    },
                )])
            } else {
                Ok(Vec::new())
            }
        }
        "turn/started" => {
            let payload: TurnStartedNotification = serde_json::from_value(params)?;
            if payload.thread_id != session_state.thread_id {
                return Ok(Vec::new());
            }
            let turn_id = session_state
                .turn_aliases
                .ensure_crp_turn_id(payload.turn.id.as_str());
            session_state.tracker.ensure_turn(payload.turn.id.as_str());
            Ok(vec![(
                CrpChannel::Control,
                CrpEvent::TurnStarted {
                    session_id: session_state.tracker.session_id.clone(),
                    turn_id,
                },
            )])
        }
        "turn/completed" => {
            let payload: TurnCompletedNotification = serde_json::from_value(params)?;
            if payload.thread_id != session_state.thread_id {
                return Ok(Vec::new());
            }
            session_state
                .turn_aliases
                .note_terminal_turn(payload.turn.id.as_str());
            let turn_id = session_state
                .turn_aliases
                .ensure_crp_turn_id(payload.turn.id.as_str());
            let (status, error) = match payload.turn.status.as_str() {
                "completed" => (CrpTurnStatus::Success, None),
                "interrupted" => (CrpTurnStatus::Interrupted, None),
                "failed" => {
                    let error = payload.turn.error.map(|err| CrpTurnError {
                        message: err.message,
                        kind: Some("app_server_error".to_string()),
                        details: merge_error_details(err.codex_error_info, err.additional_details),
                    });
                    (CrpTurnStatus::Error, error)
                }
                _ => (CrpTurnStatus::Canceled, None),
            };
            let context_window = if matches!(status, CrpTurnStatus::Success) {
                session_state
                    .turn_aliases
                    .take_context_window_for_app_turn(payload.turn.id.as_str())
            } else {
                None
            };
            Ok(vec![(
                CrpChannel::Control,
                CrpEvent::TurnCompleted {
                    session_id: session_state.tracker.session_id.clone(),
                    turn_id,
                    status,
                    context_window,
                    error,
                },
            )])
        }
        "item/agentMessage/delta" => {
            let payload: AgentMessageDeltaNotification = serde_json::from_value(params)?;
            if payload.thread_id != session_state.thread_id {
                return Ok(Vec::new());
            }
            let turn = session_state.tracker.ensure_turn(payload.turn_id.as_str());
            turn.message_id = Some(payload.item_id.clone());
            Ok(vec![(
                CrpChannel::Data,
                CrpEvent::MessageDelta {
                    session_id: session_state.tracker.session_id.clone(),
                    turn_id: session_state
                        .turn_aliases
                        .ensure_crp_turn_id(payload.turn_id.as_str()),
                    message_id: payload.item_id,
                    delta: payload.delta,
                },
            )])
        }
        "item/reasoning/summaryPartAdded" => {
            let payload: ReasoningSummaryPartAddedNotification = serde_json::from_value(params)?;
            if payload.thread_id != session_state.thread_id {
                return Ok(Vec::new());
            }
            let turn = session_state.tracker.ensure_turn(payload.turn_id.as_str());
            turn.reasoning_summaries
                .entry((payload.item_id, payload.summary_index))
                .or_default();
            Ok(Vec::new())
        }
        "item/reasoning/summaryTextDelta" => {
            let payload: ReasoningSummaryTextDeltaNotification = serde_json::from_value(params)?;
            if payload.thread_id != session_state.thread_id {
                return Ok(Vec::new());
            }
            let summary_text = {
                let turn = session_state.tracker.ensure_turn(payload.turn_id.as_str());
                let state = turn
                    .reasoning_summaries
                    .entry((payload.item_id.clone(), payload.summary_index))
                    .or_default();
                state.text.push_str(&payload.delta);
                state.text.clone()
            };
            Ok(vec![(
                CrpChannel::Control,
                CrpEvent::ReasoningSummary {
                    session_id: session_state.tracker.session_id.clone(),
                    turn_id: session_state
                        .turn_aliases
                        .ensure_crp_turn_id(payload.turn_id.as_str()),
                    summary_index: payload.summary_index,
                    text: summary_text,
                    item_id: Some(payload.item_id),
                },
            )])
        }
        "item/reasoning/textDelta" => {
            let payload: ReasoningTextDeltaNotification = serde_json::from_value(params)?;
            if payload.thread_id != session_state.thread_id {
                return Ok(Vec::new());
            }
            let turn = session_state.tracker.ensure_turn(payload.turn_id.as_str());
            turn.reasoning_text_seen.insert(payload.item_id.clone());
            let _ = payload.content_index;
            Ok(vec![(
                CrpChannel::Data,
                CrpEvent::ReasoningTrace {
                    session_id: session_state.tracker.session_id.clone(),
                    turn_id: session_state
                        .turn_aliases
                        .ensure_crp_turn_id(payload.turn_id.as_str()),
                    chunk: payload.delta,
                    encoding: None,
                    summary_index: 0,
                    item_id: Some(payload.item_id),
                },
            )])
        }
        "item/commandExecution/outputDelta" => {
            let payload: CommandExecutionOutputDeltaNotification = serde_json::from_value(params)?;
            if payload.thread_id != session_state.thread_id {
                return Ok(Vec::new());
            }
            session_state.command_execution_seen = true;
            Ok(vec![(
                CrpChannel::Data,
                CrpEvent::ToolOutputDelta {
                    session_id: session_state.tracker.session_id.clone(),
                    turn_id: session_state
                        .turn_aliases
                        .ensure_crp_turn_id(payload.turn_id.as_str()),
                    tool_call_id: payload.item_id,
                    stream: None,
                    chunk: payload.delta,
                },
            )])
        }
        "item/started" | "item/completed" => {
            let payload: ItemLifecycleNotification = serde_json::from_value(params)?;
            if payload.thread_id != session_state.thread_id {
                return Ok(Vec::new());
            }
            translate_item_lifecycle(session_state, method == "item/completed", payload)
        }
        "error" => Ok(Vec::new()),
        _ => Ok(Vec::new()),
    }
}

fn translate_item_lifecycle(
    session_state: &mut AppServerSessionState,
    completed: bool,
    payload: ItemLifecycleNotification,
) -> Result<Vec<(CrpChannel, CrpEvent)>> {
    let session_id = session_state.tracker.session_id.clone();
    let crp_turn_id = session_state
        .turn_aliases
        .ensure_crp_turn_id(payload.turn_id.as_str());
    let turn = session_state.tracker.ensure_turn(payload.turn_id.as_str());

    let events = match payload.item {
        ThreadItem::AgentMessage { id, text, .. } if completed => {
            turn.message_id = Some(id.clone());
            turn.emitted_final = true;
            vec![(
                CrpChannel::Control,
                CrpEvent::MessageFinal {
                    session_id,
                    turn_id: crp_turn_id,
                    message_id: id,
                    content: text,
                },
            )]
        }
        ThreadItem::Reasoning {
            id,
            summary,
            content,
        } if completed => {
            let mut out = Vec::new();
            for (summary_index, text) in summary.into_iter().enumerate() {
                let summary_index = summary_index as i64;
                let state = turn
                    .reasoning_summaries
                    .entry((id.clone(), summary_index))
                    .or_default();
                if state.text.is_empty() {
                    state.text = text.clone();
                    out.push((
                        CrpChannel::Control,
                        CrpEvent::ReasoningSummary {
                            session_id: session_id.clone(),
                            turn_id: crp_turn_id.clone(),
                            summary_index,
                            text,
                            item_id: Some(id.clone()),
                        },
                    ));
                }
            }
            if !turn.reasoning_text_seen.contains(&id) {
                for block in content {
                    if block.is_empty() {
                        continue;
                    }
                    out.push((
                        CrpChannel::Data,
                        CrpEvent::ReasoningTraceFinal {
                            session_id: session_id.clone(),
                            turn_id: crp_turn_id.clone(),
                            content: block,
                            encoding: None,
                            summary_index: 0,
                            item_id: Some(id.clone()),
                        },
                    ));
                }
            }
            out
        }
        ThreadItem::CommandExecution {
            id,
            command,
            cwd,
            status,
            command_actions,
            aggregated_output,
            exit_code,
            duration_ms,
            ..
        } => {
            session_state.command_execution_seen = true;
            let input_preview = json!({
                "command": command,
                "cwd": cwd,
                "command_actions": command_actions,
            });
            if completed {
                let (status, error) = match status.as_str() {
                    "completed" => (CrpToolStatus::Success, None),
                    "declined" => (CrpToolStatus::Error, Some("command_declined".to_string())),
                    _ => (
                        CrpToolStatus::Error,
                        exit_code.map(|code| format!("exit_code: {code}")),
                    ),
                };
                vec![(
                    CrpChannel::Control,
                    CrpEvent::ToolCompleted {
                        session_id,
                        turn_id: crp_turn_id,
                        tool_call_id: id,
                        tool_name: "exec".to_string(),
                        tool_label: Some("Ran".to_string()),
                        status,
                        output: Some(json!({
                            "aggregated_output": aggregated_output,
                            "exit_code": exit_code,
                            "duration_ms": duration_ms,
                        })),
                        error,
                        input_preview: Some(input_preview),
                    },
                )]
            } else {
                vec![(
                    CrpChannel::Control,
                    CrpEvent::ToolStarted {
                        session_id,
                        turn_id: crp_turn_id,
                        tool_call_id: id,
                        tool_name: "exec".to_string(),
                        tool_label: Some("Running".to_string()),
                        input: Some(input_preview.clone()),
                        input_preview: Some(input_preview),
                    },
                )]
            }
        }
        ThreadItem::FileChange {
            id,
            changes,
            status,
        } => {
            let preview = patch_input_preview(&changes);
            if completed {
                let (status, error) = match status.as_str() {
                    "completed" => (CrpToolStatus::Success, None),
                    "declined" => (
                        CrpToolStatus::Error,
                        Some("apply_patch_declined".to_string()),
                    ),
                    _ => (CrpToolStatus::Error, Some("apply_patch_failed".to_string())),
                };
                vec![(
                    CrpChannel::Control,
                    CrpEvent::ToolCompleted {
                        session_id,
                        turn_id: crp_turn_id,
                        tool_call_id: id,
                        tool_name: "apply_patch".to_string(),
                        tool_label: Some("Edited".to_string()),
                        status,
                        output: Some(json!({ "changes": changes })),
                        error,
                        input_preview: Some(preview),
                    },
                )]
            } else {
                vec![(
                    CrpChannel::Control,
                    CrpEvent::ToolStarted {
                        session_id,
                        turn_id: crp_turn_id,
                        tool_call_id: id,
                        tool_name: "apply_patch".to_string(),
                        tool_label: Some("Edited".to_string()),
                        input: Some(json!({ "preview": preview.clone() })),
                        input_preview: Some(preview),
                    },
                )]
            }
        }
        ThreadItem::McpToolCall {
            id,
            server,
            tool,
            status,
            arguments,
            result,
            error,
            duration_ms,
        } => {
            let tool_name = format!("mcp.{server}.{tool}");
            let input_preview = json!({ "server": server, "tool": tool });
            if completed {
                let (status, error) = match status.as_str() {
                    "completed" => (CrpToolStatus::Success, None),
                    _ => (CrpToolStatus::Error, error.map(|error| error.message)),
                };
                vec![(
                    CrpChannel::Control,
                    CrpEvent::ToolCompleted {
                        session_id,
                        turn_id: crp_turn_id,
                        tool_call_id: id,
                        tool_name,
                        tool_label: Some("MCP".to_string()),
                        status,
                        output: Some(json!({
                            "duration_ms": duration_ms,
                            "result": result,
                        })),
                        error,
                        input_preview: Some(input_preview),
                    },
                )]
            } else {
                vec![(
                    CrpChannel::Control,
                    CrpEvent::ToolStarted {
                        session_id,
                        turn_id: crp_turn_id,
                        tool_call_id: id,
                        tool_name,
                        tool_label: Some("MCP".to_string()),
                        input: Some(json!({
                            "server": input_preview["server"],
                            "tool": input_preview["tool"],
                            "arguments": arguments,
                        })),
                        input_preview: Some(input_preview),
                    },
                )]
            }
        }
        ThreadItem::WebSearch { id, query, .. } => {
            if completed {
                vec![(
                    CrpChannel::Control,
                    CrpEvent::ToolCompleted {
                        session_id,
                        turn_id: crp_turn_id,
                        tool_call_id: id,
                        tool_name: "web_search".to_string(),
                        tool_label: Some("Searched".to_string()),
                        status: CrpToolStatus::Success,
                        output: Some(json!({ "query": query })),
                        error: None,
                        input_preview: Some(json!({ "query": query })),
                    },
                )]
            } else {
                vec![(
                    CrpChannel::Control,
                    CrpEvent::ToolStarted {
                        session_id,
                        turn_id: crp_turn_id,
                        tool_call_id: id,
                        tool_name: "web_search".to_string(),
                        tool_label: Some("Search".to_string()),
                        input: None,
                        input_preview: None,
                    },
                )]
            }
        }
        ThreadItem::ImageView { id, path } => {
            let payload = json!({ "path": path });
            if completed {
                vec![(
                    CrpChannel::Control,
                    CrpEvent::ToolCompleted {
                        session_id,
                        turn_id: crp_turn_id,
                        tool_call_id: id,
                        tool_name: "view_image".to_string(),
                        tool_label: Some("Viewed".to_string()),
                        status: CrpToolStatus::Success,
                        output: Some(payload),
                        error: None,
                        input_preview: None,
                    },
                )]
            } else {
                vec![(
                    CrpChannel::Control,
                    CrpEvent::ToolStarted {
                        session_id,
                        turn_id: crp_turn_id,
                        tool_call_id: id,
                        tool_name: "view_image".to_string(),
                        tool_label: Some("View".to_string()),
                        input: Some(payload.clone()),
                        input_preview: Some(payload),
                    },
                )]
            }
        }
        ThreadItem::ContextCompaction { .. } if completed => vec![
            (
                CrpChannel::Control,
                CrpEvent::SessionNotice {
                    session_id: session_id.clone(),
                    turn_id: Some(crp_turn_id.clone()),
                    code: "context.compacted".to_string(),
                    severity: Some("info".to_string()),
                    message: Some("Context compacted. Earlier turns were summarized.".to_string()),
                    details: None,
                    transient: None,
                },
            ),
            (
                CrpChannel::Control,
                CrpEvent::SessionGap {
                    session_id: session_id.clone(),
                    turn_id: Some(crp_turn_id.clone()),
                    reason: Some("context_compacted".to_string()),
                },
            ),
        ],
        ThreadItem::EnteredReviewMode { .. }
        | ThreadItem::ExitedReviewMode { .. }
        | ThreadItem::ContextCompaction { .. }
        | ThreadItem::Unknown
        | ThreadItem::AgentMessage { .. }
        | ThreadItem::Reasoning { .. } => Vec::new(),
    };
    Ok(events)
}

fn merge_error_details(
    codex_error_info: Option<Value>,
    additional_details: Option<String>,
) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(value) = codex_error_info {
        parts.push(value.to_string());
    }
    if let Some(details) = additional_details {
        let trimmed = details.trim();
        if !trimmed.is_empty() {
            parts.push(trimmed.to_string());
        }
    }
    (!parts.is_empty()).then(|| parts.join("\n"))
}

fn patch_input_preview(changes: &[crate::app_server::FileUpdateChange]) -> Value {
    let mut paths: Vec<String> = changes.iter().map(|change| change.path.clone()).collect();
    paths.sort();
    let mut added = 0usize;
    let mut removed = 0usize;
    for change in changes {
        for line in change.diff.lines() {
            if line.starts_with("+++") || line.starts_with("---") {
                continue;
            }
            if line.starts_with('+') {
                added += 1;
            } else if line.starts_with('-') {
                removed += 1;
            }
        }
    }
    json!({
        "paths": paths,
        "diff_stats": {
            "added": added,
            "removed": removed,
            "files": changes.len(),
        }
    })
}

pub(super) fn canonical_context_window_from_thread_usage(
    token_usage: &crate::app_server::ThreadTokenUsage,
) -> Option<Value> {
    let context_window_tokens = token_usage.model_context_window?;
    if context_window_tokens == 0 {
        return None;
    }
    // Codex thread totals can be cumulative across the thread lifetime. The live
    // context meter should reflect the current sampled context footprint instead.
    let total_tokens = token_usage.last.total_tokens;
    let input_tokens = token_usage.last.input_tokens;
    let output_tokens = token_usage.last.output_tokens;
    let reasoning_output_tokens = token_usage.last.reasoning_output_tokens;
    let remaining_tokens_estimate = context_window_tokens.saturating_sub(total_tokens);
    let remaining_fraction = remaining_tokens_estimate as f64 / context_window_tokens as f64;
    Some(json!({
        "context_tokens_estimate": total_tokens,
        "context_window_tokens": context_window_tokens,
        "remaining_tokens_estimate": remaining_tokens_estimate,
        "remaining_fraction": remaining_fraction,
        "total_input_tokens": input_tokens,
        "total_output_tokens": output_tokens.saturating_add(reasoning_output_tokens),
    }))
}

pub(super) fn emit_unsupported_server_request_notice(
    router: &CrpEventRouter,
    session_id: &str,
    turn_id: Option<String>,
    code: &str,
    method: &str,
) {
    dispatch_event(
        router,
        CrpChannel::Control,
        CrpEvent::SessionNotice {
            session_id: session_id.to_string(),
            turn_id,
            code: code.to_string(),
            severity: Some("warning".to_string()),
            message: Some(format!(
                "app-server request `{method}` is not supported by codex-crp"
            )),
            details: Some(json!({ "request_method": method })),
            transient: Some(false),
        },
    );
}

pub(super) fn emit_turn_request_error(
    router: &CrpEventRouter,
    session_id: &str,
    turn_id: Option<String>,
    kind: &str,
    message: String,
) {
    let Some(turn_id) = turn_id else {
        warn!(%kind, %message, "request failed without turn_id; unable to emit turn.completed");
        return;
    };
    dispatch_event(
        router,
        CrpChannel::Control,
        CrpEvent::TurnCompleted {
            session_id: session_id.to_string(),
            turn_id,
            status: CrpTurnStatus::Error,
            context_window: None,
            error: Some(CrpTurnError {
                message,
                kind: Some(kind.to_string()),
                details: None,
            }),
        },
    );
}
