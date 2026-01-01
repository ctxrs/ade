use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::Utc;
use serde::Deserialize;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use ctx_core::ids::WorktreeId;
use ctx_core::models::{Workspace, Worktree, WorktreeBootstrapNotice, WorktreeBootstrapStatus};
use ctx_store::WorktreeBootstrapResultUpdate;

use crate::daemon::AppState;
use crate::logs;

const CONFIG_REL_PATH: &str = ".ctx/config.toml";
const DEFAULT_TIMEOUT_SEC: u64 = 60;
const MAX_LOG_BYTES: usize = 200 * 1024;

#[derive(Debug, Clone, Deserialize)]
struct WorkspaceConfigFile {
    #[serde(default)]
    worktree: Option<WorktreeConfig>,
}

#[derive(Debug, Clone, Deserialize)]
struct WorktreeConfig {
    #[serde(default)]
    bootstrap: Option<WorktreeBootstrapConfig>,
}

#[derive(Debug, Clone, Deserialize)]
struct WorktreeBootstrapConfig {
    #[serde(default)]
    setup_worktree: Option<BootstrapCommandSpec>,
    #[serde(default)]
    setup_worktree_unix: Option<BootstrapCommandSpec>,
    #[serde(default)]
    setup_worktree_windows: Option<BootstrapCommandSpec>,
    #[serde(default)]
    timeout_sec: Option<u64>,
    #[serde(default)]
    wait_for_completion: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum BootstrapCommandSpec {
    Script(String),
    Commands(Vec<String>),
}

#[derive(Debug, Clone)]
struct ResolvedBootstrap {
    config_path: PathBuf,
    config_key: String,
    timeout: Duration,
    spec: BootstrapCommandSpec,
    wait_for_completion: bool,
}

#[derive(Debug, Clone)]
struct WorktreeBootstrapPlan {
    config_path: PathBuf,
    config_key: String,
    timeout: Duration,
    steps: Vec<BootstrapStep>,
    wait_for_completion: bool,
}

#[derive(Debug, Clone)]
struct BootstrapStep {
    label: String,
    kind: BootstrapStepKind,
}

#[derive(Debug, Clone)]
enum BootstrapStepKind {
    Script { path: PathBuf, original: String },
    Command { command: String },
}

#[derive(Debug)]
struct BootstrapCommandResult {
    status: Option<i32>,
    stdout: String,
    stderr: String,
    timed_out: bool,
}

pub async fn run_worktree_bootstrap(
    state: &AppState,
    workspace: &Workspace,
    worktree: &Worktree,
) -> Result<()> {
    let Some(plan) = prepare_worktree_bootstrap(state, workspace, worktree).await? else {
        return Ok(());
    };
    run_worktree_bootstrap_plan(state, workspace, worktree, plan).await
}

pub async fn spawn_worktree_bootstrap(
    state: Arc<AppState>,
    workspace: Workspace,
    worktree: Worktree,
) -> Result<()> {
    let Some(plan) = prepare_worktree_bootstrap(&state, &workspace, &worktree).await? else {
        return Ok(());
    };

    let worktree_id = worktree.id;
    state
        .register_worktree_bootstrap(worktree_id, plan.wait_for_completion)
        .await;

    tokio::spawn(async move {
        if let Err(e) = run_worktree_bootstrap_plan(&state, &workspace, &worktree, plan).await {
            tracing::warn!(worktree_id = %worktree_id.0, "worktree bootstrap failed: {e:?}");
        }
        state.finish_worktree_bootstrap(worktree_id).await;
    });

    Ok(())
}

async fn prepare_worktree_bootstrap(
    state: &AppState,
    workspace: &Workspace,
    worktree: &Worktree,
) -> Result<Option<WorktreeBootstrapPlan>> {
    if worktree.git_branch.is_none() {
        return Ok(None);
    }

    let bootstrap = match load_bootstrap_config(worktree, workspace).await {
        Ok(Some(cfg)) => cfg,
        Ok(None) => return Ok(None),
        Err(err) => {
            let started_at = Utc::now();
            let error = err.to_string();
            let error_details = format!("{err:#}");
            let config_path = detect_bootstrap_config_path(worktree, workspace);
            let mut log = String::new();
            log.push_str("# ctx worktree bootstrap\n");
            log.push_str(&format!("# Worktree: {}\n", worktree.root_path.trim()));
            if let Some(path) = &config_path {
                log.push_str(&format!("# Config: {}\n", path.display()));
            }
            log.push_str(&format!("# Started: {}\n\n", started_at.to_rfc3339()));
            log.push_str("[error]\n");
            log.push_str(&error_details);
            log.push('\n');
            let finished_at = Utc::now();
            log.push_str(&format!("\n# Finished: {}\n", finished_at.to_rfc3339()));
            log.push_str("# Status: Failed\n");

            let (log, log_truncated) = truncate_log(&logs::redact_sensitive(&log));
            let log_path = write_bootstrap_log(state, worktree.id, &log).await.ok();
            update_bootstrap_result(
                state,
                WorktreeBootstrapResultUpdate {
                    worktree_id: worktree.id,
                    status: WorktreeBootstrapStatus::Failed,
                    started_at,
                    finished_at,
                    exit_code: None,
                    timeout_sec: Some(DEFAULT_TIMEOUT_SEC as i64),
                    error: Some(error.clone()),
                    log_path: log_path.as_ref().map(|p| p.to_string_lossy().to_string()),
                    log_truncated: Some(log_truncated),
                    config_path: config_path
                        .as_ref()
                        .map(|p| p.to_string_lossy().to_string()),
                    config_key: None,
                    command: None,
                    script_path: None,
                },
            )
            .await;
            let notice = WorktreeBootstrapNotice {
                worktree_id: worktree.id,
                worktree_root: worktree.root_path.clone(),
                status: WorktreeBootstrapStatus::Failed,
                started_at,
                finished_at,
                exit_code: None,
                timeout_sec: Some(DEFAULT_TIMEOUT_SEC as i64),
                config_path: config_path
                    .as_ref()
                    .map(|p| p.to_string_lossy().to_string()),
                config_key: None,
                command: None,
                script_path: None,
                log_path: log_path.map(|p| p.to_string_lossy().to_string()),
                log_truncated: Some(log_truncated),
                error: Some(error),
            };
            emit_failure_notice(state, workspace.id, notice).await;
            return Ok(None);
        }
    };

    let steps = build_bootstrap_steps(&bootstrap.spec, worktree)?;
    if steps.is_empty() {
        return Ok(None);
    }

    Ok(Some(WorktreeBootstrapPlan {
        config_path: bootstrap.config_path,
        config_key: bootstrap.config_key,
        timeout: bootstrap.timeout,
        steps,
        wait_for_completion: bootstrap.wait_for_completion,
    }))
}

async fn run_worktree_bootstrap_plan(
    state: &AppState,
    workspace: &Workspace,
    worktree: &Worktree,
    plan: WorktreeBootstrapPlan,
) -> Result<()> {
    let WorktreeBootstrapPlan {
        config_path,
        config_key,
        timeout,
        steps,
        ..
    } = plan;

    let started_at = Utc::now();
    let mut log = String::new();
    log.push_str("# ctx worktree bootstrap\n");
    log.push_str(&format!("# Worktree: {}\n", worktree.root_path.trim()));
    log.push_str(&format!("# Config: {}\n", config_path.display()));
    log.push_str(&format!("# Key: {}\n", config_key));
    log.push_str(&format!("# Started: {}\n\n", started_at.to_rfc3339()));

    let mut last_exit = None;
    let mut last_step: Option<BootstrapStep> = None;
    let mut failure_status = None;
    let mut failure_error = None;

    for step in &steps {
        last_step = Some(step.clone());
        log.push_str(&format!("$ {}\n", step.label));

        let result = match run_bootstrap_step(step, workspace, worktree, timeout).await {
            Ok(result) => result,
            Err(err) => {
                failure_status = Some(WorktreeBootstrapStatus::Failed);
                failure_error = Some(err.to_string());
                break;
            }
        };
        last_exit = result.status.map(|v| v as i64);

        append_output(&mut log, &result.stdout, "stdout");
        append_output(&mut log, &result.stderr, "stderr");

        if result.timed_out {
            failure_status = Some(WorktreeBootstrapStatus::Timeout);
            failure_error = Some(format!("bootstrap timed out after {}s", timeout.as_secs()));
            break;
        }

        if let Some(code) = result.status {
            if code != 0 {
                failure_status = Some(WorktreeBootstrapStatus::Failed);
                failure_error = Some(format!("command exited with code {code}"));
                break;
            }
        }
    }

    let finished_at = Utc::now();
    log.push_str(&format!("\n# Finished: {}\n", finished_at.to_rfc3339()));
    if let Some(status) = &failure_status {
        log.push_str(&format!("# Status: {:?}\n", status));
    } else {
        log.push_str("# Status: success\n");
    }

    let (log, log_truncated) = truncate_log(&logs::redact_sensitive(&log));
    let log_path = write_bootstrap_log(state, worktree.id, &log).await.ok();

    let (status, error) = match failure_status {
        Some(status) => (status, failure_error),
        None => (WorktreeBootstrapStatus::Success, None),
    };

    update_bootstrap_result(
        state,
        WorktreeBootstrapResultUpdate {
            worktree_id: worktree.id,
            status: status.clone(),
            started_at,
            finished_at,
            exit_code: last_exit,
            timeout_sec: Some(timeout.as_secs() as i64),
            error: error.clone(),
            log_path: log_path.as_ref().map(|p| p.to_string_lossy().to_string()),
            log_truncated: Some(log_truncated),
            config_path: Some(config_path.to_string_lossy().to_string()),
            config_key: Some(config_key.clone()),
            command: last_step.as_ref().and_then(|step| step.command_value()),
            script_path: last_step.as_ref().and_then(|step| step.script_value()),
        },
    )
    .await;

    if status != WorktreeBootstrapStatus::Success {
        let notice = WorktreeBootstrapNotice {
            worktree_id: worktree.id,
            worktree_root: worktree.root_path.clone(),
            status,
            started_at,
            finished_at,
            exit_code: last_exit,
            timeout_sec: Some(timeout.as_secs() as i64),
            config_path: Some(config_path.to_string_lossy().to_string()),
            config_key: Some(config_key.clone()),
            command: last_step.as_ref().and_then(|step| step.command_value()),
            script_path: last_step.as_ref().and_then(|step| step.script_value()),
            log_path: log_path.map(|p| p.to_string_lossy().to_string()),
            log_truncated: Some(log_truncated),
            error,
        };
        emit_failure_notice(state, workspace.id, notice).await;
    }

    Ok(())
}

impl BootstrapStep {
    fn command_value(&self) -> Option<String> {
        match &self.kind {
            BootstrapStepKind::Command { command } => Some(command.clone()),
            _ => None,
        }
    }

