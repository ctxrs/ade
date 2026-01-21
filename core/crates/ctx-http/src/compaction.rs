use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::mpsc;

use ctx_core::ids::{MessageId, RunId, TurnId};
use ctx_core::models::{
    Message, MessageAttachment, MessageRole, Session, SessionEvent, SessionEventType,
    SessionSummaryCheckpoint,
};
use ctx_providers::adapters::TurnInput;
use ctx_providers::events::NormalizedEvent;
use uuid::Uuid;

use crate::daemon::AppState;
use crate::installer;
use crate::provider_accounts;
use crate::settings::{AutoCompactionSettings, CompactionSettings, ProviderControlMode};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionTriggerKind {
    Manual,
    Auto,
}

#[derive(Debug, Clone)]
pub struct CompactionRequest {
    pub compaction_id: Uuid,
    pub kind: CompactionTriggerKind,
    pub reason: String,
    pub context_window: Option<Value>,
    pub exclude_message_id: Option<MessageId>,
    pub origin_run_id: Option<RunId>,
    pub origin_turn_id: Option<TurnId>,
}

#[derive(Debug, Clone)]
pub struct CompactionOutcome {
    pub summary: String,
    pub seed_text: String,
}

#[derive(Debug, Clone, Serialize)]
struct CompactionSessionInfo {
    session_id: String,
    task_id: String,
    workspace_id: String,
    worktree_id: String,
    provider_id: String,
    model_id: String,
}

