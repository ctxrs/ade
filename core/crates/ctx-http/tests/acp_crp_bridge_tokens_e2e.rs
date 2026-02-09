use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use tokio::process::Command;
use tokio::sync::mpsc;

use ctx_core::models::SessionEventType;
use ctx_providers::adapters::{ProviderAdapter, TurnInput};
use ctx_providers::crp::Tier1CrpAdapter;
use ctx_providers::events::NormalizedEvent;

use ctx_http::installer::{load_agent_server_config, resolve_provider_command, AgentServerCommand};

const DEFAULT_MODEL: &str = "google/gemini-3-flash-preview";
const DEFAULT_OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";

#[derive(Clone, Copy)]
struct ProviderSpec {
    id: &'static str,
    fallback_cmd: &'static str,
    fallback_args: &'static [&'static str],
    opencode_config: bool,
}

const PROVIDERS: &[ProviderSpec] = &[
    ProviderSpec {
        id: "gemini",
        fallback_cmd: "gemini",
        fallback_args: &["--experimental-acp"],
        opencode_config: false,
    },
    ProviderSpec {
        id: "opencode",
        fallback_cmd: "opencode",
        fallback_args: &["acp"],
        opencode_config: true,
    },
    ProviderSpec {
        id: "mistral",
        fallback_cmd: "vibe-acp",
        fallback_args: &[],
        opencode_config: false,
    },
    ProviderSpec {
        id: "goose",
        fallback_cmd: "goose",
        fallback_args: &["acp"],
        opencode_config: false,
    },
    ProviderSpec {
        id: "kimi",
        fallback_cmd: "kimi",
        fallback_args: &["--acp"],
        opencode_config: false,
    },
    ProviderSpec {
        id: "auggie",
        fallback_cmd: "auggie",
        fallback_args: &["--acp"],
        opencode_config: false,
    },
    ProviderSpec {
        id: "cagent",
        fallback_cmd: "cagent",
        fallback_args: &["acp"],
        opencode_config: false,
    },
    ProviderSpec {
        id: "amp",
        fallback_cmd: "amp-acp",
        fallback_args: &[],
        opencode_config: false,
    },
    ProviderSpec {
        id: "droid",
        fallback_cmd: "droid-acp",
        fallback_args: &[],
        opencode_config: false,
    },
    ProviderSpec {
        id: "copilot",
        fallback_cmd: "copilot-cli-acp",
        fallback_args: &[],
        opencode_config: false,
    },
    ProviderSpec {
        id: "kiro",
        fallback_cmd: "kiro-acp",
        fallback_args: &[],
        opencode_config: false,
    },
    ProviderSpec {
        id: "rovo",
        fallback_cmd: "rovo-dev-acp",
        fallback_args: &[],
        opencode_config: false,
    },
    ProviderSpec {
        id: "cody",
        fallback_cmd: "cody-acp",
        fallback_args: &[],
        opencode_config: false,
    },
    ProviderSpec {
        id: "continue",
        fallback_cmd: "cn",
        fallback_args: &["acp"],
        opencode_config: false,
    },
    ProviderSpec {
        id: "cline",
        fallback_cmd: "cline-acp",
        fallback_args: &[],
        opencode_config: false,
    },
    ProviderSpec {
        id: "swe-agent",
        fallback_cmd: "sweagent",
        fallback_args: &["acp"],
        opencode_config: false,
    },
    ProviderSpec {
        id: "openhands",
        fallback_cmd: "openhands",
        fallback_args: &["acp"],
        opencode_config: false,
    },
    ProviderSpec {
        id: "qwen",
        fallback_cmd: "qwen",
        fallback_args: &["--experimental-acp"],
        opencode_config: false,
    },
];

#[derive(Deserialize)]
struct TitleGenSettings {
    api_key: String,
    #[serde(default)]
    base_url: Option<String>,
}

#[derive(Deserialize)]
struct SettingsFile {
    title_generation: Option<TitleGenSettings>,
}

async fn run_git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn setup_git_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    run_git(root, &["init"]).await;
    run_git(root, &["config", "user.email", "test@example.com"]).await;
    run_git(root, &["config", "user.name", "Test"]).await;
    std::fs::write(root.join("note.txt"), "hello\n").unwrap();
    run_git(root, &["add", "."]).await;
    run_git(root, &["commit", "-m", "init"]).await;
    dir
}

