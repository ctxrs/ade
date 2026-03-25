use crate::app_server::ModelInfo;
use crate::protocol::{CrpCommandInfo, CrpModelInfo, CrpSessionConfig};
use anyhow::{anyhow, Result};
use directories::BaseDirs;
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

const PROMPTS_CMD_PREFIX: &str = "prompts";

struct SlashCommandDef {
    name: &'static str,
    description: &'static str,
    argument_hint: Option<&'static str>,
    visible: fn() -> bool,
}

fn visible_always() -> bool {
    true
}

fn visible_copy() -> bool {
    !cfg!(target_os = "android")
}

fn visible_windows_only() -> bool {
    cfg!(target_os = "windows")
}

fn visible_debug_only() -> bool {
    cfg!(debug_assertions)
}

const BUILTIN_COMMANDS: &[SlashCommandDef] = &[
    SlashCommandDef {
        name: "model",
        description: "choose what model and reasoning effort to use",
        argument_hint: None,
        visible: visible_always,
    },
    SlashCommandDef {
        name: "fast",
        description: "toggle Fast mode to enable fastest inference at 2X plan usage",
        argument_hint: None,
        visible: visible_always,
    },
    SlashCommandDef {
        name: "approvals",
        description: "choose what Codex is allowed to do",
        argument_hint: None,
        visible: visible_always,
    },
    SlashCommandDef {
        name: "permissions",
        description: "choose what Codex is allowed to do",
        argument_hint: None,
        visible: visible_always,
    },
    SlashCommandDef {
        name: "setup-default-sandbox",
        description: "set up elevated agent sandbox",
        argument_hint: None,
        visible: visible_always,
    },
    SlashCommandDef {
        name: "sandbox-add-read-dir",
        description: "let sandbox read a directory: /sandbox-add-read-dir <absolute_path>",
        argument_hint: Some("<absolute_path>"),
        visible: visible_windows_only,
    },
    SlashCommandDef {
        name: "experimental",
        description: "toggle experimental features",
        argument_hint: None,
        visible: visible_always,
    },
    SlashCommandDef {
        name: "skills",
        description: "use skills to improve how Codex performs specific tasks",
        argument_hint: None,
        visible: visible_always,
    },
    SlashCommandDef {
        name: "review",
        description: "review my current changes and find issues",
        argument_hint: Some("<instructions>"),
        visible: visible_always,
    },
    SlashCommandDef {
        name: "rename",
        description: "rename the current thread",
        argument_hint: Some("<title>"),
        visible: visible_always,
    },
    SlashCommandDef {
        name: "new",
        description: "start a new chat during a conversation",
        argument_hint: None,
        visible: visible_always,
    },
    SlashCommandDef {
        name: "resume",
        description: "resume a saved chat",
        argument_hint: None,
        visible: visible_always,
    },
    SlashCommandDef {
        name: "fork",
        description: "fork the current chat",
        argument_hint: None,
        visible: visible_always,
    },
    SlashCommandDef {
        name: "init",
        description: "create an AGENTS.md file with instructions for Codex",
        argument_hint: None,
        visible: visible_always,
    },
    SlashCommandDef {
        name: "compact",
        description: "summarize conversation to prevent hitting the context limit",
        argument_hint: None,
        visible: visible_always,
    },
    SlashCommandDef {
        name: "plan",
        description: "switch to Plan mode",
        argument_hint: Some("<prompt>"),
        visible: visible_always,
    },
    SlashCommandDef {
        name: "collab",
        description: "change collaboration mode (experimental)",
        argument_hint: None,
        visible: visible_always,
    },
    SlashCommandDef {
        name: "agent",
        description: "switch the active agent thread",
        argument_hint: None,
        visible: visible_always,
    },
    SlashCommandDef {
        name: "diff",
        description: "show git diff (including untracked files)",
        argument_hint: None,
        visible: visible_always,
    },
    SlashCommandDef {
        name: "copy",
        description: "copy the latest Codex output to your clipboard",
        argument_hint: None,
        visible: visible_copy,
    },
    SlashCommandDef {
        name: "mention",
        description: "mention a file",
        argument_hint: None,
        visible: visible_always,
    },
    SlashCommandDef {
        name: "status",
        description: "show current session configuration and token usage",
        argument_hint: None,
        visible: visible_always,
    },
    SlashCommandDef {
        name: "debug-config",
        description: "show config layers and requirement sources for debugging",
        argument_hint: None,
        visible: visible_always,
    },
    SlashCommandDef {
        name: "statusline",
        description: "configure which items appear in the status line",
        argument_hint: None,
        visible: visible_always,
    },
    SlashCommandDef {
        name: "theme",
        description: "choose a syntax highlighting theme",
        argument_hint: None,
        visible: visible_always,
    },
    SlashCommandDef {
        name: "mcp",
        description: "list configured MCP tools",
        argument_hint: None,
        visible: visible_always,
    },
    SlashCommandDef {
        name: "apps",
        description: "manage apps",
        argument_hint: None,
        visible: visible_always,
    },
    SlashCommandDef {
        name: "logout",
        description: "log out of Codex",
        argument_hint: None,
        visible: visible_always,
    },
    SlashCommandDef {
        name: "quit",
        description: "exit Codex",
        argument_hint: None,
        visible: visible_always,
    },
    SlashCommandDef {
        name: "exit",
        description: "exit Codex",
        argument_hint: None,
        visible: visible_always,
    },
    SlashCommandDef {
        name: "feedback",
        description: "send logs to maintainers",
        argument_hint: None,
        visible: visible_always,
    },
    SlashCommandDef {
        name: "rollout",
        description: "print the rollout file path",
        argument_hint: None,
        visible: visible_debug_only,
    },
    SlashCommandDef {
        name: "ps",
        description: "list background terminals",
        argument_hint: None,
        visible: visible_always,
    },
    SlashCommandDef {
        name: "clean",
        description: "stop all background terminals",
        argument_hint: None,
        visible: visible_always,
    },
    SlashCommandDef {
        name: "clear",
        description: "clear the terminal and start a new chat",
        argument_hint: None,
        visible: visible_always,
    },
    SlashCommandDef {
        name: "personality",
        description: "choose a communication style for Codex",
        argument_hint: None,
        visible: visible_always,
    },
    SlashCommandDef {
        name: "realtime",
        description: "toggle realtime voice mode (experimental)",
        argument_hint: None,
        visible: visible_always,
    },
    SlashCommandDef {
        name: "settings",
        description: "configure realtime microphone/speaker",
        argument_hint: None,
        visible: visible_always,
    },
    SlashCommandDef {
        name: "test-approval",
        description: "test approval request",
        argument_hint: None,
        visible: visible_debug_only,
    },
    SlashCommandDef {
        name: "multi-agents",
        description: "switch the active agent thread",
        argument_hint: None,
        visible: visible_always,
    },
    SlashCommandDef {
        name: "debug-m-drop",
        description: "DO NOT USE",
        argument_hint: None,
        visible: visible_always,
    },
    SlashCommandDef {
        name: "debug-m-update",
        description: "DO NOT USE",
        argument_hint: None,
        visible: visible_always,
    },
];

