use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use ctx_core::boolish::parse_boolish;
use ctx_core::provider_policy::{FULL_YOLO_APPROVAL_POLICY, FULL_YOLO_SANDBOX_MODE};
use serde_json::{json, Value};
use tokio::time::Duration;

use crate::adapters::TurnInput;
use crate::container_exec::{container_exec_spec, translate_thread_cwd_for_container};

use super::protocol::{CrpMcpServerConfig, CrpModelInfo, CrpModelsProbe, CrpSessionConfig};

const DEFAULT_CTX_MCP_TOOL_TIMEOUT_SECS: u64 = 2 * 60 * 60;

pub(super) fn model_override_disabled(env: &HashMap<String, String>) -> bool {
    env.get("CTX_CRP_DISABLE_MODEL_OVERRIDE")
        .and_then(|value| parse_boolish(value))
        .unwrap_or(false)
}

pub(super) fn build_crp_session_config(
    env: &HashMap<String, String>,
    workdir: &Path,
) -> Result<CrpSessionConfig> {
    let mcp_enabled = env
        .get("CTX_MCP_DISABLED")
        .and_then(|value| parse_boolish(value))
        .map(|disabled| !disabled)
        .unwrap_or(true);

    let (model, reasoning_effort) = if model_override_disabled(env) {
        (None, None)
    } else {
        env.get("CTX_MODEL_ID")
            .map(|value| split_model_id_and_effort(value))
            .unwrap_or((None, None))
    };

    let mcp_servers = if mcp_enabled {
        let mut mcp_env = HashMap::new();
        if let Some(url) = env.get("CTX_DAEMON_URL") {
            mcp_env.insert("CTX_DAEMON_URL".to_string(), url.clone());
        }
        if let Some(token) = env.get("CTX_AUTH_TOKEN") {
            mcp_env.insert("CTX_AUTH_TOKEN".to_string(), token.clone());
        }
        if let Some(session_id) = env.get("CTX_SESSION_ID") {
            mcp_env.insert("CTX_SESSION_ID".to_string(), session_id.clone());
        }
        if let Some(token) = env.get("CTX_MCP_TOKEN") {
            mcp_env.insert("CTX_MCP_TOKEN".to_string(), token.clone());
        }

        let mcp_command = resolve_session_mcp_command(env);
        let tool_timeout_sec = env
            .get("CTX_MCP_TOOL_TIMEOUT_SEC")
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_CTX_MCP_TOOL_TIMEOUT_SECS);

        let mut map = HashMap::new();
        map.insert(
            "ctx".to_string(),
            CrpMcpServerConfig {
                command: Some(mcp_command),
                args: Some(vec!["--stdio".to_string()]),
                env: Some(mcp_env),
                tool_timeout_sec: Some(tool_timeout_sec as f64),
            },
        );
        Some(map)
    } else {
        None
    };

    Ok(CrpSessionConfig {
        cwd: Some(translate_thread_cwd_for_container(env, workdir)?),
        spawn_cwd: Some(workdir.to_path_buf()),
        model,
        reasoning_effort,
        approval_policy: Some(FULL_YOLO_APPROVAL_POLICY.to_string()),
        sandbox_mode: Some(FULL_YOLO_SANDBOX_MODE.to_string()),
        model_provider: None,
        reasoning_trace_enabled: Some(true),
        personality: env
            .get("CTX_PROVIDER_ID")
            .map(|provider_id| provider_id.as_str())
            .filter(|provider_id| *provider_id == "codex")
            .map(|_| "pragmatic".to_string()),
        mcp_servers,
    })
}

fn resolve_session_mcp_command(env: &HashMap<String, String>) -> String {
    let configured = env
        .get("CTX_MCP_COMMAND")
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let Some(command) = configured else {
        return "ctx-mcp".to_string();
    };
    if container_exec_spec(env).is_none() || containerized_mcp_command_is_valid(command) {
        return command.to_string();
    }
    "ctx-mcp".to_string()
}

