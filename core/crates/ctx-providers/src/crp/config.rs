use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use base64::Engine;
use ctx_core::boolish::parse_boolish;
use serde_json::{json, Value};
use tokio::time::Duration;

use crate::adapters::TurnInput;
use crate::container_exec::container_exec_spec;

use super::protocol::{CrpMcpServerConfig, CrpModelInfo, CrpModelsProbe, CrpSessionConfig};

const DEFAULT_CTX_MCP_TOOL_TIMEOUT_SECS: u64 = 2 * 60 * 60;

pub(super) fn build_crp_session_config(
    env: &HashMap<String, String>,
    workdir: &Path,
) -> CrpSessionConfig {
    let mcp_enabled = env
        .get("CTX_MCP_DISABLED")
        .and_then(|value| parse_boolish(value))
        .map(|disabled| !disabled)
        .unwrap_or(true);

    let (model, reasoning_effort) = env
        .get("CTX_MODEL_ID")
        .map(|value| split_model_id_and_effort(value))
        .unwrap_or((None, None));

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

        let mcp_command = env
            .get("CTX_MCP_COMMAND")
            .cloned()
            .unwrap_or_else(|| "ctx-mcp".to_string());
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

    CrpSessionConfig {
        cwd: Some(workdir.to_path_buf()),
        model,
        reasoning_effort,
        model_provider: None,
        reasoning_trace_enabled: Some(true),
        personality: env
            .get("CTX_PROVIDER_ID")
            .map(|provider_id| provider_id.as_str())
            .filter(|provider_id| *provider_id == "codex")
            .map(|_| "pragmatic".to_string()),
        mcp_servers,
    }
}

pub(super) fn build_crp_model_probe_config(
    env: &HashMap<String, String>,
    workdir: &Path,
) -> CrpSessionConfig {
    let (model, reasoning_effort) = env
        .get("CTX_MODEL_ID")
        .map(|value| split_model_id_and_effort(value))
        .unwrap_or((None, None));
    CrpSessionConfig {
        cwd: Some(workdir.to_path_buf()),
        model,
        reasoning_effort,
        model_provider: None,
        reasoning_trace_enabled: None,
        personality: None,
        mcp_servers: None,
    }
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

pub(super) async fn build_prompt_items(
    input: &TurnInput,
    _workdir: &PathBuf,
    env: &HashMap<String, String>,
) -> Result<Vec<Value>> {
    let mut items = Vec::new();
    for block in &input.context_blocks {
        if block
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(|t| matches!(t, "text" | "image" | "local_image" | "skill"))
        {
            items.push(block.clone());
        }
    }

    let data_root = crate::env::data_root_for_host(env);
    for att in input.attachments.iter() {
        match att {
            ctx_core::models::MessageAttachment::Image {
                mime_type,
                data_base64,
                ..
            } => {
                items.push(json!({
                    "type": "image",
                    "image_url": format!("data:{mime_type};base64,{data_base64}"),
                }));
            }
            ctx_core::models::MessageAttachment::ImageRef {
                blob_id, mime_type, ..
            } => {
                let Some(data_root) = data_root.as_deref() else {
                    anyhow::bail!("missing CTX_DATA_ROOT_HOST/CTX_DATA_ROOT for image attachment");
                };
                let path = std::path::Path::new(data_root).join("blobs").join(blob_id);
                let bytes = tokio::fs::read(&path)
                    .await
                    .with_context(|| format!("reading image blob {blob_id}"))?;
                let data_base64 = base64::engine::general_purpose::STANDARD.encode(bytes);
                items.push(json!({
                    "type": "image",
                    "image_url": format!("data:{mime_type};base64,{data_base64}"),
                }));
            }
        }
    }

    items.push(json!({"type":"text","text": input.content}));
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;

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

        let cfg = build_crp_session_config(&env, &workdir);
        assert_eq!(cfg.reasoning_trace_enabled, Some(true));
        assert_eq!(cfg.personality.as_deref(), Some("pragmatic"));
    }

    #[test]
    fn build_crp_session_config_omits_personality_for_non_codex() {
        let mut env = HashMap::new();
        env.insert("CTX_PROVIDER_ID".to_string(), "claude-crp".to_string());
        let workdir = PathBuf::from("/tmp/workdir");

        let cfg = build_crp_session_config(&env, &workdir);
        assert_eq!(cfg.personality, None);
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
}