#[derive(Debug, Clone, Serialize)]
struct CompactionTriggerInfo {
    kind: CompactionTriggerKind,
    reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    context_window: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
struct CompactionSettingsSnapshot {
    enabled: bool,
    script_path: Option<String>,
    script_timeout_ms: Option<u64>,
    retain_full_transcript_tokens: Option<u32>,
    retain_tail_messages: Option<u32>,
    retain_tail_chars_per_message: Option<u32>,
    include_attachments: bool,
    auto_compact: Option<AutoCompactionSettings>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CompactionAttachment {
    kind: String,
    mime_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    blob_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    byte_len: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CompactionMessage {
    id: String,
    role: MessageRole,
    content: String,
    #[serde(default)]
    attachments: Vec<CompactionAttachment>,
    created_at: String,
}

#[derive(Debug, Serialize)]
struct CompactionScriptInput {
    session: CompactionSessionInfo,
    trigger: CompactionTriggerInfo,
    settings: CompactionSettingsSnapshot,
    transcript: Vec<CompactionMessage>,
    candidate_transcript: Vec<CompactionMessage>,
}

#[derive(Debug, Deserialize)]
struct CompactionScriptOutput {
    summary: String,
    #[serde(default)]
    seed_text: Option<String>,
    #[serde(default)]
    tail_message_ids: Option<Vec<String>>,
    #[serde(default)]
    tail_messages: Option<Vec<CompactionMessage>>,
}

pub fn is_compact_command(content: &str) -> bool {
    let trimmed = content.trim();
    trimmed == "/compact" || trimmed.starts_with("/compact ")
}

pub fn should_auto_compact(metrics: &Value, settings: &CompactionSettings) -> bool {
    let Some(auto) = settings.auto_compact.as_ref() else {
        return false;
    };
    if !settings.enabled || !auto.enabled {
        return false;
    }

    let remaining_fraction = metrics.get("remaining_fraction").and_then(Value::as_f64);
    let context_tokens = metrics
        .get("context_tokens_estimate")
        .and_then(Value::as_u64);

    if let Some(threshold) = auto.remaining_fraction_threshold {
        if let Some(remaining) = remaining_fraction {
            if remaining <= threshold {
                return true;
            }
        }
    }

    if let Some(max_tokens) = auto.max_context_tokens {
        if let Some(estimate) = context_tokens {
            if estimate >= max_tokens as u64 {
                return true;
            }
        }
    }

    false
}

pub async fn run_compaction(
    state: &AppState,
    session: &Session,
    workdir: &Path,
    request: &CompactionRequest,
) -> Result<CompactionOutcome> {
    let settings = crate::settings::load_settings(&state.data_root).await;
    let compaction = settings
        .compaction
        .as_ref()
        .context("compaction settings missing")?;

    if !compaction.enabled {
        return Err(anyhow!("compaction is disabled"));
    }

    if matches!(request.kind, CompactionTriggerKind::Auto) {
        let auto_enabled = compaction
            .auto_compact
            .as_ref()
            .map(|auto| auto.enabled)
            .unwrap_or(false);
        if !auto_enabled {
            return Err(anyhow!("auto compaction disabled"));
        }
    }

    let script_path = compaction
        .script_path
        .as_ref()
        .map(|path| path.trim().to_string())
        .filter(|path| !path.is_empty());

    let store = state.store_for_session(session.id).await?;
    let mut messages = store.list_messages_for_session(session.id).await?;
    if let Some(exclude_id) = request.exclude_message_id {
        messages.retain(|msg| msg.id != exclude_id);
    }

    let full_transcript = if script_path.is_some() {
        build_compaction_messages(&messages, compaction.retain_tail_chars_per_message)
    } else {
        build_compaction_messages(&messages, None)
    };
    let candidate_transcript = build_candidate_transcript(&full_transcript, compaction);

    let output = if let Some(script_path) = script_path.as_deref() {
        let input = CompactionScriptInput {
            session: CompactionSessionInfo {
                session_id: session.id.0.to_string(),
                task_id: session.task_id.0.to_string(),
                workspace_id: session.workspace_id.0.to_string(),
                worktree_id: session.worktree_id.0.to_string(),
                provider_id: session.provider_id.clone(),
                model_id: session.model_id.clone(),
            },
            trigger: CompactionTriggerInfo {
                kind: request.kind,
                reason: request.reason.clone(),
                context_window: request.context_window.clone(),
            },
            settings: CompactionSettingsSnapshot {
                enabled: compaction.enabled,
                script_path: compaction.script_path.clone(),
                script_timeout_ms: compaction.script_timeout_ms,
                retain_full_transcript_tokens: compaction.retain_full_transcript_tokens,
                retain_tail_messages: compaction.retain_tail_messages,
                retain_tail_chars_per_message: compaction.retain_tail_chars_per_message,
                include_attachments: compaction.include_attachments,
                auto_compact: compaction.auto_compact.clone(),
            },
            transcript: full_transcript.clone(),
            candidate_transcript: candidate_transcript.clone(),
        };

        run_compaction_script(
            script_path,
            compaction.script_timeout_ms,
            &input,
            workdir,
            session,
            request,
        )
        .await?
    } else {
        run_default_compaction(
            state,
            session,
            workdir,
            request,
            compaction,
            &full_transcript,
            &candidate_transcript,
        )
        .await?
    };

    let summary = output.summary.trim().to_string();
    if summary.is_empty() {
        return Err(anyhow!("compaction summary is required"));
    }

    let tail_messages = if let Some(tail) = output.tail_messages {
        tail
    } else if let Some(ids) = output.tail_message_ids {
        let lookup = build_message_lookup(&full_transcript);
        ids.into_iter()
            .filter_map(|id| lookup.get(&id).cloned())
            .collect()
    } else {
        candidate_transcript.clone()
    };

    let seed_text = if let Some(seed) = output.seed_text {
        seed
    } else {
        render_seed_text(&summary, &tail_messages, compaction.include_attachments)
    };

    if seed_text.trim().is_empty() {
        return Err(anyhow!("compaction seed text is empty"));
    }

    let now = Utc::now();
    let last_event_seq = store.get_session_last_event_seq(session.id).await?;
    let last_turn_id = messages.iter().rev().find_map(|msg| msg.turn_id);
    let checkpoint = SessionSummaryCheckpoint {
        session_id: session.id,
        checkpoint_id: uuid::Uuid::new_v4().to_string(),
        summary: summary.clone(),
        last_turn_id,
        last_event_seq: Some(last_event_seq),
        created_at: now,
        updated_at: now,
    };
    let _ = store.upsert_session_summary_checkpoint(checkpoint).await?;

    let _ = store
        .upsert_session_compaction_seed(session.id, seed_text.clone(), request.reason.clone())
        .await?;
    store
        .update_session_provider_session_ref(session.id, None)
        .await?;

    Ok(CompactionOutcome { summary, seed_text })
}

fn build_compaction_messages(
    messages: &[Message],
    max_chars: Option<u32>,
) -> Vec<CompactionMessage> {
    let cap = max_chars
        .map(|value| value.max(1) as usize)
        .unwrap_or(usize::MAX);
    messages
        .iter()
        .map(|msg| CompactionMessage {
            id: msg.id.0.to_string(),
            role: msg.role.clone(),
            content: truncate_content(&msg.content, cap),
            attachments: msg
                .attachments
                .iter()
                .map(CompactionAttachment::from)
                .collect(),
            created_at: msg.created_at.to_rfc3339(),
        })
        .collect()
}

fn build_candidate_transcript(
    transcript: &[CompactionMessage],
    settings: &CompactionSettings,
) -> Vec<CompactionMessage> {
    let token_limit = settings.retain_full_transcript_tokens.unwrap_or(0) as usize;
    if token_limit > 0 {
        let combined = render_transcript_for_estimate(transcript);
        if estimate_tokens(&combined) <= token_limit {
            return transcript.to_vec();
        }
    }

    let tail_limit = settings.retain_tail_messages.unwrap_or(40) as usize;
    if tail_limit == 0 || transcript.len() <= tail_limit {
        return transcript.to_vec();
    }
    transcript[transcript.len().saturating_sub(tail_limit)..].to_vec()
}

fn render_transcript_for_estimate(transcript: &[CompactionMessage]) -> String {
    let mut out = String::new();
    for msg in transcript {
        let role = match msg.role {
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
            MessageRole::System => "system",
        };
        out.push_str(role);
        out.push_str(": ");
        out.push_str(&msg.content);
        out.push('\n');
    }
    out
}

fn render_seed_text(
    summary: &str,
    tail: &[CompactionMessage],
    include_attachments: bool,
) -> String {
    let mut out = String::new();
    out.push_str("CTX COMPACTION SUMMARY\n");
    out.push_str(summary.trim());
    out.push_str("\n\nCTX TRANSCRIPT TAIL\n");
    for msg in tail {
        let role = match msg.role {
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
            MessageRole::System => "system",
        };
        out.push_str(role);
        out.push_str(": ");
        out.push_str(msg.content.trim());
        if include_attachments && !msg.attachments.is_empty() {
            let attachment_line = render_attachment_line(&msg.attachments);
            if !attachment_line.is_empty() {
                out.push('\n');
                out.push_str(&attachment_line);
            }
        }
        out.push('\n');
    }
    out
}

fn render_attachment_line(attachments: &[CompactionAttachment]) -> String {
    let mut parts = Vec::new();
    for att in attachments {
        let mut desc = format!("{}:{}", att.kind, att.mime_type);
        if let Some(name) = att.name.as_ref().filter(|v| !v.trim().is_empty()) {
            desc.push_str(&format!(" name={}", name.trim()));
        }
        if let Some(blob_id) = att.blob_id.as_ref() {
            desc.push_str(&format!(" blob_id={}", blob_id));
        }
        if let Some(byte_len) = att.byte_len {
            desc.push_str(&format!(" bytes={}", byte_len));
        }
        parts.push(desc);
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("attachments: {}", parts.join(", "))
    }
}

fn truncate_content(content: &str, max_chars: usize) -> String {
    if max_chars == 0 || content.chars().count() <= max_chars {
        return content.to_string();
    }
    let truncated: String = content.chars().take(max_chars).collect();
    format!("{}...", truncated)
}

fn estimate_tokens(text: &str) -> usize {
    let chars = text.chars().count();
    chars.div_ceil(4)
}

impl From<&MessageAttachment> for CompactionAttachment {
    fn from(att: &MessageAttachment) -> Self {
        match att {
            MessageAttachment::Image {
                mime_type,
                data_base64,
                name,
            } => Self {
                kind: "image_inline".to_string(),
                mime_type: mime_type.clone(),
                name: name.clone(),
                blob_id: None,
                byte_len: Some(data_base64.len()),
            },
            MessageAttachment::ImageRef {
                blob_id,
                mime_type,
                name,
            } => Self {
                kind: "image_ref".to_string(),
                mime_type: mime_type.clone(),
                name: name.clone(),
                blob_id: Some(blob_id.clone()),
                byte_len: None,
            },
        }
    }
}

fn build_message_lookup(transcript: &[CompactionMessage]) -> HashMap<String, CompactionMessage> {
    let mut map = HashMap::new();
    for msg in transcript {
        map.insert(msg.id.clone(), msg.clone());
    }
    map
}

async fn run_default_compaction(
    state: &AppState,
    session: &Session,
    workdir: &Path,
    _request: &CompactionRequest,
    compaction: &CompactionSettings,
    full_transcript: &[CompactionMessage],
    candidate_transcript: &[CompactionMessage],
) -> Result<CompactionScriptOutput> {
    let store = state.store_for_session(session.id).await?;
    let events = store.list_session_events(session.id).await?;
    let session_log = render_session_log(&events);
    let transcript_text =
        render_compaction_transcript(full_transcript, compaction.include_attachments);
    let prompt = build_summary_prompt(&session_log, &transcript_text);

    let summary = run_compaction_summary_llm(
        state,
        session,
        workdir,
        &prompt,
        compaction.script_timeout_ms,
    )
    .await?;
    let seed_transcript =
        render_compaction_transcript(candidate_transcript, compaction.include_attachments);
    let seed_text = render_default_seed_text(&summary, &seed_transcript);

    Ok(CompactionScriptOutput {
        summary,
        seed_text: Some(seed_text),
        tail_message_ids: None,
        tail_messages: Some(candidate_transcript.to_vec()),
    })
}

fn build_summary_prompt(session_log: &str, transcript: &str) -> String {
    let mut out = String::new();
    out.push_str("You are generating a compaction summary for a coding session.\n");
    out.push_str(
        "Summarize the session for continuation, capturing intent, key decisions, open items, and referenced files or commands.\n",
    );
    out.push_str("Return JSON only with a single key: {\"summary\":\"...\"}.\n\n");
    out.push_str("<session_log>\n");
    out.push_str(session_log);
    out.push_str("\n</session_log>\n\n");
    out.push_str("<transcript>\n");
    out.push_str(transcript);
    out.push_str("\n</transcript>\n");
    out
}

fn render_default_seed_text(summary: &str, transcript: &str) -> String {
    let mut out = String::new();
    out.push_str(
        "You are continuing a session that has been compacted. Below you will find the raw transcript with the user. Other context such as files you've read and commands you've run have been replaced by an LLM-generated summary. After reading the summary and transcript, catch up with the necessary context (such as referenced files or docs) and continue from wherever you last left off with the user.\n\n",
    );
    out.push_str("<summary>\n");
    out.push_str(summary.trim());
    out.push_str("\n</summary>\n\n");
    out.push_str("<transcript>\n");
    out.push_str(transcript);
    out.push_str("\n</transcript>");
    out
}

fn render_compaction_transcript(
    transcript: &[CompactionMessage],
    include_attachments: bool,
) -> String {
    let mut out = String::new();
    for (idx, msg) in transcript.iter().enumerate() {
        let role = match msg.role {
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
            MessageRole::System => "system",
        };
        out.push_str(role);
        out.push_str(": ");
        out.push_str(&msg.content);
        if include_attachments && !msg.attachments.is_empty() {
            let attachment_line = render_attachment_line(&msg.attachments);
            if !attachment_line.is_empty() {
                out.push('\n');
                out.push_str(&attachment_line);
            }
        }
        if idx + 1 < transcript.len() {
            out.push_str("\n\n");
        }
    }
    out
}

fn render_session_log(events: &[SessionEvent]) -> String {
    let mut out = String::new();
    for event in events {
        let entry = serde_json::json!({
            "seq": event.seq,
            "created_at": event.created_at.to_rfc3339(),
            "event_type": compaction_event_type_label(&event.event_type),
            "run_id": event.run_id.map(|id| id.0.to_string()),
            "turn_id": event.turn_id.map(|id| id.0.to_string()),
            "payload": event.payload_json.clone(),
        });
        out.push_str(&entry.to_string());
        out.push('\n');
    }
    out
}

fn compaction_event_type_label(event_type: &SessionEventType) -> &'static str {
    match event_type {
        SessionEventType::Init => "init",
        SessionEventType::UserMessage => "user_message",
        SessionEventType::InputQueued => "input_queued",
        SessionEventType::AuthRequired => "auth_required",
        SessionEventType::Notice => "notice",
        SessionEventType::AssistantChunk => "assistant_chunk",
        SessionEventType::ThoughtChunk => "thought_chunk",
        SessionEventType::AssistantComplete => "assistant_complete",
        SessionEventType::AssistantMessageInserted => "assistant_message_inserted",
        SessionEventType::ToolCall => "tool_call",
        SessionEventType::ToolCallUpdate => "tool_call_update",
        SessionEventType::ToolResult => "tool_result",
        SessionEventType::Plan => "plan",
        SessionEventType::ArtifactsSet => "artifacts_set",
        SessionEventType::Done => "done",
        SessionEventType::InterruptRequested => "interrupt_requested",
        SessionEventType::TurnInterrupted => "turn_interrupted",
        SessionEventType::Error => "error",
    }
}

async fn run_compaction_summary_llm(
    state: &AppState,
    session: &Session,
    workdir: &Path,
    prompt: &str,
    timeout_ms: Option<u64>,
) -> Result<String> {
    let adapter = {
        let map = state.providers.lock().await;
        map.get(&session.provider_id)
            .cloned()
            .ok_or_else(|| anyhow!("provider not available: {}", session.provider_id))?
    };

    let (ev_tx, mut ev_rx) = mpsc::channel::<NormalizedEvent>(128);
    let provider_env = build_compaction_provider_env(state, session, workdir).await?;
    let handle = adapter
        .run(
            TurnInput {
                content: prompt.to_string(),
                attachments: Vec::new(),
                context_blocks: Vec::new(),
                model_id: normalize_session_model_id(&session.model_id),
            },
            workdir.to_path_buf(),
            provider_env,
            ev_tx,
        )
        .await?;

    let mut done = handle.done;
    let cancel = handle.cancel;
    let abort = handle.abort;

    let timeout = Duration::from_millis(timeout_ms.unwrap_or(60000));
    let collected = tokio::time::timeout(timeout, async {
        let mut output = String::new();
        let mut completed: Option<String> = None;
        let mut error: Option<String> = None;

        loop {
            tokio::select! {
                maybe = ev_rx.recv() => {
                    let Some(ev) = maybe else { break; };
                    match ev.event_type {
                        SessionEventType::AssistantChunk => {
                            if let Some(fragment) = ev.payload_json.get("content_fragment").and_then(Value::as_str) {
                                output.push_str(fragment);
                            }
                        }
                        SessionEventType::AssistantComplete => {
                            let full = ev
                                .payload_json
                                .get("full_content")
                                .and_then(Value::as_str)
                                .or_else(|| ev.payload_json.get("content").and_then(Value::as_str))
                                .unwrap_or("");
                            if !full.trim().is_empty() {
                                completed = Some(full.to_string());
                            }
                        }
                        SessionEventType::Error => {
                            error = extract_compaction_error(&ev.payload_json);
                        }
                        _ => {}
                    }
                }
                _ = &mut done => {
                    break;
                }
            }
        }

        while let Some(ev) = ev_rx.recv().await {
            match ev.event_type {
                SessionEventType::AssistantChunk => {
                    if let Some(fragment) = ev.payload_json.get("content_fragment").and_then(Value::as_str) {
                        output.push_str(fragment);
                    }
                }
                SessionEventType::AssistantComplete => {
                    let full = ev
                        .payload_json
                        .get("full_content")
                        .and_then(Value::as_str)
                        .or_else(|| ev.payload_json.get("content").and_then(Value::as_str))
                        .unwrap_or("");
                    if !full.trim().is_empty() {
                        completed = Some(full.to_string());
                    }
                }
                SessionEventType::Error => {
                    error = extract_compaction_error(&ev.payload_json);
                }
                _ => {}
            }
        }

        if let Some(error) = error {
            return Err(anyhow!("compaction summary failed: {}", error));
        }

        Ok(completed.unwrap_or(output))
    })
    .await;

    if collected.is_err() {
        if let Some(cancel) = cancel {
            let _ = cancel.send(());
        }
        if let Some(abort) = abort {
            abort.abort();
        }
        return Err(anyhow!("compaction summary timed out"));
    }

    let raw = collected??;
    parse_summary_output(&raw)
}

async fn build_compaction_provider_env(
    state: &AppState,
    session: &Session,
    workdir: &Path,
) -> Result<HashMap<String, String>> {
    let mut provider_env = HashMap::new();
    provider_env.insert("CTX_DAEMON_URL".to_string(), state.daemon_url.clone());
    provider_env.insert(
        "CTX_DATA_ROOT".to_string(),
        state.data_root.to_string_lossy().to_string(),
    );
    provider_env.insert("CTX_SESSION_ID".to_string(), session.id.0.to_string());
    provider_env.insert("CTX_MODEL_ID".to_string(), session.model_id.clone());
    provider_env.insert("CTX_MCP_DISABLED".to_string(), "1".to_string());
    if let Some(token) = state.auth_token.clone() {
        provider_env.insert("CTX_AUTH_TOKEN".to_string(), token);
    }

    let provider_control_mode = crate::settings::load_settings(&state.data_root)
        .await
        .sandboxing
        .as_ref()
        .map(|s| s.provider_control_mode.clone())
        .unwrap_or_default();
    if let Some(mode_id) = provider_mode_id_for(&session.provider_id, &provider_control_mode) {
        provider_env.insert("CTX_PROVIDER_MODE".to_string(), mode_id.to_string());
    }

    if session.provider_id == "codex" {
        if let Ok(env) = provider_accounts::codex_env_for_active_account(&state.data_root).await {
            for (key, value) in env {
                provider_env.insert(key, value);
            }
        }
    }

    if let Ok(cfg) = installer::load_agent_server_config(&state.data_root).await {
        if let Some(cmd) = cfg.providers.get(&session.provider_id) {
            let mut bin_dirs: Vec<std::path::PathBuf> = Vec::new();
            for dep in &cmd.dependencies {
                if let Some(meta) = cfg.managed_installs.get(dep) {
                    if let Some(rel) = meta.bin_dir_rel.as_ref() {
                        bin_dirs.push(state.data_root.join(rel));
                    }
                }
            }
            if !bin_dirs.is_empty() {
                let mut path_parts: Vec<std::path::PathBuf> = bin_dirs;
                if let Some(current) = std::env::var_os("PATH") {
                    path_parts.extend(std::env::split_paths(&current));
                }
                if let Ok(joined) = std::env::join_paths(path_parts) {
                    provider_env.insert("PATH".to_string(), joined.to_string_lossy().to_string());
                }
            }
        }
    }

    if let Ok(v) = std::env::var("CTX_MCP_COMMAND") {
        provider_env.insert("CTX_MCP_COMMAND".to_string(), v);
    }
    if let Ok(v) = std::env::var("CTX_MCP_DISABLED") {
        provider_env
            .entry("CTX_MCP_DISABLED".to_string())
            .or_insert(v);
    }

    if workdir.as_os_str().is_empty() {
        return Err(anyhow!("compaction workdir is required"));
    }

    Ok(provider_env)
}

fn normalize_session_model_id(model_id: &str) -> Option<String> {
    let trimmed = model_id.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("default") {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn provider_mode_id_for(
    provider_id: &str,
    control_mode: &ProviderControlMode,
) -> Option<&'static str> {
    match control_mode {
        ProviderControlMode::Full => match provider_id {
            "codex" => Some("full-access"),
            "claude" => Some("bypassPermissions"),
            _ => None,
        },
        ProviderControlMode::HarnessNative | ProviderControlMode::CtxEnforced => None,
    }
}

fn extract_compaction_error(payload: &Value) -> Option<String> {
    payload
        .get("message")
        .or_else(|| payload.get("error"))
        .or_else(|| payload.get("detail"))
        .and_then(Value::as_str)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn parse_summary_output(raw: &str) -> Result<String> {
    let cleaned = strip_code_fence(raw).trim().to_string();
    if cleaned.is_empty() {
        return Err(anyhow!("compaction summary is required"));
    }
    if cleaned.starts_with('{') {
        if let Ok(parsed) = serde_json::from_str::<Value>(&cleaned) {
            let summary = parsed
                .get("summary")
                .and_then(Value::as_str)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty());
            if let Some(summary) = summary {
                return Ok(summary);
            }
        }
    }
    Ok(cleaned)
}

fn strip_code_fence(raw: &str) -> String {
    let trimmed = raw.trim();
    if !trimmed.starts_with("```") {
        return trimmed.to_string();
    }
    let mut lines = trimmed.lines();
    let _ = lines.next();
    let mut out_lines: Vec<&str> = lines.collect();
    if let Some(last) = out_lines.last() {
        if last.trim().starts_with("```") {
            out_lines.pop();
        }
    }
    out_lines.join("\n").trim().to_string()
}

async fn run_compaction_script(
    script_path: &str,
    timeout_ms: Option<u64>,
    input: &CompactionScriptInput,
    workdir: &Path,
    session: &Session,
    request: &CompactionRequest,
) -> Result<CompactionScriptOutput> {
    let path = resolve_script_path(script_path, workdir)?;
    let mut cmd = Command::new(&path);
    cmd.current_dir(workdir)
        .env("CTX_COMPACTION", "1")
        .env("CTX_SESSION_ID", session.id.0.to_string())
        .env("CTX_TASK_ID", session.task_id.0.to_string())
        .env("CTX_WORKSPACE_ID", session.workspace_id.0.to_string())
        .env("CTX_WORKTREE_ID", session.worktree_id.0.to_string())
        .env("CTX_PROVIDER_ID", session.provider_id.clone())
        .env("CTX_MODEL_ID", session.model_id.clone())
        .env(
            "CTX_COMPACTION_KIND",
            match request.kind {
                CompactionTriggerKind::Manual => "manual",
                CompactionTriggerKind::Auto => "auto",
            },
        )
        .env("CTX_COMPACTION_REASON", request.reason.clone())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let mut child = cmd.spawn().context("spawning compaction script")?;
    let input_json = serde_json::to_vec_pretty(input).context("serializing compaction input")?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(&input_json).await?;
    }

    let timeout = Duration::from_millis(timeout_ms.unwrap_or(15000));
    let output = tokio::time::timeout(timeout, child.wait_with_output())
        .await
        .context("compaction script timed out")??;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("compaction script failed: {}", stderr.trim()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: CompactionScriptOutput =
        serde_json::from_str(stdout.trim()).context("parsing compaction output")?;
    Ok(parsed)
}

fn resolve_script_path(script_path: &str, workdir: &Path) -> Result<PathBuf> {
    let candidate = PathBuf::from(script_path);
    if candidate.is_absolute() {
        return Ok(candidate);
    }
    Ok(workdir.join(candidate))
}