async fn run_and_collect(
    adapter: &dyn ProviderAdapter,
    workdir: &Path,
    prompt: &str,
    model_id: &str,
    env: HashMap<String, String>,
) -> Vec<NormalizedEvent> {
    let (tx, mut rx) = mpsc::channel::<NormalizedEvent>(1024);
    let handle = adapter
        .run(
            TurnInput {
                content: prompt.to_string(),
                attachments: vec![],
                context_blocks: vec![],
                model_id: Some(model_id.to_string()),
            },
            workdir.to_path_buf(),
            env,
            tx,
        )
        .await
        .unwrap();

    let events = tokio::time::timeout(Duration::from_secs(240), async move {
        let mut events = Vec::new();
        while let Some(ev) = rx.recv().await {
            let done = matches!(ev.event_type, SessionEventType::Done);
            events.push(ev);
            if done {
                break;
            }
        }
        events
    })
    .await
    .expect("timed out waiting for provider events");

    let _ = handle.done.await;
    events
}

fn expect_success(events: &[NormalizedEvent], provider_id: &str) {
    if events
        .iter()
        .any(|e| matches!(e.event_type, SessionEventType::Error))
    {
        panic!("{provider_id} emitted error event(s): {events:#?}");
    }

    let has_assistant = events
        .iter()
        .any(|e| matches!(e.event_type, SessionEventType::AssistantComplete));
    let has_thought = events
        .iter()
        .any(|e| matches!(e.event_type, SessionEventType::ThoughtChunk));
    if !has_assistant {
        if provider_id == "opencode" && has_thought {
            // Opencode ACP can emit reasoning without a final assistant chunk; accept for now.
        } else {
            panic!("{provider_id} produced no AssistantComplete event: {events:#?}");
        }
    }

    assert!(
        events
            .iter()
            .any(|e| matches!(e.event_type, SessionEventType::Done)),
        "{provider_id} produced no Done event: {events:#?}"
    );
}

fn token_tests_enabled() -> bool {
    std::env::var("CTX_E2E_TIER")
        .map(|v| v.eq_ignore_ascii_case("tokens"))
        .unwrap_or(false)
        || std::env::var("CTX_TOKEN_TESTS")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
}

fn resolve_data_root() -> PathBuf {
    if let Ok(val) = std::env::var("CTX_DATA_ROOT") {
        if !val.trim().is_empty() {
            return PathBuf::from(val);
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".ctx")
}

fn ensure_cagent_config(data_root: &Path) -> Option<PathBuf> {
    let cfg_path = data_root
        .join("providers")
        .join("agent-servers")
        .join("cagent")
        .join("config.yaml");
    if cfg_path.exists() {
        return Some(cfg_path);
    }
    if let Some(parent) = cfg_path.parent() {
        fs::create_dir_all(parent).ok()?;
    }
    let cfg = r#"agents:
  root:
    model: openai/gpt-5-mini
    description: ctx default agent
    instruction: |
      You are a helpful coding assistant.
"#;
    fs::write(&cfg_path, cfg).ok()?;
    Some(cfg_path)
}

fn load_openrouter_settings(data_root: &Path) -> Option<(String, String)> {
    let settings_path = data_root.join("settings.json");
    let raw = std::fs::read_to_string(settings_path).ok()?;
    let parsed: SettingsFile = serde_json::from_str(&raw).ok()?;
    let title = parsed.title_generation?;
    let base_url = title
        .base_url
        .unwrap_or_else(|| DEFAULT_OPENROUTER_BASE_URL.to_string());
    Some((title.api_key, base_url))
}

fn escape_shell_arg(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }
    let is_simple = value
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"@%_-+=:,./".contains(&b));
    if is_simple {
        return value.to_string();
    }
    let mut out = String::from("'");
    for ch in value.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

fn format_shell_command(command: &str, args: &[String]) -> String {
    let mut parts = Vec::with_capacity(1 + args.len());
    parts.push(escape_shell_arg(command));
    for arg in args {
        parts.push(escape_shell_arg(arg));
    }
    parts.join(" ")
}

