use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::Context;

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
        .and_then(|v| v.get("codex-crp"))
        .and_then(|v| v.get("reasoning_kind").or_else(|| v.get("reasoningKind")))
        .and_then(Value::as_str);
    if matches!(reasoning_kind, Some("summary" | "status")) {
        return false;
    }
    if let Some(meta) = meta {
        if has_status_text(meta) {
            return false;
        }
        if let Some(codex_meta) = meta.get("codex-crp") {
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

pub(super) fn read_codex_context_window_metrics(
    codex_home: &Path,
    session_ref: &str,
) -> Option<serde_json::Value> {
    let path = find_codex_session_log(codex_home, session_ref)?;
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

fn find_codex_session_log(codex_home: &Path, session_ref: &str) -> Option<PathBuf> {
    let root = codex_home.join("sessions");
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
    matches!(provider_id, "claude-crp" | "codex-crp")
}

pub(super) fn runtime_provider_id_for_session_provider<'a>(
    session_provider_id: &'a str,
    _resolved_source: &ctx_harness_sources::ResolvedHarnessSource,
) -> &'a str {
    session_provider_id
}

pub(super) async fn apply_provider_launch_overrides(
    provider_id: &str,
    workdir: &std::path::Path,
    provider_env: &mut HashMap<String, String>,
) -> Result<()> {
    if provider_id == "openhands" {
        apply_openhands_launch_overrides(workdir, provider_env).await?;
        return Ok(());
    }

    if matches!(provider_id, "opencode" | "kimi") {
        // These providers currently behave truthfully only on their native ACP tool paths.
        provider_env.insert("CTX_MCP_DISABLED".to_string(), "1".to_string());
    }
    Ok(())
}

async fn apply_openhands_launch_overrides(
    workdir: &std::path::Path,
    provider_env: &mut HashMap<String, String>,
) -> Result<()> {
    let Some(data_root) = provider_env
        .get("CTX_DATA_ROOT")
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty())
    else {
        return Ok(());
    };
    let Some(session_id) = provider_env
        .get("CTX_SESSION_ID")
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty())
    else {
        return Ok(());
    };

    let alias =
        ensure_openhands_workdir_alias(std::path::Path::new(data_root), session_id, workdir)
            .await?;
    provider_env.insert(
        "OPENHANDS_WORK_DIR".to_string(),
        alias.to_string_lossy().to_string(),
    );
    Ok(())
}

async fn ensure_openhands_workdir_alias(
    data_root: &std::path::Path,
    session_id: &str,
    workdir: &std::path::Path,
) -> Result<std::path::PathBuf> {
    let alias_root = data_root
        .join("providers")
        .join("openhands")
        .join("workdir-aliases");
    tokio::fs::create_dir_all(&alias_root).await?;
    let alias = alias_root.join(session_id);
    reset_existing_alias(&alias, workdir).await?;
    match create_dir_symlink(workdir, &alias).await {
        Ok(()) => {}
        // Windows directory symlinks often require Developer Mode or elevated privileges.
        // Falling back to the real workdir keeps OpenHands usable when the short alias cannot
        // be created, even if the path is longer than ideal.
        Err(err) if should_fallback_to_direct_openhands_workdir(&err) => {
            return Ok(workdir.to_path_buf());
        }
        Err(err) => return Err(err),
    }
    Ok(alias)
}

fn should_fallback_to_direct_openhands_workdir(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .and_then(std::io::Error::raw_os_error)
            == Some(1314)
    })
}