fn containerized_mcp_command_is_valid(command: &str) -> bool {
    let looks_like_path = command.contains('/') || command.contains('\\');
    if !looks_like_path {
        return true;
    }
    let path = Path::new(command);
    if !path.is_absolute() && !looks_like_windows_absolute_path(command) {
        return true;
    }
    path.exists()
}

fn looks_like_windows_absolute_path(command: &str) -> bool {
    let bytes = command.as_bytes();
    bytes.len() >= 3
        && bytes[1] == b':'
        && bytes[0].is_ascii_alphabetic()
        && (bytes[2] == b'\\' || bytes[2] == b'/')
}

pub(super) fn build_crp_model_probe_config(
    env: &HashMap<String, String>,
    workdir: &Path,
) -> Result<CrpSessionConfig> {
    let (model, reasoning_effort) = env
        .get("CTX_MODEL_ID")
        .map(|value| split_model_id_and_effort(value))
        .unwrap_or((None, None));
    Ok(CrpSessionConfig {
        cwd: Some(translate_thread_cwd_for_container(env, workdir)?),
        spawn_cwd: Some(workdir.to_path_buf()),
        model,
        reasoning_effort,
        approval_policy: Some(FULL_YOLO_APPROVAL_POLICY.to_string()),
        sandbox_mode: Some(FULL_YOLO_SANDBOX_MODE.to_string()),
        model_provider: None,
        reasoning_trace_enabled: None,
        personality: None,
        mcp_servers: None,
    })
}

pub(super) fn split_model_id_and_effort(model_id: &str) -> (Option<String>, Option<String>) {
    let trimmed = model_id.trim();
    if trimmed.is_empty() {
        return (None, None);
    }
    let Some((base, suffix)) = trimmed.rsplit_once('/') else {
        return (Some(trimmed.to_string()), None);
    };
    if base.trim().is_empty() {
        return (Some(trimmed.to_string()), None);
    }
    let Some(effort) = normalize_effort_id(suffix) else {
        return (Some(trimmed.to_string()), None);
    };
    (Some(base.trim().to_string()), Some(effort))
}

fn normalize_effort_id(raw: &str) -> Option<String> {
    let normalized = raw.trim().to_lowercase();
    match normalized.as_str() {
        "none" | "minimal" | "low" | "medium" | "high" | "xhigh" => Some(normalized),
        "extra_high" | "extra-high" | "extra high" => Some("xhigh".to_string()),
        _ => None,
    }
}

pub(super) fn synthetic_models_probe_for_provider(
    provider_id: &str,
    env: &HashMap<String, String>,
) -> Option<CrpModelsProbe> {
    if provider_id != "cline" {
        return None;
    }
    let model_id = env
        .get("OPENAI_MODEL")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())?;
    Some(CrpModelsProbe {
        models: vec![CrpModelInfo {
            id: model_id.clone(),
            name: Some(model_id.clone()),
        }],
        current_model_id: Some(model_id),
        catalog_source: None,
    })
}

pub(super) fn probe_timeout_for_env(
    env: &HashMap<String, String>,
    host_timeout: Duration,
    container_timeout: Duration,
) -> Duration {
    if container_exec_spec(env).is_some() {
        container_timeout
    } else {
        host_timeout
    }
}

// NOTE: The canonical CRP image transport is now blob-backed refs and inline bytes, not
// provider-visible files. Explicit `local_image` items remain compatibility-only for runtimes
// that deliberately opt into filesystem paths.

pub(super) async fn build_prompt_items(
    input: &TurnInput,
    _workdir: &PathBuf,
    _env: &HashMap<String, String>,
) -> Result<Vec<Value>> {
    let mut items = Vec::new();
    for block in &input.context_blocks {
        if block
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(|t| matches!(t, "text" | "image" | "image_ref" | "local_image" | "skill"))
        {
            items.push(block.clone());
        }
    }

    for att in input.attachments.iter() {
        match att {
            ctx_core::models::MessageAttachment::Image {
                mime_type,
                data_base64,
                name,
            } => {
                items.push(json!({
                    "type": "image",
                    "mime_type": mime_type,
                    "data": data_base64,
                    "name": name,
                }));
            }
            ctx_core::models::MessageAttachment::ImageRef {
                blob_id,
                mime_type,
                name,
            } => {
                items.push(json!({
                    "type": "image_ref",
                    "blob_id": blob_id,
                    "mime_type": mime_type,
                    "name": name,
                }));
            }
        }
    }

    items.push(json!({"type":"text","text": input.content}));
    Ok(items)
}