fn resolve_command(
    cfg: &ctx_http::installer::AgentServerConfigFile,
    data_root: &Path,
    provider_id: &str,
    fallback_cmd: &str,
    fallback_args: &[&str],
) -> AgentServerCommand {
    let mut cmd =
        resolve_provider_command(cfg, provider_id).unwrap_or_else(|| AgentServerCommand {
            command: fallback_cmd.to_string(),
            args: fallback_args.iter().map(|s| s.to_string()).collect(),
            dependencies: Vec::new(),
            managed: None,
        });
    if provider_id == "cagent" {
        let cfg_path = ensure_cagent_config(data_root);
        if let Some(cfg_path) = cfg_path {
            let cfg_str = cfg_path.to_string_lossy().to_string();
            for arg in &mut cmd.args {
                if arg == "{{cagent_config}}" {
                    *arg = cfg_str.clone();
                }
            }
        }
    }
    cmd
}

fn command_exists(command: &str) -> bool {
    let path = Path::new(command);
    if path.is_absolute() {
        return path.exists();
    }
    which::which(command).is_ok()
}

fn truncate_output(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let trimmed = text.trim();
    let out: String = trimmed.chars().take(240).collect();
    if out.is_empty() {
        "<no output>".to_string()
    } else {
        out
    }
}

async fn probe_command(
    command: &str,
    args: &[String],
    extra_args: &[&str],
    env: Option<&HashMap<String, String>>,
) -> Result<(), String> {
    let output = tokio::time::timeout(Duration::from_secs(5), async {
        let mut cmd = Command::new(command);
        cmd.args(args).args(extra_args);
        if let Some(env) = env {
            cmd.envs(env);
        }
        cmd.output().await
    })
    .await
    .map_err(|_| format!("{} probe timed out", command))?
    .map_err(|err| format!("{} probe failed: {err}", command))?;

    if output.status.success() {
        return Ok(());
    }

    let detail = if !output.stderr.is_empty() {
        truncate_output(&output.stderr)
    } else {
        truncate_output(&output.stdout)
    };
    Err(format!(
        "{} {:?} exited {}: {}",
        command, extra_args, output.status, detail
    ))
}

fn resolve_repo_root() -> Option<PathBuf> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut current = manifest_dir.as_path();
    for _ in 0..4 {
        if current.join(".ctx").is_dir() {
            return Some(current.to_path_buf());
        }
        current = current.parent()?;
    }
    None
}

fn resolve_swe_agent_config_dir() -> Option<PathBuf> {
    let repo_root = resolve_repo_root()?;
    let candidate = repo_root.join(".ctx/attachments/refs/swe-agent/config");
    if candidate.is_dir() {
        Some(candidate)
    } else {
        None
    }
}

fn resolve_swe_agent_tools_dir() -> Option<PathBuf> {
    let repo_root = resolve_repo_root()?;
    let candidate = repo_root.join(".ctx/attachments/refs/swe-agent/tools");
    if candidate.is_dir() {
        Some(candidate)
    } else {
        None
    }
}

fn resolve_swe_agent_trajectories_dir() -> Option<PathBuf> {
    let repo_root = resolve_repo_root()?;
    let candidate = repo_root.join(".ctx/attachments/refs/swe-agent/trajectories");
    if candidate.is_dir() {
        Some(candidate)
    } else {
        None
    }
}

fn create_cline_vscode_stub() -> std::io::Result<(tempfile::TempDir, PathBuf)> {
    let dir = tempfile::tempdir()?;
    let node_modules = dir.path().join("node_modules/vscode");
    fs::create_dir_all(&node_modules)?;
    let node_path = dir.path().join("node_modules");
    let stub = r#"module.exports = {
  workspace: {
    getConfiguration: () => ({ get: () => undefined })
  },
  ExtensionMode: { Development: 0, Production: 1, Test: 2 },
  ExtensionKind: { UI: 1, Workspace: 2 }
};
"#;
    fs::write(node_modules.join("index.js"), stub)?;
    Ok((dir, node_path))
}

fn create_qwen_settings_home() -> std::io::Result<tempfile::TempDir> {
    let dir = tempfile::tempdir()?;
    let qwen_dir = dir.path().join(".qwen");
    fs::create_dir_all(&qwen_dir)?;
    let settings = r#"{
  "$version": 2,
  "security": { "auth": { "selectedType": "openai" } }
}
"#;
    fs::write(qwen_dir.join("settings.json"), settings)?;
    Ok(dir)
}