    fn script_value(&self) -> Option<String> {
        match &self.kind {
            BootstrapStepKind::Script { original, .. } => Some(original.clone()),
            _ => None,
        }
    }
}

async fn load_bootstrap_config(
    worktree: &Worktree,
    workspace: &Workspace,
) -> Result<Option<ResolvedBootstrap>> {
    let worktree_cfg_path = Path::new(&worktree.root_path).join(CONFIG_REL_PATH);
    if let Some(cfg) = load_bootstrap_config_at(&worktree_cfg_path).await? {
        return Ok(resolve_bootstrap(cfg, worktree_cfg_path));
    }

    let workspace_cfg_path = Path::new(&workspace.root_path).join(CONFIG_REL_PATH);
    if workspace_cfg_path == worktree_cfg_path {
        return Ok(None);
    }
    if let Some(cfg) = load_bootstrap_config_at(&workspace_cfg_path).await? {
        return Ok(resolve_bootstrap(cfg, workspace_cfg_path));
    }
    Ok(None)
}

fn resolve_bootstrap(
    cfg: WorktreeBootstrapConfig,
    config_path: PathBuf,
) -> Option<ResolvedBootstrap> {
    let is_windows = cfg!(windows);
    let (key, spec) = if is_windows {
        cfg.setup_worktree_windows
            .map(|spec| ("setup_worktree_windows".to_string(), spec))
            .or_else(|| {
                cfg.setup_worktree
                    .map(|spec| ("setup_worktree".to_string(), spec))
            })
    } else {
        cfg.setup_worktree_unix
            .map(|spec| ("setup_worktree_unix".to_string(), spec))
            .or_else(|| {
                cfg.setup_worktree
                    .map(|spec| ("setup_worktree".to_string(), spec))
            })
    }?;

    let timeout_sec = cfg.timeout_sec.unwrap_or(DEFAULT_TIMEOUT_SEC);
    let timeout_sec = if timeout_sec == 0 {
        DEFAULT_TIMEOUT_SEC
    } else {
        timeout_sec
    };
    let wait_for_completion = cfg.wait_for_completion.unwrap_or(false);
    Some(ResolvedBootstrap {
        config_path,
        config_key: key,
        timeout: Duration::from_secs(timeout_sec),
        spec,
        wait_for_completion,
    })
}

async fn load_bootstrap_config_at(path: &Path) -> Result<Option<WorktreeBootstrapConfig>> {
    if !path.exists() {
        return Ok(None);
    }
    let txt = tokio::fs::read_to_string(path)
        .await
        .with_context(|| format!("reading {}", path.display()))?;
    let cfg: WorkspaceConfigFile = toml::from_str(&txt).context("parsing config.toml")?;
    Ok(cfg.worktree.and_then(|w| w.bootstrap))
}

fn build_bootstrap_steps(
    spec: &BootstrapCommandSpec,
    worktree: &Worktree,
) -> Result<Vec<BootstrapStep>> {
    let mut steps = Vec::new();
    match spec {
        BootstrapCommandSpec::Script(script) => {
            let trimmed = script.trim();
            if trimmed.is_empty() {
                return Ok(steps);
            }
            let path = resolve_worktree_path(&worktree.root_path, trimmed);
            steps.push(BootstrapStep {
                label: trimmed.to_string(),
                kind: BootstrapStepKind::Script {
                    path,
                    original: trimmed.to_string(),
                },
            });
        }
        BootstrapCommandSpec::Commands(commands) => {
            for cmd in commands {
                let trimmed = cmd.trim();
                if trimmed.is_empty() {
                    continue;
                }
                steps.push(BootstrapStep {
                    label: trimmed.to_string(),
                    kind: BootstrapStepKind::Command {
                        command: trimmed.to_string(),
                    },
                });
            }
        }
    }
    Ok(steps)
}

async fn run_bootstrap_step(
    step: &BootstrapStep,
    workspace: &Workspace,
    worktree: &Worktree,
    timeout: Duration,
) -> Result<BootstrapCommandResult> {
    let mut cmd = match &step.kind {
        BootstrapStepKind::Script { path, .. } => command_for_script(path),
        BootstrapStepKind::Command { command } => command_for_shell(command),
    };

    cmd.current_dir(&worktree.root_path)
        .stdin(Stdio::null())
        .env("CTX_WORKSPACE_ROOT", &workspace.root_path)
        .env("CTX_WORKTREE_ROOT", &worktree.root_path)
        .env("CTX_WORKTREE_ID", worktree.id.0.to_string())
        .env(
            "CTX_BRANCH_NAME",
            worktree.git_branch.clone().unwrap_or_default(),
        )
        .env("CTX_BASE_COMMIT_SHA", &worktree.base_commit_sha);

    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("spawning bootstrap command")?;

    let mut stdout = child.stdout.take().context("reading stdout")?;
    let mut stderr = child.stderr.take().context("reading stderr")?;

    let stdout_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        stdout.read_to_end(&mut buf).await?;
        Ok::<Vec<u8>, std::io::Error>(buf)
    });

    let stderr_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        stderr.read_to_end(&mut buf).await?;
        Ok::<Vec<u8>, std::io::Error>(buf)
    });

    let mut timed_out = false;
    let status = match tokio::time::timeout(timeout, child.wait()).await {
        Ok(status) => status.context("waiting on bootstrap command")?,
        Err(_) => {
            timed_out = true;
            let _ = child.kill().await;
            child
                .wait()
                .await
                .context("waiting on killed bootstrap command")?
        }
    };

    let stdout = stdout_task.await.unwrap_or_else(|_| Ok(Vec::new()))?;
    let stderr = stderr_task.await.unwrap_or_else(|_| Ok(Vec::new()))?;

    Ok(BootstrapCommandResult {
        status: status.code(),
        stdout: String::from_utf8_lossy(&stdout).to_string(),
        stderr: String::from_utf8_lossy(&stderr).to_string(),
        timed_out,
    })
}