#[derive(Debug, Clone)]
struct CustomPrompt {
    name: String,
    description: Option<String>,
    argument_hint: Option<String>,
}

pub fn build_session_command_infos(codex_home: &Path) -> Vec<CrpCommandInfo> {
    let mut commands = build_builtin_command_infos();
    let mut exclude = commands
        .iter()
        .map(|command| command.name.clone())
        .collect::<HashSet<_>>();
    let prompts_dir = codex_home.join("prompts");
    let prompts = discover_prompts_in_excluding(&prompts_dir, &exclude);
    commands.extend(build_prompt_command_infos(prompts, &mut exclude));
    commands
}

pub fn command_names(commands: &[CrpCommandInfo]) -> Vec<String> {
    commands
        .iter()
        .map(|command| command.name.clone())
        .collect()
}

pub fn resolve_codex_home() -> PathBuf {
    if let Some(home) = std::env::var("CODEX_HOME")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        return PathBuf::from(home);
    }
    BaseDirs::new()
        .map(|dirs| dirs.home_dir().join(".codex"))
        .unwrap_or_else(|| PathBuf::from(".codex"))
}

pub fn split_model_and_effort(model: &str) -> (String, Option<String>) {
    let Some((base, effort)) = model.rsplit_once('/') else {
        return (model.to_string(), None);
    };
    let normalized = normalize_effort_id(effort);
    if normalized.is_some() && !base.trim().is_empty() {
        (base.trim().to_string(), normalized)
    } else {
        (model.to_string(), None)
    }
}