fn env_truthy(name: &str) -> bool {
    std::env::var(name)
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn env_present(name: &str) -> bool {
    std::env::var(name)
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false)
}

fn provider_skip_reason(provider: ProviderSpec) -> Option<String> {
    if provider.id == "gemini" {
        let has_gemini_auth = std::env::var("GEMINI_API_KEY")
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false)
            || std::env::var("GOOGLE_API_KEY")
                .map(|v| !v.trim().is_empty())
                .unwrap_or(false)
            || std::env::var("GOOGLE_APPLICATION_CREDENTIALS")
                .map(|v| !v.trim().is_empty())
                .unwrap_or(false)
            || std::env::var("GOOGLE_CLOUD_PROJECT")
                .map(|v| !v.trim().is_empty())
                .unwrap_or(false)
            || std::env::var("GOOGLE_CLOUD_PROJECT_ID")
                .map(|v| !v.trim().is_empty())
                .unwrap_or(false);
        if !has_gemini_auth {
            return Some(
                "missing GEMINI/GOOGLE auth; gemini-cli does not support OpenRouter base URLs"
                    .to_string(),
            );
        }
    }
    if provider.id == "mistral" {
        let has_mistral_auth = std::env::var("MISTRAL_API_KEY")
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false);
        if !has_mistral_auth {
            return Some(
                "missing MISTRAL_API_KEY; mistral-vibe does not support OpenRouter".to_string(),
            );
        }
    }
    if provider.id == "auggie" {
        return Some("requires Augment login; not OpenRouter-compatible".to_string());
    }
    if provider.id == "amp" {
        let allow = env_truthy("AMP_TOKEN_TESTS");
        let has_amp_auth = env_present("AMP_API_KEY") || env_present("AMP_SETTINGS_FILE");
        if !allow && !has_amp_auth {
            return Some(
                "missing AMP_API_KEY/AMP_SETTINGS_FILE; set AMP_TOKEN_TESTS=1 to attempt"
                    .to_string(),
            );
        }
    }
    if provider.id == "droid" {
        let allow = env_truthy("DROID_TOKEN_TESTS");
        let has_droid_auth = env_present("FACTORY_API_KEY");
        if !allow && !has_droid_auth {
            return Some("missing FACTORY_API_KEY; set DROID_TOKEN_TESTS=1 to attempt".to_string());
        }
    }
    if provider.id == "copilot" {
        return Some("requires GitHub Copilot login; not OpenRouter-compatible".to_string());
    }
    if provider.id == "kiro" {
        let allow = env_truthy("KIRO_TOKEN_TESTS");
        if !allow {
            return Some("requires Kiro CLI login; set KIRO_TOKEN_TESTS=1 to attempt".to_string());
        }
    }
    if provider.id == "rovo" {
        return Some("requires Atlassian auth; not OpenRouter-compatible".to_string());
    }
    if provider.id == "cody" {
        let allow = env_truthy("CODY_TOKEN_TESTS");
        let has_cody_auth = env_present("SRC_ACCESS_TOKEN");
        if !allow && !has_cody_auth {
            return Some("missing SRC_ACCESS_TOKEN; set CODY_TOKEN_TESTS=1 to attempt".to_string());
        }
    }
    if provider.id == "continue" {
        let has_continue_auth = std::env::var("CONTINUE_API_KEY")
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false);
        if !has_continue_auth {
            return Some("missing CONTINUE_API_KEY; continue CLI uses Continue Cloud".to_string());
        }
    }
    if provider.id == "cline" {
        let allow = env_truthy("CLINE_TOKEN_TESTS");
        if !allow {
            return Some(
                "cline-acp bundle missing deps (vscode/grpc-health-check/package.json); set CLINE_TOKEN_TESTS=1 to attempt".to_string(),
            );
        }
    }
    if provider.id == "swe-agent" {
        let allow = env_truthy("SWE_AGENT_TOKEN_TESTS");
        if !allow {
            return Some(
                "swe-agent ACP spins up SWEEnv and can hang; set SWE_AGENT_TOKEN_TESTS=1 to run"
                    .to_string(),
            );
        }
    }
    None
}

fn maybe_add_qwen_auth(args: &mut Vec<String>) {
    if args.iter().any(|arg| arg == "--auth-type") {
        return;
    }
    args.push("--auth-type".to_string());
    args.push("openai".to_string());
}