fn command_for_shell(command: &str) -> Command {
    if cfg!(windows) {
        let mut cmd = Command::new("cmd");
        cmd.arg("/C").arg(command);
        cmd
    } else {
        let mut cmd = Command::new("sh");
        cmd.arg("-lc").arg(command);
        cmd
    }
}

fn command_for_script(path: &Path) -> Command {
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    if cfg!(windows) && ext.eq_ignore_ascii_case("ps1") {
        let mut cmd = Command::new("powershell");
        cmd.arg("-NoProfile")
            .arg("-ExecutionPolicy")
            .arg("Bypass")
            .arg("-File")
            .arg(path);
        return cmd;
    }
    if ext.eq_ignore_ascii_case("sh") {
        let mut cmd = Command::new("bash");
        cmd.arg(path);
        return cmd;
    }
    Command::new(path)
}

fn resolve_worktree_path(worktree_root: &str, raw: &str) -> PathBuf {
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        path
    } else {
        Path::new(worktree_root).join(path)
    }
}

fn detect_bootstrap_config_path(worktree: &Worktree, workspace: &Workspace) -> Option<PathBuf> {
    let worktree_cfg_path = Path::new(&worktree.root_path).join(CONFIG_REL_PATH);
    if worktree_cfg_path.exists() {
        return Some(worktree_cfg_path);
    }
    let workspace_cfg_path = Path::new(&workspace.root_path).join(CONFIG_REL_PATH);
    if workspace_cfg_path.exists() {
        return Some(workspace_cfg_path);
    }
    None
}