pub fn normalize_ctx_system_prompt_append(raw: Option<String>) -> Option<String> {
    raw.map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub fn build_app_server_config_overrides(config: &CrpSessionConfig) -> Option<Value> {
    let mut out = Map::new();
    if let Some(enabled) = config.reasoning_trace_enabled {
        out.insert("show_raw_agent_reasoning".to_string(), Value::Bool(enabled));
    }
    if let Some(mcp_servers) = &config.mcp_servers {
        for (name, server) in mcp_servers {
            if let Some(value) = mcp_server_to_value(server.clone()) {
                out.insert(format!("mcp_servers.{name}"), value);
            }
        }
    }
    (!out.is_empty()).then_some(Value::Object(out))
}

pub fn parse_cli_config_overrides(raw_entries: &[String]) -> Result<Option<Value>> {
    let mut out = Map::new();
    for raw_entry in raw_entries {
        let trimmed = raw_entry.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Some((key, raw_value)) = trimmed.split_once('=') else {
            return Err(anyhow!(
                "invalid -c/--config override `{trimmed}`; expected key=value"
            ));
        };
        let key = key.trim();
        if key.is_empty() {
            return Err(anyhow!(
                "invalid -c/--config override `{trimmed}`; key must not be empty"
            ));
        }
        out.insert(key.to_string(), parse_override_value(raw_value.trim()));
    }
    Ok((!out.is_empty()).then_some(Value::Object(out)))
}

pub fn merge_config_overrides(base: Option<Value>, overlay: Option<Value>) -> Option<Value> {
    match (base, overlay) {
        (None, None) => None,
        (Some(value), None) | (None, Some(value)) => Some(value),
        (Some(Value::Object(mut base)), Some(Value::Object(overlay))) => {
            for (key, value) in overlay {
                base.insert(key, value);
            }
            Some(Value::Object(base))
        }
        (_, Some(overlay)) => Some(overlay),
    }
}

pub fn build_model_infos(models: &[ModelInfo]) -> Vec<CrpModelInfo> {
    let mut out = Vec::new();
    for model in models {
        if model.hidden {
            continue;
        }
        if model.supported_reasoning_efforts.len() >= 2 {
            let mut seen = HashSet::new();
            for effort in &model.supported_reasoning_efforts {
                if !seen.insert(effort.reasoning_effort.clone()) {
                    continue;
                }
                let id = format!("{}/{}", model.id, effort.reasoning_effort);
                let name = format!("{} ({})", model.display_name, effort.reasoning_effort);
                out.push(CrpModelInfo {
                    id,
                    name: Some(name),
                });
            }
        } else {
            out.push(CrpModelInfo {
                id: model.id.clone(),
                name: Some(model.display_name.clone()),
            });
        }
    }
    out
}

pub fn build_current_model_id(
    config: Option<&CrpSessionConfig>,
    models: &[ModelInfo],
    current_model: Option<&str>,
    current_effort: Option<&str>,
) -> Option<String> {
    if let Some(model) = current_model {
        let model = model.trim();
        if model.is_empty() {
            return None;
        }
        if let Some(effort) = current_effort.filter(|value| !value.trim().is_empty()) {
            return Some(format!("{model}/{}", effort.trim()));
        }
        return Some(model.to_string());
    }

    if let Some(config) = config {
        if let Some(model) = config.model.as_deref() {
            let (model, effort) = split_model_and_effort(model);
            if let Some(effort) =
                effort.or_else(|| normalize_optional_effort(config.reasoning_effort.as_deref()))
            {
                return Some(format!("{model}/{effort}"));
            }
            return Some(model);
        }
    }

    let model = models.iter().find(|candidate| candidate.is_default)?;
    if model.supported_reasoning_efforts.len() >= 2 {
        return Some(format!("{}/{}", model.id, model.default_reasoning_effort));
    }
    Some(model.id.clone())
}

fn build_builtin_command_infos() -> Vec<CrpCommandInfo> {
    BUILTIN_COMMANDS
        .iter()
        .filter(|command| (command.visible)())
        .map(|command| CrpCommandInfo {
            name: command.name.to_string(),
            description: Some(command.description.to_string()),
            argument_hint: command.argument_hint.map(str::to_string),
        })
        .collect()
}

fn build_prompt_command_infos(
    prompts: Vec<CustomPrompt>,
    exclude: &mut HashSet<String>,
) -> Vec<CrpCommandInfo> {
    let mut commands = Vec::new();
    for prompt in prompts {
        let name = format!("{PROMPTS_CMD_PREFIX}:{}", prompt.name);
        if !exclude.insert(name.clone()) {
            continue;
        }
        commands.push(CrpCommandInfo {
            name,
            description: prompt
                .description
                .or_else(|| Some("send saved prompt".to_string())),
            argument_hint: prompt.argument_hint,
        });
    }
    commands
}

fn discover_prompts_in_excluding(dir: &Path, exclude: &HashSet<String>) -> Vec<CustomPrompt> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let is_md = path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("md"));
        if !is_md {
            continue;
        }
        let Some(name) = path
            .file_stem()
            .and_then(|value| value.to_str())
            .map(str::to_string)
        else {
            continue;
        };
        if exclude.contains(&name) {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let (description, argument_hint, _body) = parse_frontmatter(&content);
        out.push(CustomPrompt {
            name,
            description,
            argument_hint,
        });
    }
    out.sort_by(|left, right| left.name.cmp(&right.name));
    out
}