fn build_env(
    openrouter_api_key: &str,
    openrouter_base_url: &str,
    model_id: &str,
    provider: ProviderSpec,
    swe_agent_config_dir: Option<&Path>,
    swe_agent_tools_dir: Option<&Path>,
    swe_agent_trajectories_dir: Option<&Path>,
) -> HashMap<String, String> {
    let mut env = HashMap::new();
    env.insert(
        "OPENROUTER_API_KEY".to_string(),
        openrouter_api_key.to_string(),
    );
    env.insert(
        "OPENROUTER_BASE_URL".to_string(),
        openrouter_base_url.to_string(),
    );
    env.insert("OPENAI_API_KEY".to_string(), openrouter_api_key.to_string());
    env.insert(
        "OPENAI_BASE_URL".to_string(),
        openrouter_base_url.to_string(),
    );
    env.insert("OPENAI_MODEL".to_string(), model_id.to_string());

    if provider.opencode_config {
        let (opencode_model, opencode_model_key) = match model_id.strip_prefix("openrouter/") {
            Some(model_key) => (model_id.to_string(), model_key.to_string()),
            None => (format!("openrouter/{model_id}"), model_id.to_string()),
        };
        let cfg = serde_json::json!({
            "model": opencode_model,
            "provider": {
                "openrouter": {
                    "options": {
                        "baseURL": openrouter_base_url,
                        "apiKey": openrouter_api_key
                    },
                    "models": {
                        opencode_model_key: {}
                    }
                }
            }
        });
        env.insert("OPENCODE_CONFIG_CONTENT".to_string(), cfg.to_string());
    }

    if provider.id == "goose" {
        env.insert("GOOSE_PROVIDER".to_string(), "openai".to_string());
        env.insert("GOOSE_MODEL".to_string(), model_id.to_string());
        env.insert("GOOSE_DISABLE_KEYRING".to_string(), "1".to_string());
    }

    if provider.id == "kimi" {
        env.insert("KIMI_BASE_URL".to_string(), openrouter_base_url.to_string());
        env.insert("KIMI_API_KEY".to_string(), openrouter_api_key.to_string());
        env.insert("KIMI_MODEL_NAME".to_string(), model_id.to_string());
    }

    if provider.id == "swe-agent" {
        if let Some(dir) = swe_agent_config_dir {
            env.insert(
                "SWE_AGENT_CONFIG_DIR".to_string(),
                dir.display().to_string(),
            );
        }
        if let Some(dir) = swe_agent_tools_dir {
            env.insert("SWE_AGENT_TOOLS_DIR".to_string(), dir.display().to_string());
        }
        if let Some(dir) = swe_agent_trajectories_dir {
            env.insert(
                "SWE_AGENT_TRAJECTORY_DIR".to_string(),
                dir.display().to_string(),
            );
        }
    }

    env
}