fn append_output(log: &mut String, output: &str, label: &str) {
    let trimmed = output.trim_end_matches('\n');
    if trimmed.is_empty() {
        return;
    }
    log.push_str(&format!("[{label}]\n"));
    log.push_str(trimmed);
    log.push('\n');
}

fn truncate_log(input: &str) -> (String, bool) {
    if input.len() <= MAX_LOG_BYTES {
        return (input.to_string(), false);
    }
    let mut out = input.chars().take(MAX_LOG_BYTES).collect::<String>();
    out.push_str("\n...(truncated)\n");
    (out, true)
}

async fn write_bootstrap_log(
    state: &AppState,
    worktree_id: WorktreeId,
    contents: &str,
) -> Result<PathBuf> {
    let dir = logs::logs_dir(&state.data_root).join("worktree-bootstrap");
    tokio::fs::create_dir_all(&dir)
        .await
        .context("creating bootstrap log dir")?;
    let path = dir.join(format!("worktree-bootstrap-{}.log", worktree_id.0));
    tokio::fs::write(&path, contents)
        .await
        .with_context(|| format!("writing bootstrap log to {}", path.display()))?;
    Ok(path)
}

async fn update_bootstrap_result(state: &AppState, update: WorktreeBootstrapResultUpdate) {
    let _ = state.store.update_worktree_bootstrap_result(update).await;
}

async fn emit_failure_notice(
    state: &AppState,
    workspace_id: ctx_core::ids::WorkspaceId,
    notice: WorktreeBootstrapNotice,
) {
    state
        .workspace_catchup
        .publish_worktree_bootstrap(workspace_id, notice)
        .await;
}
