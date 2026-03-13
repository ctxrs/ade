use std::fs::File;
use std::io::{BufRead, BufReader};
use std::collections::HashMap;
use std::path::PathBuf;

use super::*;

#[allow(dead_code)]
pub(super) async fn build_rehydrate_transcript_block(
    store: &ctx_store::Store,
    session_id: ctx_core::ids::SessionId,
) -> Result<serde_json::Value> {
    let msgs = store.list_messages_for_session(session_id).await?;
    if msgs.is_empty() {
        anyhow::bail!("no messages to rehydrate");
    }

    const MAX_MESSAGES: usize = 24;
    const MAX_CHARS_PER_MESSAGE: usize = 4000;
    let tail: Vec<_> = msgs
        .into_iter()
        .rev()
        .take(MAX_MESSAGES)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

    let mut text = String::new();
    text.push_str("Session transcript (for continuity after ctx daemon restart):\n\n");
    for m in tail {
        let role = match m.role {
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
            MessageRole::System => "system",
        };
        let mut content = m.content;
        if content.chars().count() > MAX_CHARS_PER_MESSAGE {
            content = content
                .chars()
                .take(MAX_CHARS_PER_MESSAGE)
                .collect::<String>();
            content.push_str("\n…(truncated)");
        }
        text.push_str(&format!(
            "[{}] {role}:\n{content}\n\n",
            m.created_at.to_rfc3339()
        ));
    }

    Ok(json!({
        "type": "resource",
        "resource": {
            "uri": format!("ctx://session/{}/transcript", session_id.0),
            "mimeType": "text/plain",
            "text": text
        }
    }))
}