fn parse_frontmatter(content: &str) -> (Option<String>, Option<String>, String) {
    let mut segments = content.split_inclusive('\n');
    let Some(first_segment) = segments.next() else {
        return (None, None, String::new());
    };
    let first_line = first_segment.trim_end_matches(['\r', '\n']);
    if first_line.trim() != "---" {
        return (None, None, content.to_string());
    }

    let mut description = None;
    let mut argument_hint = None;
    let mut consumed = first_segment.len();
    let mut frontmatter_closed = false;

    for segment in segments {
        let line = segment.trim_end_matches(['\r', '\n']);
        let trimmed = line.trim();
        if trimmed == "---" {
            consumed += segment.len();
            frontmatter_closed = true;
            break;
        }
        if trimmed.is_empty() || trimmed.starts_with('#') {
            consumed += segment.len();
            continue;
        }
        if let Some((key, value)) = trimmed.split_once(':') {
            let mut value = value.trim().to_string();
            if value.len() >= 2 {
                let first = value.as_bytes()[0];
                let last = value.as_bytes()[value.len() - 1];
                if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
                    value = value[1..value.len().saturating_sub(1)].to_string();
                }
            }
            match key.trim().to_ascii_lowercase().as_str() {
                "description" => description = Some(value),
                "argument-hint" | "argument_hint" => argument_hint = Some(value),
                _ => {}
            }
        }
        consumed += segment.len();
    }

    if !frontmatter_closed {
        return (None, None, content.to_string());
    }

    let body = if consumed >= content.len() {
        String::new()
    } else {
        content[consumed..].to_string()
    };

    (description, argument_hint, body)
}