async fn reset_existing_alias(alias: &std::path::Path, target: &std::path::Path) -> Result<()> {
    let metadata = match tokio::fs::symlink_metadata(alias).await {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err).with_context(|| format!("stat {}", alias.display())),
    };

    if metadata.file_type().is_symlink() {
        let existing_target = tokio::fs::read_link(alias)
            .await
            .with_context(|| format!("read link {}", alias.display()))?;
        if existing_target == target {
            return Ok(());
        }
        tokio::fs::remove_file(alias)
            .await
            .with_context(|| format!("remove stale symlink {}", alias.display()))?;
        return Ok(());
    }

    if metadata.is_dir() {
        tokio::fs::remove_dir_all(alias)
            .await
            .with_context(|| format!("remove stale directory {}", alias.display()))?;
    } else {
        tokio::fs::remove_file(alias)
            .await
            .with_context(|| format!("remove stale file {}", alias.display()))?;
    }
    Ok(())
}

async fn create_dir_symlink(source: &std::path::Path, target: &std::path::Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let source = source.to_path_buf();
        let target = target.to_path_buf();
        let source_for_link = source.clone();
        let target_for_link = target.clone();
        tokio::task::spawn_blocking(move || symlink(source_for_link, target_for_link))
            .await
            .map_err(|err| anyhow!("joining symlink task: {err}"))?
            .with_context(|| format!("symlink {} -> {}", target.display(), source.display()))?;
        return Ok(());
    }

    #[cfg(windows)]
    {
        use std::os::windows::fs::symlink_dir;
        let source = source.to_path_buf();
        let target = target.to_path_buf();
        let source_for_link = source.clone();
        let target_for_link = target.clone();
        tokio::task::spawn_blocking(move || symlink_dir(source_for_link, target_for_link))
            .await
            .map_err(|err| anyhow!("joining symlink task: {err}"))?
            .with_context(|| format!("symlink {} -> {}", target.display(), source.display()))?;
        return Ok(());
    }

    #[allow(unreachable_code)]
    Err(anyhow!(
        "directory symlinks are not supported on this platform"
    ))
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

        apply_provider_launch_overrides("codex-crp", temp.path(), &mut env)
            .await
            .expect("apply overrides");

        assert_eq!(env.get("CTX_MCP_DISABLED").map(String::as_str), Some("0"));
        assert!(!env.contains_key("ACP_CWD"));
    }

    #[tokio::test]
    async fn openhands_launch_overrides_set_short_workdir_alias() {
        let data_root = tempfile::tempdir().expect("data_root");
        let workdir_parent = tempfile::tempdir().expect("workdir_parent");
        let workdir = workdir_parent.path().join("nested").join("worktree");
        tokio::fs::create_dir_all(&workdir)
            .await
            .expect("create workdir");
        let mut env = HashMap::from([
            (
                "CTX_DATA_ROOT".to_string(),
                data_root.path().to_string_lossy().to_string(),
            ),
            ("CTX_SESSION_ID".to_string(), "session-123".to_string()),
        ]);

        apply_provider_launch_overrides("openhands", &workdir, &mut env)
            .await
            .expect("apply overrides");

        let alias = env
            .get("OPENHANDS_WORK_DIR")
            .map(std::path::PathBuf::from)
            .expect("OPENHANDS_WORK_DIR");
        assert_eq!(
            alias,
            data_root
                .path()
                .join("providers")
                .join("openhands")
                .join("workdir-aliases")
                .join("session-123")
        );
        let link_target = tokio::fs::read_link(&alias).await.expect("read alias");
        assert_eq!(link_target, workdir);
        assert!(!env.contains_key("CTX_MCP_DISABLED"));
    }

    #[test]
    fn openhands_workdir_fallback_detects_windows_symlink_privilege_errors() {
        let privilege_err = Err::<(), _>(std::io::Error::from_raw_os_error(1314))
            .with_context(|| "symlink failed")
            .expect_err("expected privilege error");
        let unrelated_err = Err::<(), _>(std::io::Error::from_raw_os_error(5))
            .with_context(|| "symlink failed")
            .expect_err("expected unrelated error");

        assert!(should_fallback_to_direct_openhands_workdir(&privilege_err));
        assert!(!should_fallback_to_direct_openhands_workdir(&unrelated_err));
    }
}