pub(super) fn should_track_thought_chunk(payload: &serde_json::Value) -> bool {
    let meta = payload
        .get("acp_update")
        .and_then(|v| v.get("_meta"))
        .or_else(|| payload.get("acp_update").and_then(|v| v.get("meta")))
        .or_else(|| payload.get("_meta"))
        .or_else(|| payload.get("meta"));
    if meta
        .and_then(|v| v.get("heartbeat"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return false;
    }
    let has_status_text = |meta: &Value| {
        for key in ["status_text", "statusText", "status_string", "statusString"] {
            if meta.get(key).and_then(Value::as_str).is_some() {
                return true;
            }
        }
        if let Some(status) = meta.get("status").and_then(Value::as_str) {
            let s = status.trim().to_lowercase();
            let is_tool_status = matches!(
                s.as_str(),
                "pending"
                    | "queued"
                    | "running"
                    | "in_progress"
                    | "completed"
                    | "failed"
                    | "error"
                    | "ok"
                    | "success"
                    | "succeeded"
            );
            if !is_tool_status {
                return true;
            }
        }
        false
    };

    let reasoning_kind = meta
        .and_then(|v| v.get("codex"))
        .and_then(|v| v.get("reasoning_kind").or_else(|| v.get("reasoningKind")))
        .and_then(Value::as_str);
    if matches!(reasoning_kind, Some("summary" | "status")) {
        return false;
    }
    if let Some(meta) = meta {
        if has_status_text(meta) {
            return false;
        }
        if let Some(codex_meta) = meta.get("codex") {
            if has_status_text(codex_meta) {
                return false;
            }
        }
    }
    true
}

pub(super) fn strip_emitted_prefix(full_content: &str, emitted: &str) -> Option<String> {
    let full = full_content.trim_end_matches(['\r', '\n']);
    if full.is_empty() {
        return None;
    }
    let emitted_trimmed = emitted.trim_end_matches(|c: char| c.is_whitespace());
    if emitted_trimmed.is_empty() {
        return Some(full.to_string());
    }
    if full == emitted_trimmed {
        return None;
    }
    if full.starts_with(emitted_trimmed) {
        let suffix = full.get(emitted_trimmed.len()..).unwrap_or("").to_string();
        if suffix.trim().is_empty() {
            None
        } else {
            Some(suffix)
        }
    } else {
        Some(full.to_string())
    }
}

pub(super) fn compute_context_window_metrics(
    provider_id: &str,
    model_id: &str,
    prompt: &str,
) -> Option<serde_json::Value> {
    let context_window_tokens = model_context_window(provider_id, model_id)?;
    let context_tokens_estimate = estimate_tokens(prompt);
    let remaining_tokens_estimate = context_window_tokens.saturating_sub(context_tokens_estimate);
    let remaining_fraction = remaining_tokens_estimate as f64 / context_window_tokens as f64;
    Some(json!({
        "context_tokens_estimate": context_tokens_estimate,
        "context_window_tokens": context_window_tokens,
        "remaining_tokens_estimate": remaining_tokens_estimate,
        "remaining_fraction": remaining_fraction,
    }))
}

pub(super) fn read_codex_context_window_metrics(session_ref: &str) -> Option<serde_json::Value> {
    let path = find_codex_session_log(session_ref)?;
    let file = File::open(path).ok()?;
    let reader = BufReader::new(file);
    let mut latest_info: Option<Value> = None;

    for line in reader.lines().map_while(std::result::Result::ok) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let payload: Value = serde_json::from_str(trimmed).ok()?;
        if payload.get("type").and_then(Value::as_str) != Some("event_msg") {
            continue;
        }
        let event_payload = payload.get("payload")?;
        if event_payload.get("type").and_then(Value::as_str) != Some("token_count") {
            continue;
        }
        if let Some(info) = event_payload.get("info") {
            latest_info = Some(info.clone());
        }
    }

    let info = latest_info?;
    let context_window_tokens = info.get("model_context_window").and_then(Value::as_u64)?;
    let last_usage = info.get("last_token_usage").and_then(Value::as_object);
    let input_tokens = last_usage
        .and_then(|m| m.get("input_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output_tokens = last_usage
        .and_then(|m| m.get("output_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let reasoning_tokens = last_usage
        .and_then(|m| m.get("reasoning_output_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let total_tokens = last_usage
        .and_then(|m| m.get("total_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(
            input_tokens
                .saturating_add(output_tokens)
                .saturating_add(reasoning_tokens),
        );

    if context_window_tokens == 0 {
        return None;
    }
    let remaining_tokens_estimate = context_window_tokens.saturating_sub(total_tokens);
    let remaining_fraction = remaining_tokens_estimate as f64 / context_window_tokens as f64;

    Some(json!({
        "context_tokens_estimate": total_tokens,
        "context_window_tokens": context_window_tokens,
        "remaining_tokens_estimate": remaining_tokens_estimate,
        "remaining_fraction": remaining_fraction,
        "total_input_tokens": input_tokens,
        "total_output_tokens": output_tokens.saturating_add(reasoning_tokens),
    }))
}

fn find_codex_session_log(session_ref: &str) -> Option<PathBuf> {
    let base = directories::BaseDirs::new()?;
    let root = base.home_dir().join(".codex").join("sessions");
    if !root.exists() {
        return None;
    }
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.ends_with(".jsonl") && name.contains(session_ref) {
                return Some(path);
            }
        }
    }
    None
}

pub(crate) fn model_context_window(provider_id: &str, model_id: &str) -> Option<usize> {
    match (provider_id, model_id) {
        ("fake", "fake-model") => Some(100),
        _ => None,
    }
}

fn estimate_tokens(text: &str) -> usize {
    let chars = text.chars().count();
    chars.div_ceil(4)
}

pub(super) fn normalize_session_model_id(model_id: &str) -> Option<String> {
    let trimmed = model_id.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("default") {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub(super) fn provider_supports_system_prompt_append(provider_id: &str) -> bool {
    matches!(provider_id, "claude-crp" | "codex")
}

pub(super) fn runtime_provider_id_for_session_provider<'a>(
    session_provider_id: &'a str,
    _resolved_source: &harness_sources::ResolvedHarnessSource,
) -> &'a str {
    session_provider_id
}

pub(super) async fn apply_provider_launch_overrides(
    provider_id: &str,
    _workdir: &std::path::Path,
    provider_env: &mut HashMap<String, String>,
) -> Result<()> {
    if !matches!(provider_id, "opencode" | "kimi") {
        return Ok(());
    }

    // These providers currently behave truthfully only on their native ACP tool paths.
    provider_env.insert("CTX_MCP_DISABLED".to_string(), "1".to_string());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn opencode_launch_overrides_disable_mcp() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut env = HashMap::new();

        apply_provider_launch_overrides("opencode", temp.path(), &mut env)
            .await
            .expect("apply overrides");

        assert_eq!(env.get("CTX_MCP_DISABLED").map(String::as_str), Some("1"));
        assert!(!env.contains_key("ACP_CWD"));
    }

    #[tokio::test]
    async fn kimi_launch_overrides_disable_mcp() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut env = HashMap::new();

        apply_provider_launch_overrides("kimi", temp.path(), &mut env)
            .await
            .expect("apply overrides");

        assert_eq!(env.get("CTX_MCP_DISABLED").map(String::as_str), Some("1"));
        assert!(!env.contains_key("ACP_CWD"));
    }

    #[tokio::test]
    async fn unrelated_launch_overrides_leave_env_unchanged() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut env = HashMap::from([("CTX_MCP_DISABLED".to_string(), "0".to_string())]);

        apply_provider_launch_overrides("codex", temp.path(), &mut env)
            .await
            .expect("apply overrides");

        assert_eq!(env.get("CTX_MCP_DISABLED").map(String::as_str), Some("0"));
        assert!(!env.contains_key("ACP_CWD"));
    }
}