fn mcp_server_to_value(config: crate::protocol::CrpMcpServerConfig) -> Option<Value> {
    let mut out = Map::new();
    if let Some(timeout) = config.tool_timeout_sec {
        out.insert("tool_timeout_sec".to_string(), Value::from(timeout));
    }
    if let Some(enabled_tools) = config.enabled_tools {
        out.insert(
            "enabled_tools".to_string(),
            Value::Array(enabled_tools.into_iter().map(Value::String).collect()),
        );
    }
    if let Some(disabled_tools) = config.disabled_tools {
        out.insert(
            "disabled_tools".to_string(),
            Value::Array(disabled_tools.into_iter().map(Value::String).collect()),
        );
    }
    if let Some(command) = config.command {
        out.insert("command".to_string(), Value::String(command));
        if let Some(args) = config.args.filter(|args| !args.is_empty()) {
            out.insert(
                "args".to_string(),
                Value::Array(args.into_iter().map(Value::String).collect()),
            );
        }
        if let Some(env) = config.env.filter(|env| !env.is_empty()) {
            out.insert("env".to_string(), map_to_value(env));
        }
        if let Some(env_vars) = config.env_vars.filter(|vars| !vars.is_empty()) {
            out.insert(
                "env_vars".to_string(),
                Value::Array(env_vars.into_iter().map(Value::String).collect()),
            );
        }
        if let Some(cwd) = config.cwd {
            out.insert(
                "cwd".to_string(),
                Value::String(cwd.to_string_lossy().to_string()),
            );
        }
        return Some(Value::Object(out));
    }
    if let Some(url) = config.url {
        out.insert("url".to_string(), Value::String(url));
        if let Some(headers) = config.http_headers.filter(|headers| !headers.is_empty()) {
            out.insert("http_headers".to_string(), map_to_value(headers));
        }
        if let Some(headers) = config
            .env_http_headers
            .filter(|headers| !headers.is_empty())
        {
            out.insert("env_http_headers".to_string(), map_to_value(headers));
        }
        return Some(Value::Object(out));
    }
    None
}

fn map_to_value(input: HashMap<String, String>) -> Value {
    let mut out = Map::new();
    for (key, value) in input {
        out.insert(key, Value::String(value));
    }
    Value::Object(out)
}

fn normalize_effort_id(raw: &str) -> Option<String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "none" | "minimal" | "low" | "medium" | "high" | "xhigh" => {
            Some(raw.trim().to_ascii_lowercase())
        }
        _ => None,
    }
}

fn normalize_optional_effort(raw: Option<&str>) -> Option<String> {
    raw.and_then(normalize_effort_id)
}

fn parse_override_value(raw: &str) -> Value {
    if raw.is_empty() {
        return Value::String(String::new());
    }
    serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    #[test]
    fn split_model_and_effort_supports_reasoning_suffixes() {
        assert_eq!(
            split_model_and_effort("gpt-5.4/xhigh"),
            ("gpt-5.4".to_string(), Some("xhigh".to_string()))
        );
        assert_eq!(
            split_model_and_effort("gpt-5.4"),
            ("gpt-5.4".to_string(), None)
        );
    }

    #[test]
    fn parse_cli_config_overrides_parses_json_literals() {
        let overrides = parse_cli_config_overrides(&[
            "show_raw_agent_reasoning=true".to_string(),
            "model_reasoning_summary=\"detailed\"".to_string(),
            "max_output_tokens=4096".to_string(),
        ])
        .expect("config overrides should parse");
        assert_eq!(
            overrides,
            Some(serde_json::json!({
                "show_raw_agent_reasoning": true,
                "model_reasoning_summary": "detailed",
                "max_output_tokens": 4096
            }))
        );
    }

    #[test]
    fn build_current_model_id_uses_explicit_reasoning_effort() {
        let config = CrpSessionConfig {
            model: Some("gpt-5.4".to_string()),
            reasoning_effort: Some("high".to_string()),
            ..CrpSessionConfig::default()
        };
        assert_eq!(
            build_current_model_id(Some(&config), &[], None, None),
            Some("gpt-5.4/high".to_string())
        );
    }

    #[test]
    fn parse_frontmatter_extracts_description_and_argument_hint() {
        let (description, argument_hint, body) = parse_frontmatter(
            "---\ndescription: \"Review\"\nargument_hint: \"[path]\"\n---\nBody\n",
        );
        assert_eq!(description.as_deref(), Some("Review"));
        assert_eq!(argument_hint.as_deref(), Some("[path]"));
        assert_eq!(body, "Body\n");
    }

    #[test]
    fn discover_prompts_reads_markdown_files() {
        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join("alpha.md"), "hello").expect("write prompt");
        let prompts = discover_prompts_in_excluding(dir.path(), &HashSet::new());
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].name, "alpha");
    }
}