#[tokio::test]
#[ignore]
async fn acp_crp_bridge_token_providers() {
    if !token_tests_enabled() {
        eprintln!("skipping token tests; set CTX_E2E_TIER=tokens or CTX_TOKEN_TESTS=1");
        return;
    }

    let data_root = resolve_data_root();
    let (openrouter_api_key, openrouter_base_url) = match (
        std::env::var("OPENROUTER_API_KEY").ok(),
        std::env::var("OPENROUTER_BASE_URL").ok(),
    ) {
        (Some(key), Some(base_url)) => (key, base_url),
        _ => match load_openrouter_settings(&data_root) {
            Some(creds) => creds,
            None => {
                eprintln!(
                    "skipping token tests; missing OpenRouter credentials (set OPENROUTER_API_KEY/OPENROUTER_BASE_URL or configure title_generation in settings.json)"
                );
                return;
            }
        },
    };

    let model_id = std::env::var("CTX_TOKENS_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());

    let cfg = load_agent_server_config(&data_root)
        .await
        .unwrap_or_default();
    let bridge_cmd = resolve_command(&cfg, &data_root, "acp-crp-bridge", "acp-crp-bridge", &[]);

    if !command_exists(&bridge_cmd.command) {
        panic!("acp-crp-bridge not found: {}", bridge_cmd.command);
    }

    let repo = setup_git_repo().await;
    let prompt = "Reply with the single word: pong";
    let swe_agent_config_dir = resolve_swe_agent_config_dir();
    let swe_agent_tools_dir = resolve_swe_agent_tools_dir();
    let swe_agent_trajectories_dir = resolve_swe_agent_trajectories_dir();

    for provider in PROVIDERS {
        let acp_cmd = resolve_command(
            &cfg,
            &data_root,
            provider.id,
            provider.fallback_cmd,
            provider.fallback_args,
        );
        if !command_exists(&acp_cmd.command) {
            eprintln!(
                "skipping {}: command not found ({})",
                provider.id, acp_cmd.command
            );
            continue;
        }
        if let Some(reason) = provider_skip_reason(*provider) {
            eprintln!("skipping {}: {}", provider.id, reason);
            continue;
        }

        eprintln!("running {}...", provider.id);
        let mut _cline_stub = None;
        let mut _qwen_home = None;
        let mut acp_args = acp_cmd.args.clone();
        if provider.id == "qwen" {
            maybe_add_qwen_auth(&mut acp_args);
        }
        if provider.id == "swe-agent" {
            if let Some(config_dir) = swe_agent_config_dir.as_ref() {
                let model = if model_id.starts_with("openrouter/") {
                    model_id.to_string()
                } else {
                    format!("openrouter/{model_id}")
                };
                acp_args.push("--agent.model.name".to_string());
                acp_args.push(model);
                acp_args.push("--agent.model.api_key".to_string());
                acp_args.push("$OPENROUTER_API_KEY".to_string());
                acp_args.push("--agent.model.api_base".to_string());
                acp_args.push(openrouter_base_url.to_string());
                acp_args.push("--config".to_string());
                acp_args.push(config_dir.join("default.yaml").display().to_string());
            } else {
                eprintln!("skipping {}: swe-agent config dir not found", provider.id);
                continue;
            }
        }

        let mut env = build_env(
            &openrouter_api_key,
            &openrouter_base_url,
            &model_id,
            *provider,
            swe_agent_config_dir.as_deref(),
            swe_agent_tools_dir.as_deref(),
            swe_agent_trajectories_dir.as_deref(),
        );
        if provider.id == "cline" {
            match create_cline_vscode_stub() {
                Ok((dir, node_path)) => {
                    env.insert("NODE_PATH".to_string(), node_path.display().to_string());
                    _cline_stub = Some(dir);
                }
                Err(err) => {
                    eprintln!(
                        "skipping {}: failed to create vscode stub: {}",
                        provider.id, err
                    );
                    continue;
                }
            }
            if let Err(reason) =
                probe_command(&acp_cmd.command, &acp_cmd.args, &["--version"], Some(&env)).await
            {
                eprintln!("skipping {}: {}", provider.id, reason);
                continue;
            }
        }
        if provider.id == "qwen" {
            match create_qwen_settings_home() {
                Ok(dir) => {
                    env.insert("HOME".to_string(), dir.path().display().to_string());
                    _qwen_home = Some(dir);
                }
                Err(err) => {
                    eprintln!(
                        "skipping {}: failed to create qwen settings: {}",
                        provider.id, err
                    );
                    continue;
                }
            }
        }
        if provider.id == "openhands" {
            if let Err(reason) =
                probe_command(&acp_cmd.command, &acp_cmd.args, &["--help"], Some(&env)).await
            {
                eprintln!("skipping {}: {}", provider.id, reason);
                continue;
            }
        }
        if provider.id == "swe-agent" {
            if let Err(reason) =
                probe_command(&acp_cmd.command, &acp_cmd.args, &["--version"], Some(&env)).await
            {
                eprintln!("skipping {}: {}", provider.id, reason);
                continue;
            }
        }

        let acp_command = format_shell_command(&acp_cmd.command, &acp_args);
        let mut bridge_args = bridge_cmd.args.clone();
        bridge_args.push("--acp-command".to_string());
        bridge_args.push(acp_command);

        let adapter =
            Tier1CrpAdapter::from_raw(provider.id, bridge_cmd.command.clone(), bridge_args);
        let events = run_and_collect(&adapter, repo.path(), prompt, &model_id, env).await;
        expect_success(&events, provider.id);
        eprintln!("completed {}", provider.id);
    }
}