pub(super) fn provider_requires_flattened_text_prompt(provider_id: &str) -> bool {
    provider_id.eq_ignore_ascii_case("opencode")
}

pub(super) fn flatten_prompt_items_as_text(items: &[Value]) -> Result<String> {
    let mut out = Vec::new();
    for item in items {
        let text = match item {
            Value::String(text) => Some(text.as_str()),
            Value::Object(obj) => obj
                .get("text")
                .and_then(Value::as_str)
                .or_else(|| obj.get("content").and_then(Value::as_str)),
            _ => None,
        };
        let Some(text) = text else {
            anyhow::bail!("provider requires text-only ACP prompt items");
        };
        if !text.is_empty() {
            out.push(text.to_string());
        }
    }
    if out.is_empty() {
        anyhow::bail!("provider requires a non-empty text ACP prompt");
    }
    Ok(out.join("\n\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;
    use std::fs;

    #[test]
    fn probe_timeout_for_env_defaults_to_host_timeout() {
        let env = HashMap::<String, String>::new();
        assert_eq!(
            probe_timeout_for_env(&env, Duration::from_secs(10), Duration::from_secs(45)),
            Duration::from_secs(10)
        );
    }

    #[test]
    fn probe_timeout_for_env_uses_container_timeout_when_container_exec_is_present() {
        let mut env = HashMap::<String, String>::new();
        env.insert(
            "CTX_HARNESS_CONTAINER_ID".to_string(),
            "ctx-workspace-123".to_string(),
        );
        assert_eq!(
            probe_timeout_for_env(&env, Duration::from_secs(10), Duration::from_secs(45)),
            Duration::from_secs(45)
        );
    }

    #[test]
    fn build_crp_session_config_sets_pragmatic_personality_for_codex() {
        let mut env = HashMap::new();
        env.insert("CTX_PROVIDER_ID".to_string(), "codex".to_string());
        let workdir = PathBuf::from("/tmp/workdir");

        let cfg = build_crp_session_config(&env, &workdir).expect("build session config");
        assert_eq!(
            cfg.approval_policy.as_deref(),
            Some(FULL_YOLO_APPROVAL_POLICY)
        );
        assert_eq!(cfg.sandbox_mode.as_deref(), Some(FULL_YOLO_SANDBOX_MODE));
        assert_eq!(cfg.reasoning_trace_enabled, Some(true));
        assert_eq!(cfg.personality.as_deref(), Some("pragmatic"));
    }

    #[test]
    fn build_crp_session_config_omits_personality_for_non_codex() {
        let mut env = HashMap::new();
        env.insert("CTX_PROVIDER_ID".to_string(), "claude-crp".to_string());
        let workdir = PathBuf::from("/tmp/workdir");

        let cfg = build_crp_session_config(&env, &workdir).expect("build session config");
        assert_eq!(
            cfg.approval_policy.as_deref(),
            Some(FULL_YOLO_APPROVAL_POLICY)
        );
        assert_eq!(cfg.sandbox_mode.as_deref(), Some(FULL_YOLO_SANDBOX_MODE));
        assert_eq!(cfg.personality, None);
    }

    #[test]
    fn build_crp_session_config_can_disable_model_override() {
        let mut env = HashMap::new();
        env.insert(
            "CTX_MODEL_ID".to_string(),
            "openai/gpt-4.1-mini".to_string(),
        );
        env.insert(
            "CTX_CRP_DISABLE_MODEL_OVERRIDE".to_string(),
            "1".to_string(),
        );
        let workdir = PathBuf::from("/tmp/workdir");

        let cfg = build_crp_session_config(&env, &workdir).expect("build session config");
        assert_eq!(cfg.model, None);
        assert_eq!(cfg.reasoning_effort, None);
    }

    #[test]
    fn build_crp_session_config_uses_default_ctx_mcp_for_container_when_override_is_host_only() {
        let mut env = HashMap::new();
        env.insert(
            "CTX_HARNESS_CONTAINER_ID".to_string(),
            "ctx-harness-123".to_string(),
        );
        env.insert(
            "CTX_MCP_COMMAND".to_string(),
            "/Users/example-user/.cache/cargo/ctx-monorepo/debug/ctx-mcp".to_string(),
        );

        let cfg =
            build_crp_session_config(&env, Path::new("/ctx/ws")).expect("build session config");
        let command = cfg
            .mcp_servers
            .as_ref()
            .and_then(|servers| servers.get("ctx"))
            .and_then(|server| server.command.as_deref());
        assert_eq!(command, Some("ctx-mcp"));
    }

    #[test]
    fn build_crp_session_config_preserves_existing_absolute_ctx_mcp_for_container() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let mcp_path = tempdir.path().join("ctx-mcp");
        fs::write(&mcp_path, b"#!/bin/sh\n").expect("write mcp");

        let mut env = HashMap::new();
        env.insert(
            "CTX_HARNESS_CONTAINER_ID".to_string(),
            "ctx-harness-123".to_string(),
        );
        env.insert(
            "CTX_MCP_COMMAND".to_string(),
            mcp_path.to_string_lossy().to_string(),
        );

        let cfg =
            build_crp_session_config(&env, Path::new("/ctx/ws")).expect("build session config");
        let command = cfg
            .mcp_servers
            .as_ref()
            .and_then(|servers| servers.get("ctx"))
            .and_then(|server| server.command.as_deref());
        assert_eq!(command, Some(mcp_path.to_string_lossy().as_ref()));
    }

    #[test]
    fn build_crp_model_probe_config_forces_full_yolo_policy() {
        let workdir = PathBuf::from("/tmp/workdir");

        let cfg = build_crp_model_probe_config(&HashMap::new(), &workdir)
            .expect("build model probe config");
        assert_eq!(
            cfg.approval_policy.as_deref(),
            Some(FULL_YOLO_APPROVAL_POLICY)
        );
        assert_eq!(cfg.sandbox_mode.as_deref(), Some(FULL_YOLO_SANDBOX_MODE));
    }

    #[test]
    fn build_crp_session_config_maps_container_thread_cwd_to_guest_worktree() {
        let mut env = HashMap::new();
        env.insert(
            "CTX_HARNESS_CONTAINER_ID".to_string(),
            "ctx-harness-123".to_string(),
        );
        env.insert(
            "CTX_HARNESS_HOST_WORKTREE_ROOT".to_string(),
            "/Users/example-user/code/repo".to_string(),
        );
        env.insert(
            "CTX_HARNESS_GUEST_WORKTREE_ROOT".to_string(),
            "/ctx/ws/worktrees/wt-123".to_string(),
        );
        env.insert(
            "CTX_HARNESS_GUEST_WORKSPACE_ROOT".to_string(),
            "/ctx/ws".to_string(),
        );
        let workdir = PathBuf::from("/Users/example-user/code/repo/src");

        let cfg = build_crp_session_config(&env, &workdir).expect("build session config");
        assert_eq!(cfg.cwd, Some(PathBuf::from("/ctx/ws/worktrees/wt-123/src")));
        assert_eq!(cfg.spawn_cwd, Some(workdir));
    }

    #[test]
    fn synthetic_cline_models_probe_uses_openai_model() {
        let mut env = HashMap::new();
        env.insert(
            "OPENAI_MODEL".to_string(),
            "openai/gpt-5.2-codex".to_string(),
        );
        let probe =
            synthetic_models_probe_for_provider("cline", &env).expect("cline synthetic probe");
        assert_eq!(
            probe.current_model_id.as_deref(),
            Some("openai/gpt-5.2-codex")
        );
        assert_eq!(probe.models.len(), 1);
        assert_eq!(probe.models[0].id, "openai/gpt-5.2-codex");
    }

    #[test]
    fn synthetic_models_probe_is_provider_scoped() {
        let env = HashMap::new();
        assert!(synthetic_models_probe_for_provider("qwen", &env).is_none());
    }

    #[test]
    fn synthetic_cline_models_probe_requires_openai_model() {
        let env = HashMap::new();
        assert!(synthetic_models_probe_for_provider("cline", &env).is_none());
    }

    #[test]
    fn flatten_prompt_items_as_text_joins_text_blocks_in_order() {
        let items = vec![
            json!({"type":"text","text":"system"}),
            json!({"type":"text","text":"user"}),
        ];
        assert_eq!(
            flatten_prompt_items_as_text(&items).expect("flattened prompt"),
            "system\n\nuser"
        );
    }

    #[test]
    fn flatten_prompt_items_as_text_rejects_non_text_items() {
        let items = vec![json!({"type":"image","image_url":"data:image/png;base64,AAAA"})];
        let err = flatten_prompt_items_as_text(&items).expect_err("image item should fail");
        assert!(err
            .to_string()
            .contains("provider requires text-only ACP prompt items"));
    }

    #[tokio::test]
    async fn build_prompt_items_emits_inline_images_as_bytes_items() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut env = HashMap::new();
        env.insert(
            "CTX_DATA_ROOT".to_string(),
            temp.path().to_string_lossy().to_string(),
        );
        let bytes = vec![1u8, 2, 3, 4];
        let input = TurnInput {
            content: "describe image".to_string(),
            context_blocks: Vec::new(),
            attachments: vec![ctx_core::models::MessageAttachment::Image {
                mime_type: "image/png".to_string(),
                data_base64: base64::engine::general_purpose::STANDARD.encode(&bytes),
                name: Some("inline.png".to_string()),
            }],
            model_id: None,
        };

        let items = build_prompt_items(&input, &PathBuf::from("."), &env)
            .await
            .expect("inline image should be emitted as CRP image item");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].get("type").and_then(Value::as_str), Some("image"));
        assert_eq!(
            items[0].get("mime_type").and_then(Value::as_str),
            Some("image/png")
        );
        assert_eq!(
            items[0].get("data").and_then(Value::as_str),
            Some(
                base64::engine::general_purpose::STANDARD
                    .encode(&bytes)
                    .as_str()
            )
        );
    }

    #[tokio::test]
    async fn build_prompt_items_emits_blob_refs_as_image_refs() {
        let host_root = tempfile::tempdir().expect("host tempdir");
        let runtime_root = tempfile::tempdir().expect("runtime tempdir");
        let blob_dir = host_root.path().join("blobs");
        tokio::fs::create_dir_all(&blob_dir)
            .await
            .expect("create blob dir");
        let blob_id = "blob-123";
        let blob_path = blob_dir.join(blob_id);
        let bytes = vec![9u8, 8, 7, 6];
        tokio::fs::write(&blob_path, &bytes)
            .await
            .expect("write blob");

        let mut env = HashMap::new();
        env.insert(
            "CTX_DATA_ROOT_HOST".to_string(),
            host_root.path().to_string_lossy().to_string(),
        );
        env.insert(
            "CTX_DATA_ROOT".to_string(),
            runtime_root.path().to_string_lossy().to_string(),
        );

        let input = TurnInput {
            content: "describe image".to_string(),
            context_blocks: Vec::new(),
            attachments: vec![ctx_core::models::MessageAttachment::ImageRef {
                blob_id: blob_id.to_string(),
                mime_type: "image/png".to_string(),
                name: Some("blob.png".to_string()),
            }],
            model_id: None,
        };

        let items = build_prompt_items(&input, &PathBuf::from("."), &env)
            .await
            .expect("blob image should be emitted as image_ref");
        assert_eq!(items.len(), 2);
        assert_eq!(
            items[0].get("type").and_then(Value::as_str),
            Some("image_ref")
        );
        assert_eq!(
            items[0].get("blob_id").and_then(Value::as_str),
            Some(blob_id)
        );
        assert_eq!(
            items[0].get("mime_type").and_then(Value::as_str),
            Some("image/png")
        );
    }
}
