use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use base64::Engine;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader, BufWriter};
use tokio::process::{Child, Command};
use tokio::sync::{broadcast, mpsc, oneshot, watch, Mutex};
use tokio::time::{timeout, Duration};
use uuid::Uuid;

use ctx_core::models::SessionEventType;

use crate::adapters::{
    ProviderAdapter, ProviderCapabilities, ProviderHealth, ProviderProcessInfo,
    ProviderRestartMode, ProviderStatus, RunHandle, TurnInput,
};
use crate::container_exec::{build_container_exec_command, container_exec_spec};
use crate::events::NormalizedEvent;

const CRP_VERSION: u32 = 1;
const DEFAULT_CTX_MCP_TOOL_TIMEOUT_SECS: u64 = 2 * 60 * 60;
const CRP_MODEL_PROBE_TIMEOUT: Duration = Duration::from_secs(10);
const CRP_MODEL_PROBE_TIMEOUT_CONTAINER: Duration = Duration::from_secs(45);
const CRP_RUNTIME_LAUNCH_PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const CRP_RUNTIME_LAUNCH_PROBE_TIMEOUT_CONTAINER: Duration = Duration::from_secs(5);
const CRP_AUTH_EVENT_FORWARD_TIMEOUT: Duration = Duration::from_secs(60 * 10);
const CODEX_CRP_DUMP_CODEX_EVENTS_ENV: &str = "CODEX_CRP_DUMP_CODEX_EVENTS_PATH";
const CODEX_CRP_DUMP_CRP_EVENTS_ENV: &str = "CODEX_CRP_DUMP_CRP_EVENTS_PATH";

#[derive(Clone)]
pub struct Tier1CrpAdapter {
    id: String,
    command: String,
    pool: Arc<CrpSessionPool>,
}

enum BundledLinuxRewrite {
    NotBundledPath,
    AlreadyLinux,
    Candidate(String),
}

fn bundled_linux_candidate_for_marker(path: &str, marker: &str) -> BundledLinuxRewrite {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return BundledLinuxRewrite::NotBundledPath;
    }
    let sep = if trimmed.contains('\\') { '\\' } else { '/' };
    let needle = format!("{sep}{marker}{sep}");
    let Some(idx) = trimmed.find(&needle) else {
        return BundledLinuxRewrite::NotBundledPath;
    };
    let prefix = &trimmed[..idx];
    let bundles_segment = format!("{sep}bundles");
    let prefix_path = Path::new(prefix);
    let looks_like_bundle_root = prefix == "bundles"
        || prefix.ends_with(&bundles_segment)
        || prefix_path.join("manifest.json").is_file()
        || prefix_path.join("runtime_lock.v2.json").is_file();
    if !looks_like_bundle_root {
        return BundledLinuxRewrite::NotBundledPath;
    }
    let rest = &trimmed[idx + needle.len()..];
    let mut parts = rest.split(sep);
    let Some(id) = parts.next() else {
        return BundledLinuxRewrite::NotBundledPath;
    };
    let Some(os) = parts.next() else {
        return BundledLinuxRewrite::NotBundledPath;
    };
    let Some(arch) = parts.next() else {
        return BundledLinuxRewrite::NotBundledPath;
    };
    if os == "linux" {
        return BundledLinuxRewrite::AlreadyLinux;
    }
    let tail: String = parts.collect::<Vec<_>>().join(&sep.to_string());
    let candidate = format!("{prefix}{needle}{id}{sep}linux{sep}{arch}{sep}{tail}");
    BundledLinuxRewrite::Candidate(candidate)
}

fn bundled_linux_candidate(path: &str) -> BundledLinuxRewrite {
    let providers = bundled_linux_candidate_for_marker(path, "providers");
    if !matches!(providers, BundledLinuxRewrite::NotBundledPath) {
        return providers;
    }
    bundled_linux_candidate_for_marker(path, "runtimes")
}

#[derive(Debug, Deserialize)]
struct BundledManifestRuntimeEntry {
    id: String,
    os: String,
    arch: String,
    root: String,
    bin: String,
}

#[derive(Debug, Deserialize)]
struct BundledManifestForRuntimeRewrite {
    #[serde(default)]
    runtimes: Vec<BundledManifestRuntimeEntry>,
}

fn bundles_root_for_path(path: &Path) -> Option<PathBuf> {
    path.ancestors().find_map(|ancestor| {
        let looks_like_bundle_root = ancestor
            .file_name()
            .is_some_and(|name| name == std::ffi::OsStr::new("bundles"))
            || ancestor.join("manifest.json").is_file()
            || ancestor.join("runtime_lock.v2.json").is_file();
        if looks_like_bundle_root {
            Some(ancestor.to_path_buf())
        } else {
            None
        }
    })
}

fn resolve_runtime_linux_path_from_manifest(path: &str) -> Option<String> {
    let source_path = Path::new(path);
    let bundles_root = bundles_root_for_path(source_path)?;
    let rel = source_path.strip_prefix(&bundles_root).ok()?;
    let mut parts = rel.components();
    if parts.next()?.as_os_str() != std::ffi::OsStr::new("runtimes") {
        return None;
    }
    let runtime_id = parts.next()?.as_os_str().to_string_lossy().to_string();
    let _host_os = parts.next()?;
    let arch = parts.next()?.as_os_str().to_string_lossy().to_string();
    let source_bin_name = source_path.file_name()?.to_string_lossy().to_string();

    let manifest_path = bundles_root.join("manifest.json");
    let raw = std::fs::read_to_string(&manifest_path).ok()?;
    let manifest: BundledManifestForRuntimeRewrite = serde_json::from_str(&raw).ok()?;
    let runtime = manifest
        .runtimes
        .iter()
        .find(|entry| entry.id == runtime_id && entry.os == "linux" && entry.arch == arch)?;
    let root = Path::new(&runtime.root);
    let runtime_root = if root.is_absolute() {
        root.to_path_buf()
    } else {
        bundles_root.join(root)
    };
    let candidate = runtime_root.join(&runtime.bin);
    if !candidate.exists() {
        return None;
    }
    if candidate
        .file_name()
        .is_some_and(|name| name.to_string_lossy() == source_bin_name)
    {
        return Some(candidate.to_string_lossy().to_string());
    }
    None
}

fn rewrite_bundled_path_for_linux(path: &str) -> Result<String> {
    match bundled_linux_candidate(path) {
        BundledLinuxRewrite::NotBundledPath | BundledLinuxRewrite::AlreadyLinux => {
            Ok(path.to_string())
        }
        BundledLinuxRewrite::Candidate(candidate) => {
            if std::path::Path::new(&candidate).exists() {
                Ok(candidate)
            } else if let Some(runtime_candidate) = resolve_runtime_linux_path_from_manifest(path) {
                Ok(runtime_candidate)
            } else {
                anyhow::bail!(
                    "missing linux bundled path for container execution: source='{}' expected='{}'",
                    path,
                    candidate
                );
            }
        }
    }
}

fn rewrite_bundled_paths_in_shell_command(
    raw: &str,
    env: &HashMap<String, String>,
) -> Result<String> {
    let tokens = shlex::split(raw).ok_or_else(|| {
        anyhow::anyhow!("invalid shell command in --acp-command: unmatched quote")
    })?;
    if tokens.is_empty() {
        return Ok(raw.to_string());
    }

    let mut rewritten = Vec::with_capacity(tokens.len());
    for token in tokens {
        rewritten.push(rewrite_bundled_path_for_linux(&token)?);
    }

    let first_is_js_entrypoint = rewritten
        .first()
        .and_then(|command| std::path::Path::new(command).extension())
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("js"));
    if first_is_js_entrypoint {
        let node = resolve_node_binary_from_env(env)
            .ok_or_else(|| anyhow::anyhow!("could not resolve node binary for JS ACP command"))?;
        let rewritten_node = rewrite_bundled_path_for_linux(&node)?;
        rewritten.insert(0, rewritten_node);
    }

    shlex::try_join(rewritten.iter().map(String::as_str))
        .map_err(|err| anyhow::anyhow!("failed to quote --acp-command after rewrite: {err}"))
}

fn rewrite_container_args_for_linux(
    args: &[String],
    env: &HashMap<String, String>,
) -> Result<Vec<String>> {
    let mut out = Vec::with_capacity(args.len());
    let mut idx = 0;
    while idx < args.len() {
        let arg = &args[idx];
        if arg == "--acp-command" {
            out.push(arg.clone());
            if let Some(acp_command) = args.get(idx + 1) {
                out.push(rewrite_bundled_paths_in_shell_command(acp_command, env)?);
                idx += 2;
                continue;
            }
            idx += 1;
            continue;
        }
        out.push(rewrite_bundled_path_for_linux(arg)?);
        idx += 1;
    }
    Ok(out)
}

fn resolve_node_binary_from_env(env: &HashMap<String, String>) -> Option<String> {
    let path_value = env
        .get("PATH")
        .cloned()
        .or_else(|| std::env::var("PATH").ok())?;
    let executable_names: &[&str] = if cfg!(windows) {
        &["node.exe", "node"]
    } else {
        &["node"]
    };
    for dir in std::env::split_paths(std::ffi::OsStr::new(&path_value)) {
        for name in executable_names {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate.to_string_lossy().to_string());
            }
        }
    }
    None
}

fn rewrite_container_command_for_linux(
    command: &str,
    args: &[String],
    env: &HashMap<String, String>,
) -> Result<(String, Vec<String>)> {
    let rewritten_command = rewrite_bundled_path_for_linux(command)?;
    let rewritten_args = rewrite_container_args_for_linux(args, env)?;
    let is_js_entrypoint = std::path::Path::new(&rewritten_command)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("js"));
    if !is_js_entrypoint {
        return Ok((rewritten_command, rewritten_args));
    }
    let Some(node_binary) = resolve_node_binary_from_env(env) else {
        return Ok((rewritten_command, rewritten_args));
    };
    let rewritten_node = rewrite_bundled_path_for_linux(&node_binary)?;
    let mut final_args = Vec::with_capacity(rewritten_args.len() + 1);
    final_args.push(rewritten_command);
    final_args.extend(rewritten_args);
    Ok((rewritten_node, final_args))
}

fn resolve_explicit_command_path(command: &str) -> Option<PathBuf> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Intentionally do not consult PATH for determinism. Providers should be configured
    // with a known absolute path (e.g. managed install path or bundled asset path).
    let p = Path::new(trimmed);
    // On Windows, paths may be expressed with either '\\' or '/' separators.
    if p.is_absolute() || trimmed.contains('/') || trimmed.contains('\\') {
        return if p.exists() {
            Some(p.to_path_buf())
        } else {
            None
        };
    }
    None
}

impl Tier1CrpAdapter {
    fn new(id: &str, command: &str, args: Vec<String>) -> Self {
        let agent = CrpAgentConfig {
            provider_id: id.to_string(),
            command: command.to_string(),
            args: args.clone(),
        };
        Self {
            id: id.to_string(),
            command: command.to_string(),
            pool: Arc::new(CrpSessionPool::new(agent)),
        }
    }

    pub fn from_raw(id: &str, command: String, args: Vec<String>) -> Self {
        Self::new(id, &command, args)
    }

    pub fn codex() -> Self {
        Self::new("codex", "codex", vec![])
    }

    pub fn claude() -> Self {
        Self::new("claude-crp", "claude-crp", vec![])
    }
}

#[async_trait]
impl ProviderAdapter for Tier1CrpAdapter {
    async fn inspect(&self) -> Result<ProviderStatus> {
        let detected_path = resolve_explicit_command_path(&self.command);
        let installed = detected_path.is_some();
        let mut diagnostics = Vec::new();
        if !installed {
            diagnostics.push(format!(
                "CRP runtime executable not found: {}",
                self.command
            ));
        }

        Ok(ProviderStatus {
            provider_id: self.id.clone(),
            installed,
            detected_path: detected_path.map(|p| p.to_string_lossy().to_string()),
            version: None,
            capabilities: if installed {
                Some(default_caps(&self.id))
            } else {
                None
            },
            health: if installed {
                ProviderHealth::Ok
            } else {
                ProviderHealth::Missing
            },
            diagnostics,
            details: HashMap::new(),
        })
    }

    async fn run(
        &self,
        input: TurnInput,
        workdir: PathBuf,
        env: HashMap<String, String>,
        event_sink: mpsc::Sender<NormalizedEvent>,
    ) -> Result<RunHandle> {
        // Fail fast with a clear error if the runtime isn't available.
        // `inspect()` already checks this, but some call paths can attempt runs even after a stale status.
        if resolve_explicit_command_path(&self.command).is_none() {
            anyhow::bail!("CRP runtime executable not found: {}", self.command);
        }
        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
        let (done_tx, done_rx) = oneshot::channel::<()>();
        let pool = Arc::clone(&self.pool);
        let error_sink = event_sink.clone();
        let join = tokio::spawn(async move {
            let session_key = env
                .get("CTX_SESSION_ID")
                .cloned()
                .unwrap_or_else(|| Uuid::new_v4().to_string());
            let request = CrpPromptRequest {
                session_key,
                input,
                workdir,
                env,
                event_sink,
                cancel_rx,
            };
            if let Err(err) = pool.prompt(request).await {
                let _ = error_sink
                    .send(NormalizedEvent {
                        event_type: SessionEventType::Error,
                        payload_json: json!({ "message": err.to_string() }),
                    })
                    .await;
            }
            let _ = done_tx.send(());
        });
        let abort = join.abort_handle();

        Ok(RunHandle {
            done: done_rx,
            cancel: Some(cancel_tx),
            abort: Some(abort),
        })
    }

    async fn cancel(&self, mut handle: RunHandle) -> Result<()> {
        if let Some(cancel) = handle.cancel.take() {
            let _ = cancel.send(());
        }
        let done = tokio::time::timeout(std::time::Duration::from_secs(2), &mut handle.done).await;
        if done.is_err() {
            if let Some(abort) = handle.abort.take() {
                abort.abort();
            }
        }
        Ok(())
    }

    async fn list_processes(&self) -> Vec<ProviderProcessInfo> {
        self.pool.list_processes().await
    }

    async fn restart(&self, reason: &str, mode: ProviderRestartMode) -> Result<()> {
        self.pool.restart(reason, mode).await
    }

    async fn has_live_session(&self, session_key: &str) -> bool {
        self.pool.has_session(session_key).await
    }

    async fn authenticate_session(
        &self,
        session_key: String,
        workdir: PathBuf,
        env: HashMap<String, String>,
        method_id: Option<String>,
        event_sink: mpsc::Sender<NormalizedEvent>,
    ) -> Result<()> {
        let session = self
            .pool
            .get_or_create_session(&session_key, &workdir, &env)
            .await?;
        let mut rx = session.process.events.subscribe();
        let mut stderr_rx = session.process.stderr_lines.subscribe();
        let mut shutdown_rx = session.process.shutdown.subscribe();
        let auth_session_key = session_key.clone();
        if !session.opened.load(Ordering::SeqCst) && !session.opening.load(Ordering::SeqCst) {
            let config = build_crp_session_config(&env, &workdir);
            let provider_session_id = env
                .get("CTX_PROVIDER_SESSION_REF")
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty());
            session.opening.store(true, Ordering::SeqCst);
            session
                .process
                .send(CrpCommand::SessionOpen {
                    session_id: Some(session_key.clone()),
                    provider_session_id,
                    config: Some(config),
                })
                .await?;
        }
        session
            .process
            .send(CrpCommand::SessionAuthenticate {
                session_id: Some(session_key),
                method_id,
            })
            .await?;
        let session_for_events = Arc::clone(&session);
        tokio::spawn(async move {
            let deadline = tokio::time::Instant::now() + CRP_AUTH_EVENT_FORWARD_TIMEOUT;
            let mut last_seq = 0u64;
            let mut tool_output_cache: HashMap<String, String> = HashMap::new();
            let mut tool_input_cache: HashMap<String, CachedToolInput> = HashMap::new();
            loop {
                let now = tokio::time::Instant::now();
                if now >= deadline {
                    break;
                }
                let timeout_remaining = deadline.saturating_duration_since(now);
                tokio::select! {
                    _ = shutdown_rx.changed() => {
                        break;
                    }
                    _ = tokio::time::sleep(timeout_remaining) => {
                        break;
                    }
                    recv = rx.recv() => {
                        match recv {
                            Ok(env) => {
                                if !event_matches_session(&env.event, &auth_session_key) {
                                    continue;
                                }
                                if env.seq <= last_seq {
                                    continue;
                                }
                                last_seq = env.seq;
                                if matches!(&env.event, CrpEvent::SessionOpened { .. }) {
                                    session_for_events.opened.store(true, Ordering::SeqCst);
                                    session_for_events.opening.store(false, Ordering::SeqCst);
                                }
                                let auth_terminal_event = matches!(
                                    &env.event,
                                    CrpEvent::SessionNotice { code, .. }
                                    if code == "auth_complete"
                                        || code == "auth_completed"
                                        || code == "auth_success"
                                        || code == "authenticated"
                                        || code == "auth_failed"
                                        || code == "auth_error"
                                );
                                if auth_terminal_event {
                                    session_for_events.opening.store(false, Ordering::SeqCst);
                                }
                                let mapped = map_crp_event(
                                    env.event,
                                    env.channel,
                                    env.seq,
                                    &mut tool_output_cache,
                                    &mut tool_input_cache,
                                );
                                for event in mapped.events {
                                    if event_sink.send(event).await.is_err() {
                                        return;
                                    }
                                }
                                if auth_terminal_event {
                                    break;
                                }
                            }
                            Err(broadcast::error::RecvError::Lagged(_)) => {
                                let _ = event_sink
                                    .send(NormalizedEvent {
                                        event_type: SessionEventType::Notice,
                                        payload_json: json!({
                                            "kind": "session_gap",
                                            "reason": "crp_receiver_lagged",
                                        }),
                                    })
                                    .await;
                            }
                            Err(broadcast::error::RecvError::Closed) => {
                                break;
                            }
                        }
                    }
                    stderr = stderr_rx.recv() => {
                        match stderr {
                            Ok(line) => {
                                if let Some(auth_url) = extract_auth_url_from_stderr_line(&line) {
                                    if event_sink
                                        .send(NormalizedEvent {
                                            event_type: SessionEventType::Notice,
                                            payload_json: json!({
                                                "kind": "auth_url_stderr",
                                                "auth_url": auth_url,
                                                "source": "crp_stderr",
                                            }),
                                        })
                                        .await
                                        .is_err()
                                    {
                                        return;
                                    }
                                }
                                if let Some(message) = extract_auth_error_from_stderr_line(&line) {
                                    let _ = event_sink
                                        .send(NormalizedEvent {
                                            event_type: SessionEventType::Error,
                                            payload_json: json!({
                                                "message": message,
                                                "source": "crp_stderr",
                                            }),
                                        })
                                        .await;
                                    break;
                                }
                            }
                            Err(broadcast::error::RecvError::Lagged(_)) => {}
                            Err(broadcast::error::RecvError::Closed) => {
                                break;
                            }
                        }
                    }
                }
            }
        });
        Ok(())
    }
}

fn default_caps(id: &str) -> ProviderCapabilities {
    ProviderCapabilities {
        stream_events: true,
        stream_format: "crp-jsonl".into(),
        has_turn_boundaries: true,
        has_tool_call_ids: true,
        has_file_change_events: false,
        has_command_events: false,
        supports_resume: matches!(id, "codex"),
        supports_stable_session_id: true,
        supports_fork_or_rewind: false,
        supports_headless: true,
        supports_server_mode: false,
        supports_interactive_tui: false,
        supports_private_state_dir: false,
        supports_sandbox_flags: false,
        supports_approval_flags: false,
        notes: vec![],
    }
}

#[derive(Debug, Clone)]
struct CrpAgentConfig {
    provider_id: String,
    command: String,
    args: Vec<String>,
}

struct CrpSessionPool {
    agent: CrpAgentConfig,
    sessions: Mutex<HashMap<String, Arc<CrpSession>>>,
    active_prompts: Arc<StdMutex<HashSet<String>>>,
}

impl CrpSessionPool {
    fn new(agent: CrpAgentConfig) -> Self {
        Self {
            agent,
            sessions: Mutex::new(HashMap::new()),
            active_prompts: Arc::new(StdMutex::new(HashSet::new())),
        }
    }

    async fn list_processes(&self) -> Vec<ProviderProcessInfo> {
        let sessions = self.sessions.lock().await;
        let mut out = Vec::new();
        for (session_id, session) in sessions.iter() {
            if let Some(pid) = session.process.pid().await {
                out.push(ProviderProcessInfo {
                    provider_id: self.agent.provider_id.clone(),
                    pid,
                    label: Some(session_id.clone()),
                });
            }
        }
        out
    }

    async fn has_session(&self, session_key: &str) -> bool {
        let sessions = self.sessions.lock().await;
        sessions.contains_key(session_key)
    }

    async fn restart(&self, reason: &str, mode: ProviderRestartMode) -> Result<()> {
        match mode {
            ProviderRestartMode::Immediate => {
                self.restart_immediate(reason).await;
                Ok(())
            }
            ProviderRestartMode::Drain => {
                self.restart_drain(reason).await;
                Ok(())
            }
        }
    }

    async fn restart_immediate(&self, reason: &str) {
        let sessions = {
            let mut guard = self.sessions.lock().await;
            guard.drain().collect::<Vec<_>>()
        };
        for (session_key, session) in sessions {
            session
                .process
                .shutdown(&format!("{reason}: immediate restart ({session_key})"))
                .await;
        }
    }

    async fn restart_drain(&self, reason: &str) {
        let active = self.active_prompt_snapshot();
        let sessions_to_kill = {
            let mut guard = self.sessions.lock().await;
            let mut to_kill = Vec::new();
            for (session_key, session) in guard.iter() {
                session.draining.store(true, Ordering::SeqCst);
                if !active.contains(session_key) {
                    to_kill.push((session_key.clone(), Arc::clone(session)));
                }
            }
            for (session_key, _) in &to_kill {
                guard.remove(session_key);
            }
            to_kill
        };

        for (session_key, session) in sessions_to_kill {
            session
                .process
                .shutdown(&format!("{reason}: drain idle ({session_key})"))
                .await;
        }
    }

    fn active_prompt_snapshot(&self) -> HashSet<String> {
        let Ok(guard) = self.active_prompts.lock() else {
            return HashSet::new();
        };
        guard.iter().cloned().collect()
    }

    async fn drain_session_if_needed(&self, session_key: &str, session: &Arc<CrpSession>) {
        if !session.draining.load(Ordering::SeqCst) {
            return;
        }
        let should_remove = {
            let mut guard = self.sessions.lock().await;
            let Some(current) = guard.get(session_key) else {
                return;
            };
            if !Arc::ptr_eq(current, session) {
                return;
            }
            guard.remove(session_key);
            true
        };
        if should_remove {
            session
                .process
                .shutdown(&format!("drain completed ({session_key})"))
                .await;
        }
    }

    async fn prompt(&self, req: CrpPromptRequest) -> Result<()> {
        let _guard =
            ActivePromptGuard::new(Arc::clone(&self.active_prompts), req.session_key.clone())?;
        let session = self
            .get_or_create_session(&req.session_key, &req.workdir, &req.env)
            .await?;

        let turn_id = format!("crp-{}", Uuid::new_v4());
        let mut rx = session.process.events.subscribe();
        let mut shutdown_rx = session.process.shutdown.subscribe();
        let shutdown_reason = {
            let reason = shutdown_rx.borrow().clone();
            reason
        };
        if let Some(reason) = shutdown_reason {
            let _ = req
                .event_sink
                .send(NormalizedEvent {
                    event_type: SessionEventType::TurnInterrupted,
                    payload_json: json!({
                        "reason": reason,
                        "provider_cancelled": true,
                    }),
                })
                .await;
            self.drain_session_if_needed(&req.session_key, &session)
                .await;
            return Ok(());
        }

        if !session.opened.load(Ordering::SeqCst) && !session.opening.load(Ordering::SeqCst) {
            let config = build_crp_session_config(&req.env, &req.workdir);
            let provider_session_id = req
                .env
                .get("CTX_PROVIDER_SESSION_REF")
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty());
            session.opening.store(true, Ordering::SeqCst);
            session
                .process
                .send(CrpCommand::SessionOpen {
                    session_id: Some(req.session_key.clone()),
                    provider_session_id,
                    config: Some(config),
                })
                .await?;
        }
        validate_provider_slash_command_support(&self.agent.provider_id, &req.input.content)?;
        match parse_native_crp_slash_command_for_provider(
            &self.agent.provider_id,
            &req.input.content,
        ) {
            Some(CrpSlashCommand::Compact) => {
                session
                    .process
                    .send(CrpCommand::SessionCompact {
                        session_id: Some(req.session_key.clone()),
                        turn_id: Some(turn_id.clone()),
                    })
                    .await?;
            }
            Some(CrpSlashCommand::Undo) => {
                session
                    .process
                    .send(CrpCommand::SessionUndo {
                        session_id: Some(req.session_key.clone()),
                        turn_id: Some(turn_id.clone()),
                    })
                    .await?;
            }
            Some(CrpSlashCommand::Review { instructions }) => {
                session
                    .process
                    .send(CrpCommand::SessionReview {
                        session_id: Some(req.session_key.clone()),
                        turn_id: Some(turn_id.clone()),
                        instructions,
                    })
                    .await?;
            }
            None => {
                let items = build_prompt_items(&req.input, &req.workdir, &req.env).await?;
                let (model, reasoning_effort) = req
                    .input
                    .model_id
                    .as_deref()
                    .map(split_model_id_and_effort)
                    .unwrap_or((None, None));
                session
                    .process
                    .send(CrpCommand::SessionPrompt {
                        session_id: Some(req.session_key.clone()),
                        turn_id: Some(turn_id.clone()),
                        items: Some(items),
                        prompt: Some(req.input.content.clone()),
                        model,
                        reasoning_effort,
                        cwd: Some(req.workdir.clone()),
                    })
                    .await?;
            }
        }
        let mut last_seq = 0u64;
        let mut tool_output_cache: HashMap<String, String> = HashMap::new();
        let mut tool_input_cache: HashMap<String, CachedToolInput> = HashMap::new();
        // Debugging aid: when set, dump normalized session events (post-CRP mapping) to this file.
        // This lets us diff: raw Codex dump -> CRP stdout -> ctx-normalized events -> web UI.
        let dump_norm_path = std::env::var("CTX_CRP_DUMP_NORMALIZED_EVENTS_PATH")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let mut dump_norm_file = dump_norm_path.as_deref().and_then(|path| {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .ok()
        });
        let mut cancel_rx = req.cancel_rx;
        loop {
            tokio::select! {
                _ = &mut cancel_rx => {
                    let _ = session.process.send(CrpCommand::SessionCancel {
                        session_id: Some(req.session_key.clone()),
                        turn_id: Some(turn_id.clone()),
                    }).await;
                    break;
                }
                shutdown = shutdown_rx.changed() => {
                    let reason = match shutdown {
                        Ok(()) => {
                            let current = shutdown_rx.borrow().clone();
                            current.unwrap_or_else(|| "crp_shutdown".to_string())
                        }
                        Err(_) => "crp_shutdown".to_string(),
                    };
                    let _ = req
                        .event_sink
                        .send(NormalizedEvent {
                            event_type: SessionEventType::TurnInterrupted,
                            payload_json: json!({
                                "reason": reason,
                                "provider_cancelled": true,
                            }),
                        })
                        .await;
                    break;
                }
                recv = rx.recv() => {
                    match recv {
                        Ok(env) => {
                            if !event_matches_session(&env.event, &req.session_key) {
                                continue;
                            }
                            if let Some(event_turn_id) = event_turn_id(&env.event) {
                                if event_turn_id != turn_id {
                                    continue;
                                }
                            }
                            if env.seq <= last_seq {
                                continue;
                            }
                            last_seq = env.seq;
                            if matches!(&env.event, CrpEvent::SessionOpened { .. }) {
                                session.opened.store(true, Ordering::SeqCst);
                                session.opening.store(false, Ordering::SeqCst);
                            }
                            let auth_required = matches!(
                                &env.event,
                                CrpEvent::SessionNotice { code, .. } if code == "auth_required"
                            );
                            if auth_required {
                                session.opening.store(false, Ordering::SeqCst);
                            }
                            let mapped = map_crp_event(
                                env.event,
                                env.channel,
                                env.seq,
                                &mut tool_output_cache,
                                &mut tool_input_cache,
                            );
                            for event in mapped.events {
                                if let Some(f) = dump_norm_file.as_mut() {
                                    // Best-effort only; never fail the turn because debug dumping failed.
                                    let _ = writeln!(
                                        f,
                                        "{}",
                                        json!({
                                            "session_key": req.session_key,
                                            "turn_id": turn_id,
                                            "crp_seq": env.seq,
                                            "event_type": format!("{:?}", event.event_type),
                                            "payload_json": event.payload_json,
                                        })
                                    );
                                }
                                let _ = req.event_sink.send(event).await;
                            }
                            if auth_required {
                                let _ = req
                                    .event_sink
                                    .send(NormalizedEvent {
                                        event_type: SessionEventType::TurnInterrupted,
                                        payload_json: json!({
                                            "reason": "auth_required",
                                        }),
                                    })
                                    .await;
                                break;
                            }
                            if mapped.done {
                                break;
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            let payload = json!({
                                "kind": "session_gap",
                                "reason": "crp_receiver_lagged",
                            });
                            let _ = req
                                .event_sink
                                .send(NormalizedEvent {
                                    event_type: SessionEventType::Notice,
                                    payload_json: payload,
                                })
                                .await;
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
            }
        }

        self.drain_session_if_needed(&req.session_key, &session)
            .await;
        Ok(())
    }

    async fn get_or_create_session(
        &self,
        session_key: &str,
        workdir: &PathBuf,
        env: &HashMap<String, String>,
    ) -> Result<Arc<CrpSession>> {
        let drained = {
            let mut sessions = self.sessions.lock().await;
            if let Some(existing) = sessions.get(session_key) {
                if !existing.draining.load(Ordering::SeqCst) {
                    return Ok(Arc::clone(existing));
                }
                sessions.remove(session_key)
            } else {
                None
            }
        };
        if let Some(existing) = drained {
            existing
                .process
                .shutdown(&format!("drain replace ({session_key})"))
                .await;
        }

        let process = CrpProcess::spawn(&self.agent, workdir, env)
            .await
            .with_context(|| format!("spawning CRP runtime {}", self.agent.command))?;
        let session = Arc::new(CrpSession {
            process,
            opened: AtomicBool::new(false),
            opening: AtomicBool::new(false),
            draining: AtomicBool::new(false),
        });
        let mut sessions = self.sessions.lock().await;
        sessions.insert(session_key.to_string(), Arc::clone(&session));
        Ok(session)
    }
}

struct ActivePromptGuard {
    session_key: String,
    active_prompts: Arc<StdMutex<HashSet<String>>>,
}

impl ActivePromptGuard {
    fn new(active_prompts: Arc<StdMutex<HashSet<String>>>, session_key: String) -> Result<Self> {
        if let Ok(mut active) = active_prompts.lock() {
            if active.contains(&session_key) {
                anyhow::bail!("session {session_key} already has an active prompt");
            }
            active.insert(session_key.clone());
        }
        Ok(Self {
            session_key,
            active_prompts,
        })
    }
}

impl Drop for ActivePromptGuard {
    fn drop(&mut self) {
        if let Ok(mut active) = self.active_prompts.lock() {
            active.remove(&self.session_key);
        }
    }
}

struct CrpSession {
    process: Arc<CrpProcess>,
    opened: AtomicBool,
    opening: AtomicBool,
    draining: AtomicBool,
}

struct CrpPromptRequest {
    session_key: String,
    input: TurnInput,
    workdir: PathBuf,
    env: HashMap<String, String>,
    event_sink: mpsc::Sender<NormalizedEvent>,
    cancel_rx: oneshot::Receiver<()>,
}

#[derive(Debug, PartialEq, Eq)]
enum CrpSlashCommand {
    Compact,
    Undo,
    Review { instructions: Option<String> },
}

fn parse_crp_slash_command(content: &str) -> Option<CrpSlashCommand> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with('/') {
        return None;
    }
    let matches_command = |name: &str| {
        if !trimmed.starts_with(name) {
            return false;
        }
        trimmed
            .chars()
            .nth(name.len())
            .map(|ch| ch.is_whitespace())
            .unwrap_or(true)
    };
    if matches_command("/compact") {
        return Some(CrpSlashCommand::Compact);
    }
    if matches_command("/undo") {
        return Some(CrpSlashCommand::Undo);
    }
    if matches_command("/review") {
        let rest = trimmed["/review".len()..].trim();
        let instructions = if rest.is_empty() {
            None
        } else {
            Some(rest.to_string())
        };
        return Some(CrpSlashCommand::Review { instructions });
    }
    None
}

fn parse_native_crp_slash_command_for_provider(
    provider_id: &str,
    content: &str,
) -> Option<CrpSlashCommand> {
    if provider_id != "codex" {
        return None;
    }
    parse_crp_slash_command(content)
}

#[derive(Debug, PartialEq, Eq)]
enum ClaudeSlashCommandPolicy {
    Supported,
    Redundant(&'static str),
    Unsupported(&'static str),
}

fn extract_slash_command_name(content: &str) -> Option<String> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with('/') {
        return None;
    }
    let token = trimmed.split_whitespace().next()?;
    let normalized = token.trim_start_matches('/').trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return None;
    }
    Some(normalized)
}

fn classify_claude_slash_command(name: &str) -> ClaudeSlashCommandPolicy {
    if name.starts_with("mcp__") {
        return ClaudeSlashCommandPolicy::Unsupported("MCP prompt commands are intentionally out of scope in ctx right now.");
    }

    match name {
        "allowed-tools"
        | "clear"
        | "config"
        | "continue"
        | "diff"
        | "exit"
        | "login"
        | "logout"
        | "model"
        | "new"
        | "permissions"
        | "quit"
        | "rename"
        | "reset"
        | "resume"
        | "sandbox"
        | "settings"
        | "status"
        | "tasks" => {
            ClaudeSlashCommandPolicy::Redundant("ctx handles this workflow outside Claude slash commands.")
        }
        "add-dir"
        | "agents"
        | "android"
        | "app"
        | "checkpoint"
        | "chrome"
        | "copy"
        | "desktop"
        | "export"
        | "extra-usage"
        | "fork"
        | "hooks"
        | "ide"
        | "install-github-app"
        | "install-slack-app"
        | "ios"
        | "keybindings"
        | "mcp"
        | "mobile"
        | "passes"
        | "plugin"
        | "privacy-settings"
        | "rc"
        | "reload-plugins"
        | "remote-control"
        | "remote-env"
        | "rewind"
        | "statusline"
        | "stickers"
        | "terminal-setup"
        | "theme"
        | "upgrade"
        | "vim" => ClaudeSlashCommandPolicy::Unsupported(
            "Claude Code exposes this command in TUI/native integrations, but ctx cannot wire it through the Claude Agent SDK path today.",
        ),
        _ => ClaudeSlashCommandPolicy::Supported,
    }
}

fn validate_provider_slash_command_support(
    provider_id: &str,
    content: &str,
) -> Result<()> {
    if provider_id != "claude-crp" {
        return Ok(());
    }
    let Some(name) = extract_slash_command_name(content) else {
        return Ok(());
    };
    match classify_claude_slash_command(&name) {
        ClaudeSlashCommandPolicy::Supported => Ok(()),
        ClaudeSlashCommandPolicy::Redundant(reason) => Err(anyhow!(
            "Claude command `/{name}` is intentionally not supported in ctx: {reason}"
        )),
        ClaudeSlashCommandPolicy::Unsupported(reason) => Err(anyhow!(
            "Claude command `/{name}` is not supported in ctx today: {reason}"
        )),
    }
}

struct CrpProcess {
    agent: CrpAgentConfig,
    child: Mutex<Child>,
    pid: AtomicU32,
    write_tx: mpsc::UnboundedSender<String>,
    events: broadcast::Sender<CrpEventEnvelope>,
    stderr_lines: broadcast::Sender<String>,
    shutdown: watch::Sender<Option<String>>,
}

struct CrpLogPaths {
    codex_events: PathBuf,
    crp_events: PathBuf,
    stderr: PathBuf,
}

impl CrpProcess {
    async fn spawn(
        agent: &CrpAgentConfig,
        workdir: &PathBuf,
        env: &HashMap<String, String>,
    ) -> Result<Arc<Self>> {
        let mut cmd = if let Some(spec) = container_exec_spec(env) {
            let (container_command, container_args) =
                rewrite_container_command_for_linux(&agent.command, &agent.args, env)?;
            build_container_exec_command(&spec, workdir, env, &container_command, &container_args)
        } else {
            let mut cmd = Command::new(&agent.command);
            cmd.args(&agent.args);
            cmd.current_dir(workdir);
            cmd
        };
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        let mut stderr_log_path: Option<PathBuf> = None;
        if let Some(paths) = crp_log_paths(env, &agent.provider_id) {
            if let Some(parent) = paths.stderr.parent() {
                if std::fs::create_dir_all(parent).is_ok() {
                    if !env.contains_key(CODEX_CRP_DUMP_CODEX_EVENTS_ENV) {
                        cmd.env(CODEX_CRP_DUMP_CODEX_EVENTS_ENV, &paths.codex_events);
                    }
                    if !env.contains_key(CODEX_CRP_DUMP_CRP_EVENTS_ENV) {
                        cmd.env(CODEX_CRP_DUMP_CRP_EVENTS_ENV, &paths.crp_events);
                    }
                    stderr_log_path = Some(paths.stderr);
                }
            }
        }
        apply_outer_process_env(&mut cmd, env);

        let mut child = cmd.spawn()?;
        let pid = child.id().unwrap_or(0);

        let stdin = child.stdin.take().context("capturing CRP stdin")?;
        let stdout = child.stdout.take().context("capturing CRP stdout")?;
        let stderr = child.stderr.take().context("capturing CRP stderr")?;

        let (write_tx, mut write_rx) = mpsc::unbounded_channel::<String>();
        let _writer = tokio::spawn(async move {
            let mut stdin = tokio::io::BufWriter::new(stdin);
            while let Some(line) = write_rx.recv().await {
                if stdin.write_all(line.as_bytes()).await.is_err() {
                    break;
                }
                if stdin.write_all(b"\n").await.is_err() {
                    break;
                }
                let _ = stdin.flush().await;
            }
        });

        let (events, _) = broadcast::channel(512);
        let (stderr_lines, _) = broadcast::channel(256);
        let (shutdown, _) = watch::channel::<Option<String>>(None);
        let process = Arc::new(Self {
            agent: agent.clone(),
            child: Mutex::new(child),
            pid: AtomicU32::new(pid),
            write_tx,
            events,
            stderr_lines,
            shutdown,
        });

        let stdout_process = Arc::clone(&process);
        tokio::spawn(async move {
            stdout_pump(stdout_process, stdout).await;
        });
        let stderr_process = Arc::clone(&process);
        tokio::spawn(async move {
            stderr_pump(stderr_process, stderr, stderr_log_path).await;
        });

        let monitor_process = Arc::clone(&process);
        tokio::spawn(async move {
            monitor_crp_child_exit(monitor_process).await;
        });

        Ok(process)
    }

    async fn pid(&self) -> Option<u32> {
        let pid = self.pid.load(Ordering::Relaxed);
        if pid != 0 {
            return Some(pid);
        }
        let child = self.child.lock().await;
        child.id()
    }

    async fn send(&self, command: CrpCommand) -> Result<()> {
        let envelope = CrpCommandEnvelope {
            v: CRP_VERSION,
            command,
        };
        let line = serde_json::to_string(&envelope)?;
        self.write_tx
            .send(line)
            .map_err(|_| anyhow::anyhow!("crp runtime stdin closed"))?;
        Ok(())
    }

    fn signal_shutdown(&self, reason: &str) {
        let next = reason.to_string();
        let prefer_over_stdout_close =
            next.starts_with("crp_runtime_exited:") || next.starts_with("crp_runtime_wait_failed:");

        // Avoid clobbering an existing shutdown reason (e.g. drain/restart), which is
        // user-visible via TurnInterrupted. The only exception is upgrading a generic
        // stdout-close reason to a more specific exit/wait failure.
        let _ = self.shutdown.send_if_modified(|current| {
            let should_replace = match current.as_deref() {
                None => true,
                Some("crp_runtime_stdout_closed") => prefer_over_stdout_close,
                Some(_) => false,
            };

            if should_replace {
                *current = Some(next.clone());
            }

            should_replace
        });
    }

    async fn shutdown(&self, reason: &str) {
        self.signal_shutdown(reason);
        let mut child = self.child.lock().await;
        if let Err(err) = child.kill().await {
            tracing::debug!(
                provider_id = %self.agent.provider_id,
                "crp shutdown failed ({reason}): {err}"
            );
        }
        let _ = child.wait().await;
        self.pid.store(0, Ordering::Relaxed);
    }
}

async fn monitor_crp_child_exit(process: Arc<CrpProcess>) {
    let mut shutdown_rx = process.shutdown.subscribe();
    loop {
        if shutdown_rx.borrow().is_some() {
            return;
        }

        let status = {
            let mut child = process.child.lock().await;
            child.try_wait()
        };

        match status {
            Ok(Some(status)) => {
                process.pid.store(0, Ordering::Relaxed);
                process.signal_shutdown(&format!("crp_runtime_exited: {status}"));
                return;
            }
            Ok(None) => {}
            Err(err) => {
                process.pid.store(0, Ordering::Relaxed);
                process.signal_shutdown(&format!("crp_runtime_wait_failed: {err}"));
                return;
            }
        }

        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(500)) => {},
            changed = shutdown_rx.changed() => {
                if changed.is_err() || shutdown_rx.borrow().is_some() {
                    return;
                }
            }
        }
    }
}

fn crp_log_paths(env: &HashMap<String, String>, provider_id: &str) -> Option<CrpLogPaths> {
    let data_root = crate::env::data_root_for_host(env)?;
    let timestamp = Utc::now().format("%Y-%m-%dT%H-%M-%SZ");
    let suffix = Uuid::new_v4().simple().to_string();
    let base = format!("crp-{}-{}-{}", provider_id, timestamp, suffix);
    let dir = Path::new(&data_root).join("logs").join("providers");
    Some(CrpLogPaths {
        codex_events: dir.join(format!("{base}.codex-events.jsonl")),
        crp_events: dir.join(format!("{base}.crp-events.jsonl")),
        stderr: dir.join(format!("{base}.stderr.log")),
    })
}

async fn stdout_pump(process: Arc<CrpProcess>, stdout: impl tokio::io::AsyncRead + Unpin) {
    // Debugging aid: when set, dump raw CRP stdout lines from the runtime to this file.
    // This lets us confirm what the runtime emitted without involving storage/UI layers.
    let dump_path = std::env::var("CTX_CRP_DUMP_EVENTS_PATH")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let mut dump_file = dump_path.as_deref().and_then(|path| {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok()
    });

    let mut lines = BufReader::new(stdout).lines();
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if let Some(f) = dump_file.as_mut() {
                    // Best-effort only; never fail the pump because debug dumping failed.
                    let _ = writeln!(f, "{trimmed}");
                }
                match serde_json::from_str::<CrpEventEnvelope>(trimmed) {
                    Ok(env) => {
                        let _ = process.events.send(env);
                    }
                    Err(err) => {
                        tracing::warn!(
                            provider_id = %process.agent.provider_id,
                            error = %err,
                            "failed to parse CRP event"
                        );
                    }
                }
            }
            Ok(None) => break,
            Err(err) => {
                tracing::warn!(
                    provider_id = %process.agent.provider_id,
                    error = %err,
                    "failed to read CRP stdout"
                );
                break;
            }
        }
    }

    process.signal_shutdown("crp_runtime_stdout_closed");
}

async fn stderr_pump(
    process: Arc<CrpProcess>,
    stderr: impl tokio::io::AsyncRead + Unpin,
    log_path: Option<PathBuf>,
) {
    let mut log_file = match log_path {
        Some(path) => tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .await
            .ok(),
        None => None,
    };
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(file) = log_file.as_mut() {
            let redacted = redact_sensitive(trimmed);
            if file.write_all(redacted.as_bytes()).await.is_err() {
                log_file = None;
            } else {
                let _ = file.write_all(b"\n").await;
                let _ = file.flush().await;
            }
        }
        let _ = process.stderr_lines.send(redact_sensitive(trimmed));
        tracing::debug!(
            provider_id = %process.agent.provider_id,
            "crp stderr: {}",
            trimmed
        );
    }
}

fn redact_sensitive(input: &str) -> String {
    fn redact_after_marker(mut s: String, marker: &str) -> String {
        let redacted = "[REDACTED]";
        let mut search_from = 0usize;
        while let Some(rel) = s[search_from..].find(marker) {
            let marker_start = search_from + rel;
            let start = marker_start + marker.len();
            if start >= s.len() {
                break;
            }
            if s[start..].starts_with(redacted) {
                search_from = start + redacted.len();
                continue;
            }

            let mut end = s.len();
            for (i, ch) in s[start..].char_indices() {
                if ch.is_whitespace() || ch == '"' || ch == '\'' || ch == '&' {
                    end = start + i;
                    break;
                }
            }

            s.replace_range(start..end, redacted);
            search_from = start + redacted.len();
        }
        s
    }

    let mut out = input.to_string();
    out = redact_after_marker(out, "Bearer ");
    out = redact_after_marker(out, "bearer ");
    out = redact_after_marker(out, "Authorization: Bearer ");
    out = redact_after_marker(out, "authorization: Bearer ");
    out = redact_after_marker(out, "token=");
    out = redact_after_marker(out, "TOKEN=");
    out = redact_after_marker(out, "CTX_AUTH_TOKEN=");
    out = redact_after_marker(out, "ctxAuthToken\":\"");
    out = redact_after_marker(out, "ctx_auth_token\":\"");
    out
}

fn extract_auth_url_from_stderr_line(line: &str) -> Option<String> {
    let mut search_from = 0usize;
    while search_from < line.len() {
        let haystack = &line[search_from..];
        let start_rel = haystack
            .find("https://")
            .or_else(|| haystack.find("http://"))?;
        let start = search_from + start_rel;
        let end = line[start..]
            .char_indices()
            .find_map(|(idx, ch)| {
                if ch.is_whitespace()
                    || matches!(ch, '"' | '\'' | '<' | '>' | '(' | ')' | '[' | ']')
                {
                    Some(start + idx)
                } else {
                    None
                }
            })
            .unwrap_or(line.len());
        let candidate = line[start..end].trim_end_matches(['.', ',', ';', ':']);
        if candidate.starts_with("http://") || candidate.starts_with("https://") {
            return Some(candidate.to_string());
        }
        search_from = end.saturating_add(1);
    }
    None
}

fn extract_auth_error_from_stderr_line(line: &str) -> Option<String> {
    let lowered = line.to_ascii_lowercase();
    if lowered.contains("auggie does not currently support authenticating over acp")
        || lowered.contains("please run `auggie login` from your terminal then try again")
    {
        return Some(
            "Auggie does not currently support ACP authentication in this environment. Run `auggie login` via the fallback flow."
                .to_string(),
        );
    }
    if lowered.contains("interactive consent could not be obtained")
        || lowered.contains("please run gemini cli in an interactive terminal to authenticate")
    {
        return Some(
            "Gemini CLI could not obtain interactive OAuth consent in this environment."
                .to_string(),
        );
    }
    None
}

#[derive(Debug, Serialize)]
struct CrpCommandEnvelope {
    v: u32,
    #[serde(flatten)]
    command: CrpCommand,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
#[allow(clippy::enum_variant_names)]
enum CrpCommand {
    #[serde(rename = "session.open")]
    SessionOpen {
        session_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        provider_session_id: Option<String>,
        config: Option<CrpSessionConfig>,
    },
    #[serde(rename = "session.prompt")]
    SessionPrompt {
        session_id: Option<String>,
        turn_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        items: Option<Vec<Value>>,
        prompt: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        reasoning_effort: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        cwd: Option<PathBuf>,
    },
    #[serde(rename = "session.compact")]
    SessionCompact {
        session_id: Option<String>,
        turn_id: Option<String>,
    },
    #[serde(rename = "session.undo")]
    SessionUndo {
        session_id: Option<String>,
        turn_id: Option<String>,
    },
    #[serde(rename = "session.review")]
    SessionReview {
        session_id: Option<String>,
        turn_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        instructions: Option<String>,
    },
    #[serde(rename = "session.authenticate")]
    SessionAuthenticate {
        session_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        method_id: Option<String>,
    },
    #[serde(rename = "session.cancel")]
    SessionCancel {
        session_id: Option<String>,
        turn_id: Option<String>,
    },
    #[serde(rename = "models.list")]
    ModelsList {
        #[serde(skip_serializing_if = "Option::is_none")]
        config: Option<CrpSessionConfig>,
    },
}

#[derive(Debug, Serialize)]
struct CrpSessionConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    cwd: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model_provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_trace_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    personality: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mcp_servers: Option<HashMap<String, CrpMcpServerConfig>>,
}

#[derive(Debug, Serialize)]
struct CrpMcpServerConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    args: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    env: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_timeout_sec: Option<f64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CrpModelInfo {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CrpModelsProbe {
    pub models: Vec<CrpModelInfo>,
    pub current_model_id: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct CrpEventEnvelope {
    #[allow(dead_code)]
    #[serde(default)]
    v: Option<u32>,
    seq: u64,
    #[allow(dead_code)]
    channel: CrpChannel,
    #[serde(flatten)]
    event: CrpEvent,
}

#[derive(Debug, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
enum CrpChannel {
    Control,
    Data,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize, Clone)]
#[serde(tag = "type")]
enum CrpEvent {
    #[serde(rename = "session.opened")]
    SessionOpened {
        session_id: String,
        provider_session_id: Option<String>,
        #[serde(default)]
        commands: Option<Value>,
        #[serde(default)]
        slash_commands: Option<Vec<String>>,
        #[serde(default)]
        models: Option<Value>,
        #[serde(default)]
        current_model_id: Option<String>,
        #[serde(default)]
        agents: Option<Value>,
        #[serde(default)]
        output_style: Option<String>,
        #[serde(default)]
        available_output_styles: Option<Vec<String>>,
        #[serde(default)]
        skills: Option<Vec<String>>,
        #[serde(default)]
        plugins: Option<Value>,
        #[serde(default)]
        tools: Option<Vec<String>>,
        #[serde(default)]
        permission_mode: Option<String>,
        #[serde(default)]
        mcp_servers: Option<Value>,
        #[serde(default)]
        account: Option<Value>,
        #[serde(default)]
        fast_mode_state: Option<String>,
    },
    #[serde(rename = "turn.started")]
    TurnStarted { session_id: String, turn_id: String },
    #[serde(rename = "message.delta")]
    MessageDelta {
        session_id: String,
        turn_id: String,
        message_id: String,
        delta: String,
    },
    #[serde(rename = "message.final")]
    MessageFinal {
        session_id: String,
        turn_id: String,
        message_id: String,
        content: String,
    },
    #[serde(rename = "reasoning.summary")]
    ReasoningSummary {
        session_id: String,
        turn_id: String,
        #[serde(default)]
        summary_index: i64,
        text: String,
        #[serde(default)]
        item_id: Option<String>,
    },
    #[serde(rename = "reasoning.trace")]
    ReasoningTrace {
        session_id: String,
        turn_id: String,
        chunk: String,
        #[serde(default)]
        encoding: Option<String>,
        #[serde(default)]
        summary_index: i64,
        #[serde(default)]
        item_id: Option<String>,
    },
    #[serde(rename = "reasoning.trace.final")]
    ReasoningTraceFinal {
        session_id: String,
        turn_id: String,
        content: String,
        #[serde(default)]
        encoding: Option<String>,
        #[serde(default)]
        summary_index: i64,
        #[serde(default)]
        item_id: Option<String>,
    },
    #[serde(rename = "tool.started")]
    ToolStarted {
        session_id: String,
        turn_id: String,
        tool_call_id: String,
        tool_name: String,
        #[serde(default)]
        tool_label: Option<String>,
        #[serde(default)]
        input: Option<Value>,
        #[serde(default)]
        input_preview: Option<Value>,
    },
    #[serde(rename = "tool.output.delta")]
    ToolOutputDelta {
        session_id: String,
        turn_id: String,
        tool_call_id: String,
        #[serde(default)]
        chunk: String,
    },
    #[serde(rename = "tool.completed")]
    ToolCompleted {
        session_id: String,
        turn_id: String,
        tool_call_id: String,
        tool_name: String,
        #[serde(default)]
        tool_label: Option<String>,
        status: CrpToolStatus,
        #[serde(default)]
        output: Option<Value>,
        #[serde(default)]
        error: Option<String>,
        #[serde(default)]
        input_preview: Option<Value>,
    },
    #[serde(rename = "models.list")]
    ModelsList {
        models: Vec<CrpModelInfo>,
        #[serde(default)]
        current_model_id: Option<String>,
    },
    #[serde(rename = "turn.completed")]
    TurnCompleted {
        session_id: String,
        turn_id: String,
        status: CrpTurnStatus,
        #[serde(default)]
        error: Option<CrpTurnError>,
    },
    #[serde(rename = "session.gap")]
    SessionGap {
        session_id: String,
        #[serde(default)]
        turn_id: Option<String>,
        #[serde(default)]
        reason: Option<String>,
    },
    #[serde(rename = "session.notice")]
    SessionNotice {
        session_id: String,
        #[serde(default)]
        turn_id: Option<String>,
        code: String,
        #[serde(default)]
        severity: Option<String>,
        #[serde(default)]
        message: Option<String>,
        #[serde(default)]
        details: Option<Value>,
        #[serde(default)]
        transient: Option<bool>,
    },
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "snake_case")]
enum CrpTurnStatus {
    Success,
    Error,
    Canceled,
    Interrupted,
}

#[derive(Debug, Deserialize, Clone)]
struct CrpTurnError {
    message: String,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    details: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "snake_case")]
enum CrpToolStatus {
    Success,
    Error,
}

struct MappedCrpEvent {
    events: Vec<NormalizedEvent>,
    done: bool,
}

#[derive(Debug, Clone)]
struct CachedToolInput {
    input: Option<Value>,
    input_preview: Option<Value>,
}

fn map_crp_event(
    event: CrpEvent,
    channel: CrpChannel,
    seq: u64,
    tool_output_cache: &mut HashMap<String, String>,
    tool_input_cache: &mut HashMap<String, CachedToolInput>,
) -> MappedCrpEvent {
    let crp_channel = match channel {
        CrpChannel::Data => Some("data"),
        CrpChannel::Control => None,
    };
    match event {
        CrpEvent::SessionOpened {
            session_id,
            provider_session_id,
            commands,
            slash_commands,
            models,
            current_model_id,
            agents,
            output_style,
            available_output_styles,
            skills,
            plugins,
            tools,
            permission_mode,
            mcp_servers,
            account,
            fast_mode_state,
        } => {
            let mut payload = serde_json::Map::new();
            payload.insert("session_id".to_string(), json!(session_id));
            if let Some(provider_session_id) = provider_session_id {
                payload.insert(
                    "provider_session_id".to_string(),
                    json!(provider_session_id),
                );
            }
            if let Some(commands) = commands {
                payload.insert("commands".to_string(), commands);
            }
            if let Some(slash_commands) = slash_commands {
                payload.insert("slash_commands".to_string(), json!(slash_commands));
            }
            if let Some(models) = models {
                payload.insert("models".to_string(), models);
            }
            if let Some(current_model_id) = current_model_id {
                payload.insert("current_model_id".to_string(), json!(current_model_id));
            }
            if let Some(agents) = agents {
                payload.insert("agents".to_string(), agents);
            }
            if let Some(output_style) = output_style {
                payload.insert("output_style".to_string(), json!(output_style));
            }
            if let Some(available_output_styles) = available_output_styles {
                payload.insert(
                    "available_output_styles".to_string(),
                    json!(available_output_styles),
                );
            }
            if let Some(skills) = skills {
                payload.insert("skills".to_string(), json!(skills));
            }
            if let Some(plugins) = plugins {
                payload.insert("plugins".to_string(), plugins);
            }
            if let Some(tools) = tools {
                payload.insert("tools".to_string(), json!(tools));
            }
            if let Some(permission_mode) = permission_mode {
                payload.insert("permission_mode".to_string(), json!(permission_mode));
            }
            if let Some(mcp_servers) = mcp_servers {
                payload.insert("mcp_servers".to_string(), mcp_servers);
            }
            if let Some(account) = account {
                payload.insert("account".to_string(), account);
            }
            if let Some(fast_mode_state) = fast_mode_state {
                payload.insert("fast_mode_state".to_string(), json!(fast_mode_state));
            }
            MappedCrpEvent {
                events: vec![NormalizedEvent {
                    event_type: SessionEventType::Init,
                    payload_json: Value::Object(payload),
                }],
                done: false,
            }
        }
        // The scheduler already emits the canonical turn lifecycle events. Treat harness-emitted
        // `turn.started` as internal signal only to avoid duplicating `turn_started` rows with a
        // mismatched payload shape.
        CrpEvent::TurnStarted { .. } => MappedCrpEvent {
            events: Vec::new(),
            done: false,
        },
        CrpEvent::MessageDelta {
            delta, message_id, ..
        } => MappedCrpEvent {
            events: vec![NormalizedEvent {
                event_type: SessionEventType::AssistantChunk,
                payload_json: json!({
                    "content_fragment": delta,
                    "message_id": message_id,
                    "crp_seq": seq,
                    "crp_channel": crp_channel,
                }),
            }],
            done: false,
        },
        CrpEvent::MessageFinal {
            content,
            message_id,
            ..
        } => MappedCrpEvent {
            events: vec![NormalizedEvent {
                event_type: SessionEventType::AssistantComplete,
                payload_json: json!({
                    "full_content": content,
                    "message_id": message_id,
                    "crp_seq": seq,
                }),
            }],
            done: false,
        },
        CrpEvent::ReasoningSummary {
            text,
            item_id,
            summary_index,
            ..
        } => MappedCrpEvent {
            events: vec![NormalizedEvent {
                event_type: SessionEventType::Notice,
                payload_json: json!({
                    "kind": "reasoning_summary",
                    "summary_index": summary_index,
                    "text": text,
                    "item_id": item_id,
                    "crp_seq": seq,
                }),
            }],
            done: false,
        },
        CrpEvent::ReasoningTrace {
            chunk,
            encoding,
            summary_index,
            item_id,
            ..
        } => {
            let mut payload = json!({
                "content_fragment": chunk,
                "encoding": encoding,
                "summary_index": summary_index,
                "item_id": item_id,
                "crp_seq": seq,
            });
            if let Some(channel) = crp_channel {
                if let Some(obj) = payload.as_object_mut() {
                    obj.insert("crp_channel".to_string(), json!(channel));
                }
            }
            MappedCrpEvent {
                events: vec![NormalizedEvent {
                    event_type: SessionEventType::ThoughtChunk,
                    payload_json: payload,
                }],
                done: false,
            }
        }
        CrpEvent::ReasoningTraceFinal {
            content,
            encoding,
            summary_index,
            item_id,
            ..
        } => {
            let mut payload = json!({
                "content_fragment": "",
                "full_content": content,
                "is_final": true,
                "encoding": encoding,
                "summary_index": summary_index,
                "item_id": item_id,
                "crp_seq": seq,
            });
            // item_id enables deterministic thought chunk grouping/deduping in clients.
            if let Some(item_id) = item_id {
                if let Some(obj) = payload.as_object_mut() {
                    obj.insert("item_id".to_string(), json!(item_id));
                }
            }
            if let Some(channel) = crp_channel {
                if let Some(obj) = payload.as_object_mut() {
                    obj.insert("crp_channel".to_string(), json!(channel));
                }
            }
            MappedCrpEvent {
                events: vec![NormalizedEvent {
                    event_type: SessionEventType::ThoughtChunk,
                    payload_json: payload,
                }],
                done: false,
            }
        }
        CrpEvent::ToolStarted {
            tool_call_id,
            tool_name,
            tool_label,
            input,
            input_preview,
            ..
        } => {
            if input.is_some() || input_preview.is_some() {
                tool_input_cache.insert(
                    tool_call_id.clone(),
                    CachedToolInput {
                        input: input.clone(),
                        input_preview: input_preview.clone(),
                    },
                );
            }
            let payload = build_tool_started_payload(
                tool_call_id,
                tool_name,
                tool_label,
                input,
                input_preview,
                seq,
            );
            MappedCrpEvent {
                events: vec![NormalizedEvent {
                    event_type: SessionEventType::ToolCall,
                    payload_json: payload,
                }],
                done: false,
            }
        }
        CrpEvent::ToolOutputDelta {
            tool_call_id,
            chunk,
            ..
        } => {
            let output_text = {
                let entry = tool_output_cache.entry(tool_call_id.clone()).or_default();
                entry.push_str(&chunk);
                entry.clone()
            };
            let mut payload = json!({
                "tool_call_id": tool_call_id,
                "outputText": output_text,
                "status": "running",
                "crp_seq": seq,
            });
            if let Some(channel) = crp_channel {
                if let Some(obj) = payload.as_object_mut() {
                    obj.insert("crp_channel".to_string(), json!(channel));
                }
            }
            MappedCrpEvent {
                events: vec![NormalizedEvent {
                    event_type: SessionEventType::ToolCallUpdate,
                    payload_json: payload,
                }],
                done: false,
            }
        }
        CrpEvent::ToolCompleted {
            tool_call_id,
            tool_name,
            tool_label,
            status,
            output,
            error,
            input_preview,
            ..
        } => {
            let tool_call_id_for_cache = tool_call_id.clone();
            let cached = tool_input_cache.remove(&tool_call_id_for_cache);
            let (input, cached_preview) = cached
                .map(|c| (c.input, c.input_preview))
                .unwrap_or((None, None));
            let input_preview = input_preview.or(cached_preview);
            let payload = build_tool_completed_payload(
                tool_call_id,
                tool_name,
                tool_label,
                status,
                output,
                error,
                input,
                input_preview,
                seq,
            );
            tool_output_cache.remove(&tool_call_id_for_cache);
            MappedCrpEvent {
                events: vec![NormalizedEvent {
                    event_type: SessionEventType::ToolResult,
                    payload_json: payload,
                }],
                done: false,
            }
        }
        CrpEvent::TurnCompleted { status, error, .. } => {
            let (event_type, payload) = match status {
                CrpTurnStatus::Success => (
                    SessionEventType::Done,
                    json!({"status": "completed", "crp_seq": seq}),
                ),
                CrpTurnStatus::Error => {
                    let message = error
                        .as_ref()
                        .map(|err| err.message.clone())
                        .unwrap_or_else(|| "crp_turn_error".to_string());
                    let kind = error.as_ref().and_then(|err| err.kind.clone());
                    let details = error.as_ref().and_then(|err| err.details.clone());
                    let mut payload = serde_json::Map::new();
                    payload.insert("message".to_string(), json!(message));
                    if let Some(kind) = kind {
                        payload.insert("kind".to_string(), json!(kind));
                    }
                    if let Some(details) = details {
                        payload.insert("details".to_string(), json!(details));
                    }
                    payload.insert("crp_seq".to_string(), json!(seq));
                    (SessionEventType::Error, Value::Object(payload))
                }
                CrpTurnStatus::Canceled | CrpTurnStatus::Interrupted => (
                    SessionEventType::TurnInterrupted,
                    json!({"reason": "crp_turn_interrupted", "crp_seq": seq}),
                ),
            };
            MappedCrpEvent {
                events: vec![NormalizedEvent {
                    event_type,
                    payload_json: payload,
                }],
                done: true,
            }
        }
        CrpEvent::ModelsList { .. } => MappedCrpEvent {
            events: Vec::new(),
            done: false,
        },
        CrpEvent::SessionNotice {
            code,
            severity,
            message,
            details,
            transient,
            ..
        } => {
            let mut payload = serde_json::Map::new();
            payload.insert("kind".to_string(), json!(code.clone()));
            payload.insert("code".to_string(), json!(code));
            if let Some(severity) = severity {
                payload.insert("severity".to_string(), json!(severity));
            }
            if let Some(message) = message {
                payload.insert("message".to_string(), json!(message));
            }
            if let Some(details) = details {
                if let Value::Object(map) = &details {
                    if let Some(auth_methods) =
                        map.get("auth_methods").or_else(|| map.get("authMethods"))
                    {
                        payload.insert("auth_methods".to_string(), auth_methods.clone());
                    }
                    if let Some(provider) = map.get("provider") {
                        payload.insert("provider".to_string(), provider.clone());
                    }
                }
                payload.insert("details".to_string(), details);
            }
            if let Some(transient) = transient {
                payload.insert("transient".to_string(), json!(transient));
            }
            payload.insert("crp_seq".to_string(), json!(seq));
            MappedCrpEvent {
                events: vec![NormalizedEvent {
                    event_type: SessionEventType::Notice,
                    payload_json: Value::Object(payload),
                }],
                done: false,
            }
        }
        CrpEvent::SessionGap { reason, .. } => MappedCrpEvent {
            events: vec![NormalizedEvent {
                event_type: SessionEventType::Notice,
                payload_json: json!({
                    "kind": "session_gap",
                    "reason": reason,
                    "crp_seq": seq,
                }),
            }],
            done: false,
        },
    }
}

fn build_tool_started_payload(
    tool_call_id: String,
    tool_name: String,
    tool_label: Option<String>,
    input: Option<Value>,
    input_preview: Option<Value>,
    seq: u64,
) -> Value {
    let mut payload = serde_json::Map::new();
    let tool_name_for_call = tool_name.clone();
    payload.insert("tool_call_id".to_string(), json!(tool_call_id.clone()));
    payload.insert("kind".to_string(), json!(tool_name.clone()));
    if let Some(label) = tool_label.as_ref() {
        payload.insert("tool_label".to_string(), json!(label));
    }
    payload.insert("status".to_string(), json!("running"));
    let raw_input = input.clone().or_else(|| input_preview.clone());
    if let Some(input) = raw_input.clone() {
        payload.insert("rawInput".to_string(), input);
    }
    if let Some(input_preview) = input_preview.clone() {
        payload.insert("input_preview".to_string(), input_preview);
    }
    let mut tool_call_obj = serde_json::Map::new();
    tool_call_obj.insert("id".to_string(), json!(tool_call_id));
    tool_call_obj.insert("name".to_string(), json!(tool_name_for_call.clone()));
    tool_call_obj.insert("kind".to_string(), json!(tool_name_for_call));
    tool_call_obj.insert(
        "rawInput".to_string(),
        raw_input.clone().unwrap_or(Value::Null),
    );
    tool_call_obj.insert("status".to_string(), json!("running"));
    payload.insert("toolCall".to_string(), Value::Object(tool_call_obj));
    payload.insert("crp_seq".to_string(), json!(seq));
    Value::Object(payload)
}

#[allow(clippy::too_many_arguments)]
fn build_tool_completed_payload(
    tool_call_id: String,
    tool_name: String,
    tool_label: Option<String>,
    status: CrpToolStatus,
    output: Option<Value>,
    error: Option<String>,
    input: Option<Value>,
    input_preview: Option<Value>,
    seq: u64,
) -> Value {
    let mut payload = serde_json::Map::new();
    let tool_name_for_call = tool_name.clone();
    payload.insert("tool_call_id".to_string(), json!(tool_call_id.clone()));
    payload.insert("kind".to_string(), json!(tool_name.clone()));
    if let Some(label) = tool_label.as_ref() {
        payload.insert("tool_label".to_string(), json!(label));
    }
    payload.insert(
        "status".to_string(),
        json!(match status {
            CrpToolStatus::Success => "completed",
            CrpToolStatus::Error => "failed",
        }),
    );
    let raw_input = input.clone().or_else(|| input_preview.clone());
    if let Some(input) = raw_input.clone() {
        payload.insert("rawInput".to_string(), input);
    }
    if let Some(input_preview) = input_preview.clone() {
        payload.insert("input_preview".to_string(), input_preview);
    }
    if let Some(output) = output.clone() {
        if let Some(text) = extract_output_text(&output) {
            payload.insert("output_text".to_string(), json!(text));
        }
        payload.insert("rawOutput".to_string(), output);
    }
    if let Some(err) = error {
        payload.insert("error".to_string(), json!(err));
    }
    if let Some(label) = tool_label.clone() {
        payload.insert("tool_label".to_string(), json!(label));
    }
    let mut tool_call_obj = serde_json::Map::new();
    tool_call_obj.insert("id".to_string(), json!(tool_call_id));
    tool_call_obj.insert("name".to_string(), json!(tool_name_for_call.clone()));
    tool_call_obj.insert("kind".to_string(), json!(tool_name_for_call));
    tool_call_obj.insert(
        "rawInput".to_string(),
        raw_input.clone().unwrap_or(Value::Null),
    );
    tool_call_obj.insert(
        "rawOutput".to_string(),
        payload.get("rawOutput").cloned().unwrap_or(Value::Null),
    );
    tool_call_obj.insert(
        "status".to_string(),
        payload.get("status").cloned().unwrap_or(Value::Null),
    );
    payload.insert("toolCall".to_string(), Value::Object(tool_call_obj));
    payload.insert("crp_seq".to_string(), json!(seq));
    Value::Object(payload)
}

fn extract_output_text(output: &Value) -> Option<String> {
    if let Some(text) = output.get("formatted_output").and_then(Value::as_str) {
        return Some(text.to_string());
    }
    if let Some(text) = output.get("aggregated_output").and_then(Value::as_str) {
        return Some(text.to_string());
    }
    if let Some(text) = output.get("stdout").and_then(Value::as_str) {
        return Some(text.to_string());
    }
    if let Some(text) = output.get("result").and_then(Value::as_str) {
        return Some(text.to_string());
    }
    output.as_str().map(|text| text.to_string())
}

fn event_turn_id(event: &CrpEvent) -> Option<&str> {
    match event {
        CrpEvent::SessionGap { turn_id, .. } => turn_id.as_deref(),
        CrpEvent::SessionNotice { turn_id, .. } => turn_id.as_deref(),
        CrpEvent::TurnStarted { turn_id, .. }
        | CrpEvent::MessageDelta { turn_id, .. }
        | CrpEvent::MessageFinal { turn_id, .. }
        | CrpEvent::ReasoningSummary { turn_id, .. }
        | CrpEvent::ReasoningTrace { turn_id, .. }
        | CrpEvent::ReasoningTraceFinal { turn_id, .. }
        | CrpEvent::ToolStarted { turn_id, .. }
        | CrpEvent::ToolOutputDelta { turn_id, .. }
        | CrpEvent::ToolCompleted { turn_id, .. }
        | CrpEvent::TurnCompleted { turn_id, .. } => Some(turn_id.as_str()),
        CrpEvent::SessionOpened { .. } | CrpEvent::ModelsList { .. } => None,
    }
}

fn event_matches_session(event: &CrpEvent, session_id: &str) -> bool {
    match event {
        CrpEvent::SessionOpened { session_id: id, .. }
        | CrpEvent::TurnStarted { session_id: id, .. }
        | CrpEvent::MessageDelta { session_id: id, .. }
        | CrpEvent::MessageFinal { session_id: id, .. }
        | CrpEvent::ReasoningSummary { session_id: id, .. }
        | CrpEvent::ReasoningTrace { session_id: id, .. }
        | CrpEvent::ReasoningTraceFinal { session_id: id, .. }
        | CrpEvent::ToolStarted { session_id: id, .. }
        | CrpEvent::ToolOutputDelta { session_id: id, .. }
        | CrpEvent::ToolCompleted { session_id: id, .. }
        | CrpEvent::TurnCompleted { session_id: id, .. }
        | CrpEvent::SessionGap { session_id: id, .. }
        | CrpEvent::SessionNotice { session_id: id, .. } => id == session_id,
        CrpEvent::ModelsList { .. } => false,
    }
}

fn build_crp_session_config(env: &HashMap<String, String>, workdir: &Path) -> CrpSessionConfig {
    let mcp_enabled = env
        .get("CTX_MCP_DISABLED")
        .map(|v| v != "1" && v.to_lowercase() != "true")
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

fn build_crp_model_probe_config(env: &HashMap<String, String>, workdir: &Path) -> CrpSessionConfig {
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

fn split_model_id_and_effort(model_id: &str) -> (Option<String>, Option<String>) {
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

fn synthetic_models_probe_for_provider(
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
    })
}

fn probe_timeout_for_env(env: &HashMap<String, String>) -> Duration {
    if container_exec_spec(env).is_some() {
        CRP_MODEL_PROBE_TIMEOUT_CONTAINER
    } else {
        CRP_MODEL_PROBE_TIMEOUT
    }
}

fn runtime_launch_probe_timeout_for_env(env: &HashMap<String, String>) -> Duration {
    if container_exec_spec(env).is_some() {
        CRP_RUNTIME_LAUNCH_PROBE_TIMEOUT_CONTAINER
    } else {
        CRP_RUNTIME_LAUNCH_PROBE_TIMEOUT
    }
}

fn spawn_probe_output_tail_reader<R>(
    reader: R,
    provider_id: String,
    stream_name: &'static str,
    tail_ref: Arc<Mutex<Vec<String>>>,
) where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            tracing::debug!(
                provider_id = %provider_id,
                stream = stream_name,
                "crp {}: {}",
                stream_name,
                trimmed
            );
            let mut tail = tail_ref.lock().await;
            if tail.len() >= 20 {
                let _ = tail.remove(0);
            }
            tail.push(trimmed.to_string());
        }
    });
}

async fn format_probe_output_tail(label: &str, tail: &Arc<Mutex<Vec<String>>>) -> String {
    let lines = tail.lock().await;
    if lines.is_empty() {
        String::new()
    } else {
        format!("; {label}_tail={}", lines.join(" | "))
    }
}

fn apply_outer_process_env(cmd: &mut Command, env: &HashMap<String, String>) {
    let is_container_exec = container_exec_spec(env).is_some();
    for (key, value) in env {
        if should_skip_outer_process_env_key(key, is_container_exec) {
            continue;
        }
        cmd.env(key, value);
    }
}

fn should_skip_outer_process_env_key(key: &str, is_container_exec: bool) -> bool {
    if !is_container_exec {
        return false;
    }
    matches!(key, "HOME" | "TMPDIR" | "TMP" | "TEMP") || key.starts_with("XDG_")
}

pub async fn probe_crp_models(
    provider_id: &str,
    command: String,
    args: Vec<String>,
    workdir: PathBuf,
    env: HashMap<String, String>,
) -> Result<CrpModelsProbe> {
    if provider_id == "cline" {
        return synthetic_models_probe_for_provider(provider_id, &env).ok_or_else(|| {
            anyhow!(
                "Cline model discovery requires OPENAI_MODEL; configure a model override for the endpoint"
            )
        });
    }
    if let Some(probe) = synthetic_models_probe_for_provider(provider_id, &env) {
        return Ok(probe);
    }
    let command_label = command.clone();
    let container_spec = container_exec_spec(&env);
    let probe_timeout = probe_timeout_for_env(&env);
    let mut cmd = if let Some(spec) = container_spec {
        let (container_command, container_args) =
            rewrite_container_command_for_linux(&command, &args, &env)?;
        build_container_exec_command(&spec, &workdir, &env, &container_command, &container_args)
    } else {
        let mut cmd = Command::new(&command);
        cmd.args(&args);
        cmd.current_dir(&workdir);
        cmd
    };
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    apply_outer_process_env(&mut cmd, &env);

    let mut child = cmd
        .spawn()
        .with_context(|| format!("spawning CRP runtime {provider_id} ({command_label})"))?;
    let stdin = child.stdin.take().context("capturing CRP stdin")?;
    let stdout = child.stdout.take().context("capturing CRP stdout")?;
    let stderr = child.stderr.take().context("capturing CRP stderr")?;

    let stderr_tail: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    spawn_probe_output_tail_reader(
        stderr,
        provider_id.to_string(),
        "stderr",
        Arc::clone(&stderr_tail),
    );

    let mut stdin = BufWriter::new(stdin);
    let mut stdout_reader = BufReader::new(stdout).lines();
    let config = build_crp_model_probe_config(&env, &workdir);
    let envelope = CrpCommandEnvelope {
        v: CRP_VERSION,
        command: CrpCommand::ModelsList {
            config: Some(config),
        },
    };
    let line = serde_json::to_string(&envelope)?;
    stdin.write_all(line.as_bytes()).await?;
    stdin.write_all(b"\n").await?;
    stdin.flush().await?;

    let result = match timeout(probe_timeout, async {
        loop {
            let Some(line) = stdout_reader.next_line().await? else {
                anyhow::bail!("crp runtime closed before models.list response");
            };
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let env = match serde_json::from_str::<CrpEventEnvelope>(trimmed) {
                Ok(env) => env,
                Err(_) => continue,
            };
            if let CrpEvent::ModelsList {
                models,
                current_model_id,
            } = env.event
            {
                return Ok(CrpModelsProbe {
                    models,
                    current_model_id,
                });
            }
        }
    })
    .await
    {
        Ok(result) => result?,
        Err(_) => {
            let stderr_tail = {
                let lines = stderr_tail.lock().await;
                if lines.is_empty() {
                    String::new()
                } else {
                    format!("; stderr_tail={}", lines.join(" | "))
                }
            };
            anyhow::bail!(
                "CRP models.list probe timed out after {}s{}",
                probe_timeout.as_secs(),
                stderr_tail
            );
        }
    };

    let _ = child.kill().await;
    let _ = child.wait().await;

    Ok(result)
}

pub async fn probe_crp_runtime_launch(
    provider_id: &str,
    command: String,
    args: Vec<String>,
    workdir: PathBuf,
    env: HashMap<String, String>,
) -> Result<()> {
    let command_label = command.clone();
    let container_spec = container_exec_spec(&env);
    let probe_timeout = runtime_launch_probe_timeout_for_env(&env);
    let mut cmd = if let Some(spec) = container_spec {
        let (container_command, container_args) =
            rewrite_container_command_for_linux(&command, &args, &env)?;
        build_container_exec_command(&spec, &workdir, &env, &container_command, &container_args)
    } else {
        let mut cmd = Command::new(&command);
        cmd.args(&args);
        cmd.current_dir(&workdir);
        cmd
    };
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    apply_outer_process_env(&mut cmd, &env);

    let mut child = cmd
        .spawn()
        .with_context(|| format!("spawning CRP runtime {provider_id} ({command_label})"))?;
    let stdout = child.stdout.take().context("capturing CRP stdout")?;
    let stderr = child.stderr.take().context("capturing CRP stderr")?;

    let stdout_tail: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    spawn_probe_output_tail_reader(
        stdout,
        provider_id.to_string(),
        "stdout",
        Arc::clone(&stdout_tail),
    );
    let stderr_tail: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    spawn_probe_output_tail_reader(
        stderr,
        provider_id.to_string(),
        "stderr",
        Arc::clone(&stderr_tail),
    );

    let started_at = tokio::time::Instant::now();
    loop {
        if let Some(exit_status) = child
            .try_wait()
            .with_context(|| format!("waiting for CRP runtime {provider_id} during launch probe"))?
        {
            let stdout_tail = format_probe_output_tail("stdout", &stdout_tail).await;
            let stderr_tail = format_probe_output_tail("stderr", &stderr_tail).await;
            anyhow::bail!(
                "CRP runtime exited during launch probe with status {exit_status}{stdout_tail}{stderr_tail}"
            );
        }
        if started_at.elapsed() >= probe_timeout {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn build_prompt_items(
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
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn extract_auth_url_from_stderr_line_parses_google_url() {
        let line = "ERROR ... Raw: https://accounts.google.com/o/oauth2/v2/auth?client_id=abc";
        assert_eq!(
            extract_auth_url_from_stderr_line(line).as_deref(),
            Some("https://accounts.google.com/o/oauth2/v2/auth?client_id=abc")
        );
    }

    #[test]
    fn extract_auth_error_from_stderr_line_detects_interactive_consent_failure() {
        let line = "message: 'Interactive consent could not be obtained.'";
        assert_eq!(
            extract_auth_error_from_stderr_line(line).as_deref(),
            Some("Gemini CLI could not obtain interactive OAuth consent in this environment.")
        );
    }

    #[test]
    fn extract_auth_error_from_stderr_line_detects_auggie_acp_auth_unsupported() {
        let line = "message: 'Authentication required: Auggie does not currently support authenticating over ACP. Please run `auggie login` from your terminal then try again.'";
        assert_eq!(
            extract_auth_error_from_stderr_line(line).as_deref(),
            Some(
                "Auggie does not currently support ACP authentication in this environment. Run `auggie login` via the fallback flow."
            )
        );
    }

    #[test]
    fn container_exec_outer_process_env_skips_provider_home_and_xdg_keys() {
        assert!(should_skip_outer_process_env_key("HOME", true));
        assert!(should_skip_outer_process_env_key("TMPDIR", true));
        assert!(should_skip_outer_process_env_key("XDG_CONFIG_HOME", true));
        assert!(should_skip_outer_process_env_key("XDG_STATE_HOME", true));
        assert!(!should_skip_outer_process_env_key("OPENAI_API_KEY", true));
        assert!(!should_skip_outer_process_env_key("HOME", false));
    }

    #[test]
    fn probe_timeout_for_env_defaults_to_host_timeout() {
        let env = HashMap::<String, String>::new();
        assert_eq!(probe_timeout_for_env(&env), CRP_MODEL_PROBE_TIMEOUT);
    }

    #[test]
    fn probe_timeout_for_env_uses_container_timeout_when_container_exec_is_present() {
        let mut env = HashMap::<String, String>::new();
        env.insert(
            "CTX_HARNESS_CONTAINER_ID".to_string(),
            "ctx-workspace-123".to_string(),
        );
        assert_eq!(
            probe_timeout_for_env(&env),
            CRP_MODEL_PROBE_TIMEOUT_CONTAINER
        );
    }

    #[test]
    fn tool_completed_retains_started_preview_when_completed_omits_it() {
        let mut tool_output_cache: HashMap<String, String> = HashMap::new();
        let mut tool_input_cache: HashMap<String, CachedToolInput> = HashMap::new();

        let started_preview = json!({"summary":"Read: foo.txt"});
        let started = map_crp_event(
            CrpEvent::ToolStarted {
                session_id: "s".to_string(),
                turn_id: "t".to_string(),
                tool_call_id: "call1".to_string(),
                tool_name: "read_file".to_string(),
                tool_label: None,
                input: None,
                input_preview: Some(started_preview.clone()),
            },
            CrpChannel::Control,
            1,
            &mut tool_output_cache,
            &mut tool_input_cache,
        );
        assert_eq!(started.events.len(), 1);
        assert!(matches!(
            &started.events[0].event_type,
            SessionEventType::ToolCall
        ));

        let completed = map_crp_event(
            CrpEvent::ToolCompleted {
                session_id: "s".to_string(),
                turn_id: "t".to_string(),
                tool_call_id: "call1".to_string(),
                tool_name: "read_file".to_string(),
                tool_label: None,
                status: CrpToolStatus::Success,
                output: None,
                error: None,
                input_preview: None,
            },
            CrpChannel::Control,
            2,
            &mut tool_output_cache,
            &mut tool_input_cache,
        );
        assert_eq!(completed.events.len(), 1);
        assert!(matches!(
            &completed.events[0].event_type,
            SessionEventType::ToolResult
        ));

        let payload = &completed.events[0].payload_json;
        assert_eq!(payload.get("input_preview"), Some(&started_preview));
        assert_eq!(payload.get("rawInput"), Some(&started_preview));
    }

    #[test]
    fn native_crp_slash_commands_are_codex_only() {
        assert_eq!(
            parse_native_crp_slash_command_for_provider("codex", "/compact"),
            Some(CrpSlashCommand::Compact)
        );
        assert_eq!(
            parse_native_crp_slash_command_for_provider("codex", "/review focus on security"),
            Some(CrpSlashCommand::Review {
                instructions: Some("focus on security".to_string())
            })
        );
        assert_eq!(
            parse_native_crp_slash_command_for_provider("claude-crp", "/compact"),
            None
        );
        assert_eq!(
            parse_native_crp_slash_command_for_provider("claude-crp", "/review focus on security"),
            None
        );
    }

    #[test]
    fn claude_command_policy_blocks_redundant_and_unsupported_commands() {
        assert_eq!(
            classify_claude_slash_command("compact"),
            ClaudeSlashCommandPolicy::Supported
        );
        assert_eq!(
            classify_claude_slash_command("clear"),
            ClaudeSlashCommandPolicy::Redundant(
                "ctx handles this workflow outside Claude slash commands."
            )
        );
        assert_eq!(
            classify_claude_slash_command("mcp__docs__search"),
            ClaudeSlashCommandPolicy::Unsupported(
                "MCP prompt commands are intentionally out of scope in ctx right now."
            )
        );
        assert!(validate_provider_slash_command_support("claude-crp", "/compact").is_ok());
        assert!(validate_provider_slash_command_support("claude-crp", "/clear").is_err());
        assert!(
            validate_provider_slash_command_support("claude-crp", "/mcp__docs__search").is_err()
        );
        assert!(validate_provider_slash_command_support("codex", "/clear").is_ok());
    }

    #[test]
    fn session_opened_preserves_claude_supported_command_metadata() {
        let mut tool_output_cache: HashMap<String, String> = HashMap::new();
        let mut tool_input_cache: HashMap<String, CachedToolInput> = HashMap::new();

        let mapped = map_crp_event(
            CrpEvent::SessionOpened {
                session_id: "session-1".to_string(),
                provider_session_id: Some("provider-session-1".to_string()),
                commands: Some(json!([
                    {
                        "name": "compact",
                        "description": "Summarize conversation to save context",
                        "argument_hint": "<focus>"
                    }
                ])),
                slash_commands: Some(vec!["compact".to_string(), "review".to_string()]),
                models: Some(json!([
                    {
                        "id": "sonnet",
                        "name": "Sonnet"
                    }
                ])),
                current_model_id: Some("sonnet".to_string()),
                agents: Some(json!([
                    {
                        "name": "Explore",
                        "description": "Research the repo"
                    }
                ])),
                output_style: Some("default".to_string()),
                available_output_styles: Some(vec!["default".to_string(), "brief".to_string()]),
                skills: Some(vec!["simplify".to_string()]),
                plugins: Some(json!([
                    {
                        "name": "plugin-a",
                        "path": "/tmp/plugin-a"
                    }
                ])),
                tools: Some(vec!["Read".to_string(), "Write".to_string()]),
                permission_mode: Some("default".to_string()),
                mcp_servers: Some(json!([{ "name": "github", "status": "connected" }])),
                account: Some(json!({ "email": "dev@example.com" })),
                fast_mode_state: Some("off".to_string()),
            },
            CrpChannel::Control,
            1,
            &mut tool_output_cache,
            &mut tool_input_cache,
        );

        assert_eq!(mapped.events.len(), 1);
        assert!(matches!(mapped.events[0].event_type, SessionEventType::Init));
        let payload = &mapped.events[0].payload_json;
        assert_eq!(
            payload.get("session_id"),
            Some(&json!("session-1"))
        );
        assert_eq!(
            payload.get("provider_session_id"),
            Some(&json!("provider-session-1"))
        );
        assert_eq!(
            payload.pointer("/commands/0/name"),
            Some(&json!("compact"))
        );
        assert_eq!(
            payload.pointer("/commands/0/description"),
            Some(&json!("Summarize conversation to save context"))
        );
        assert_eq!(
            payload.get("slash_commands"),
            Some(&json!(["compact", "review"]))
        );
        assert_eq!(payload.get("current_model_id"), Some(&json!("sonnet")));
        assert_eq!(payload.get("output_style"), Some(&json!("default")));
        assert_eq!(
            payload.get("available_output_styles"),
            Some(&json!(["default", "brief"]))
        );
        assert_eq!(payload.get("skills"), Some(&json!(["simplify"])));
        assert_eq!(payload.get("permission_mode"), Some(&json!("default")));
        assert_eq!(payload.get("fast_mode_state"), Some(&json!("off")));
    }

    #[test]
    fn rewrite_bundled_path_for_linux_rewrites_provider_paths() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let host = tmp
            .path()
            .join("bundles/providers/acp-crp-bridge/macos/aarch64/acp-crp-bridge");
        let linux = tmp
            .path()
            .join("bundles/providers/acp-crp-bridge/linux/aarch64/acp-crp-bridge");
        fs::create_dir_all(linux.parent().expect("parent")).expect("mkdir");
        fs::write(&linux, b"ok").expect("write");

        let rewritten = rewrite_bundled_path_for_linux(host.to_string_lossy().as_ref())
            .expect("rewrite should succeed");
        assert_eq!(rewritten, linux.to_string_lossy());
    }

    #[test]
    fn rewrite_bundled_path_for_linux_rewrites_e2e_bundle_provider_paths() {
        let tmp = tempfile::Builder::new()
            .prefix("ctx-e2e-bundles-runtime-probe-")
            .tempdir()
            .expect("tempdir");
        let host = tmp.path().join("providers/codex/macos/aarch64/codex-crp");
        let linux = tmp.path().join("providers/codex/linux/aarch64/codex-crp");
        fs::create_dir_all(linux.parent().expect("parent")).expect("mkdir");
        fs::write(&linux, b"ok").expect("write linux");
        fs::write(tmp.path().join("manifest.json"), "{}").expect("write manifest");

        let rewritten = rewrite_bundled_path_for_linux(host.to_string_lossy().as_ref())
            .expect("rewrite should succeed");
        assert_eq!(rewritten, linux.to_string_lossy());
    }

    #[test]
    fn rewrite_bundled_path_for_linux_rewrites_runtime_flavor_directory() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let host = tmp
            .path()
            .join("bundles/runtimes/node/macos/aarch64/node-v24.12.0-darwin-arm64/bin/node");
        let linux = tmp
            .path()
            .join("bundles/runtimes/node/linux/aarch64/node-v24.12.0-linux-arm64/bin/node");
        fs::create_dir_all(linux.parent().expect("parent")).expect("mkdir");
        fs::write(&linux, b"ok").expect("write");
        let manifest_path = tmp.path().join("bundles/manifest.json");
        fs::write(
            &manifest_path,
            serde_json::json!({
                "version": 1,
                "providers": [],
                "runtimes": [
                    {
                        "id": "node",
                        "os": "linux",
                        "arch": "aarch64",
                        "root": "runtimes/node/linux/aarch64/node-v24.12.0-linux-arm64",
                        "bin": "bin/node"
                    }
                ],
                "images": [],
                "daemons": []
            })
            .to_string(),
        )
        .expect("write manifest");

        let rewritten = rewrite_bundled_path_for_linux(host.to_string_lossy().as_ref())
            .expect("rewrite should succeed");
        assert_eq!(rewritten, linux.to_string_lossy());
    }

    #[test]
    fn rewrite_bundled_path_for_linux_rewrites_runtime_flavor_directory_for_e2e_bundle_root() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let host = tmp
            .path()
            .join("runtimes/node/macos/aarch64/node-v24.12.0-darwin-arm64/bin/node");
        let linux = tmp
            .path()
            .join("runtimes/node/linux/aarch64/node-v24.12.0-linux-arm64/bin/node");
        fs::create_dir_all(linux.parent().expect("parent")).expect("mkdir");
        fs::write(&linux, b"ok").expect("write");
        let manifest_path = tmp.path().join("manifest.json");
        fs::write(
            &manifest_path,
            serde_json::json!({
                "version": 1,
                "providers": [],
                "runtimes": [
                    {
                        "id": "node",
                        "os": "linux",
                        "arch": "aarch64",
                        "root": "runtimes/node/linux/aarch64/node-v24.12.0-linux-arm64",
                        "bin": "bin/node"
                    }
                ],
                "images": [],
                "daemons": []
            })
            .to_string(),
        )
        .expect("write manifest");

        let rewritten = rewrite_bundled_path_for_linux(host.to_string_lossy().as_ref())
            .expect("rewrite should succeed");
        assert_eq!(rewritten, linux.to_string_lossy());
    }

    #[test]
    fn rewrite_container_args_for_linux_rewrites_nested_acp_command_paths() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let host_provider = tmp
            .path()
            .join("bundles/providers/pi/macos/aarch64/pi-acp.js");
        let linux_provider = tmp
            .path()
            .join("bundles/providers/pi/linux/aarch64/pi-acp.js");
        let host_node = tmp
            .path()
            .join("bundles/runtimes/node/macos/aarch64/node-v1/bin/node");
        let linux_node = tmp
            .path()
            .join("bundles/runtimes/node/linux/aarch64/node-v1/bin/node");
        fs::create_dir_all(linux_provider.parent().expect("parent")).expect("mkdir");
        fs::create_dir_all(host_node.parent().expect("parent")).expect("mkdir host node");
        fs::create_dir_all(linux_node.parent().expect("parent")).expect("mkdir");
        fs::write(&linux_provider, b"ok").expect("write");
        fs::write(&host_node, b"ok").expect("write host node");
        fs::write(&linux_node, b"ok").expect("write");

        let raw_acp = format!("{} --foo", host_provider.to_string_lossy());
        let args = vec!["--acp-command".to_string(), raw_acp];
        let mut env = HashMap::new();
        env.insert(
            "PATH".to_string(),
            host_node
                .parent()
                .expect("node dir")
                .to_string_lossy()
                .to_string(),
        );
        let rewritten = rewrite_container_args_for_linux(&args, &env).expect("rewrite args");
        assert_eq!(rewritten.len(), 2);
        let parsed = shlex::split(&rewritten[1]).expect("parse rewritten command");
        assert_eq!(
            parsed,
            vec![
                linux_node.to_string_lossy().to_string(),
                linux_provider.to_string_lossy().to_string(),
                "--foo".to_string(),
            ]
        );
    }

    #[test]
    fn rewrite_container_args_for_linux_preserves_quoted_paths_with_spaces() {
        let tmp = tempfile::Builder::new()
            .prefix("ctx bundles with spaces ")
            .tempdir()
            .expect("tempdir");
        let host_provider = tmp
            .path()
            .join("bundles/providers/pi/macos/aarch64/pi-acp.js");
        let linux_provider = tmp
            .path()
            .join("bundles/providers/pi/linux/aarch64/pi-acp.js");
        let host_node = tmp
            .path()
            .join("bundles/runtimes/node/macos/aarch64/node-v1/bin/node");
        let linux_node = tmp
            .path()
            .join("bundles/runtimes/node/linux/aarch64/node-v1/bin/node");
        fs::create_dir_all(linux_provider.parent().expect("parent")).expect("mkdir");
        fs::create_dir_all(host_node.parent().expect("parent")).expect("mkdir host node");
        fs::create_dir_all(linux_node.parent().expect("parent")).expect("mkdir");
        fs::write(&linux_provider, b"ok").expect("write");
        fs::write(&host_node, b"ok").expect("write host node");
        fs::write(&linux_node, b"ok").expect("write");

        let raw_acp = shlex::try_join(
            [
                host_provider.to_string_lossy().to_string(),
                "--flag".to_string(),
            ]
            .iter()
            .map(String::as_str),
        )
        .expect("quote acp command");
        let args = vec!["--acp-command".to_string(), raw_acp];
        let mut env = HashMap::new();
        env.insert(
            "PATH".to_string(),
            host_node
                .parent()
                .expect("node dir")
                .to_string_lossy()
                .to_string(),
        );
        let rewritten = rewrite_container_args_for_linux(&args, &env).expect("rewrite args");
        assert_eq!(rewritten.len(), 2);
        let parsed = shlex::split(&rewritten[1]).expect("parse rewritten command");
        assert_eq!(
            parsed,
            vec![
                linux_node.to_string_lossy().to_string(),
                linux_provider.to_string_lossy().to_string(),
                "--flag".to_string(),
            ]
        );
    }

    #[test]
    fn rewrite_container_args_for_linux_keeps_explicit_node_binary_for_acp_command() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let host_provider = tmp
            .path()
            .join("bundles/providers/goose/macos/aarch64/goose-acp.js");
        let linux_provider = tmp
            .path()
            .join("bundles/providers/goose/linux/aarch64/goose-acp.js");
        let host_node = tmp
            .path()
            .join("bundles/runtimes/node/macos/aarch64/node-v1/bin/node");
        let linux_node = tmp
            .path()
            .join("bundles/runtimes/node/linux/aarch64/node-v1/bin/node");
        fs::create_dir_all(linux_provider.parent().expect("parent")).expect("mkdir");
        fs::create_dir_all(linux_node.parent().expect("parent")).expect("mkdir");
        fs::write(&linux_provider, b"ok").expect("write provider");
        fs::write(&linux_node, b"ok").expect("write node");

        let raw_acp = shlex::try_join(
            [
                host_node.to_string_lossy().to_string(),
                host_provider.to_string_lossy().to_string(),
                "--flag".to_string(),
            ]
            .iter()
            .map(String::as_str),
        )
        .expect("quote acp command");
        let args = vec!["--acp-command".to_string(), raw_acp];
        let mut env = HashMap::new();
        env.insert(
            "PATH".to_string(),
            linux_node
                .parent()
                .expect("node dir")
                .to_string_lossy()
                .to_string(),
        );

        let rewritten = rewrite_container_args_for_linux(&args, &env).expect("rewrite args");
        let parsed = shlex::split(&rewritten[1]).expect("parse rewritten command");
        assert_eq!(
            parsed,
            vec![
                linux_node.to_string_lossy().to_string(),
                linux_provider.to_string_lossy().to_string(),
                "--flag".to_string(),
            ]
        );
    }

    #[test]
    fn rewrite_container_command_for_linux_uses_explicit_node_binary_for_js_entrypoints() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let node_dir = tmp.path().join("runtimes/node/linux/aarch64/node-v1/bin");
        fs::create_dir_all(&node_dir).expect("mkdir node dir");
        let node_bin = node_dir.join("node");
        fs::write(&node_bin, b"ok").expect("write node");
        let script = tmp
            .path()
            .join("providers/goose/linux/aarch64/goose-acp.js");
        fs::create_dir_all(script.parent().expect("parent")).expect("mkdir script parent");
        fs::write(&script, b"#!/usr/bin/env node\n").expect("write script");

        let mut env = HashMap::new();
        env.insert("PATH".to_string(), node_dir.to_string_lossy().to_string());
        let args = vec!["--flag".to_string()];

        let (command, rewritten_args) =
            rewrite_container_command_for_linux(script.to_string_lossy().as_ref(), &args, &env)
                .expect("rewrite command");

        assert_eq!(command, node_bin.to_string_lossy());
        assert_eq!(
            rewritten_args,
            vec![script.to_string_lossy().to_string(), "--flag".to_string()]
        );
    }

    #[test]
    fn rewrite_bundled_path_for_linux_errors_when_linux_target_missing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let host = tmp
            .path()
            .join("bundles/providers/cursor/macos/aarch64/cursor-agent-acp.js");
        fs::create_dir_all(host.parent().expect("parent")).expect("mkdir");
        fs::write(&host, b"ok").expect("write");

        let err = rewrite_bundled_path_for_linux(host.to_string_lossy().as_ref())
            .expect_err("expected missing linux target error");
        let msg = err.to_string();
        assert!(msg.contains("missing linux bundled path"));
    }

    #[test]
    fn rewrite_bundled_path_for_linux_ignores_managed_install_provider_paths() {
        let path =
            "/tmp/providers/agent-servers/cursor-agent-acp/node_modules/@scope/pkg/dist/bin/app.js";
        let rewritten = rewrite_bundled_path_for_linux(path).expect("rewrite should succeed");
        assert_eq!(rewritten, path);
    }

    #[test]
    fn rewrite_bundled_path_for_linux_ignores_managed_install_runtime_paths() {
        let path = "/tmp/runtimes/node/v24.12.0/bin/node";
        let rewritten = rewrite_bundled_path_for_linux(path).expect("rewrite should succeed");
        assert_eq!(rewritten, path);
    }

    #[test]
    fn rewrite_container_args_for_linux_rejects_invalid_shell_command() {
        let args = vec!["--acp-command".to_string(), "\"unterminated".to_string()];
        let err =
            rewrite_container_args_for_linux(&args, &HashMap::new()).expect_err("expected parse error");
        assert!(err
            .to_string()
            .contains("invalid shell command in --acp-command"));
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

    #[cfg(unix)]
    fn write_probe_script(dir: &tempfile::TempDir, body: &str) -> PathBuf {
        let script = dir.path().join("probe.sh");
        fs::write(&script, format!("#!/bin/sh\n{body}\n")).expect("write script");
        let mut perms = fs::metadata(&script).expect("stat script").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script, perms).expect("chmod script");
        script
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn probe_crp_runtime_launch_accepts_runtime_that_stays_alive() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let script = write_probe_script(&tmp, "cat >/dev/null");
        probe_crp_runtime_launch(
            "codex",
            script.to_string_lossy().to_string(),
            Vec::new(),
            tmp.path().to_path_buf(),
            HashMap::new(),
        )
        .await
        .expect("launch probe should succeed");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn probe_crp_runtime_launch_reports_early_exit_output() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let script = write_probe_script(&tmp, "echo bridge missing >&2\nexit 17");
        let err = probe_crp_runtime_launch(
            "opencode",
            script.to_string_lossy().to_string(),
            Vec::new(),
            tmp.path().to_path_buf(),
            HashMap::new(),
        )
        .await
        .expect_err("launch probe should fail");
        let msg = err.to_string();
        assert!(msg.contains("launch probe"));
        assert!(msg.contains("bridge missing"));
        assert!(msg.contains("exit status: 17"));
    }

    #[cfg(feature = "fuzz_tests")]
    mod fuzz_tests {
        use super::*;
        use rand::rngs::StdRng;
        use rand::{Rng, SeedableRng};
        use serde_json::{json, Map, Value};
        use std::panic::{catch_unwind, AssertUnwindSafe};
        use std::path::PathBuf;

        const ITERATIONS: usize = 200;
        const MAX_DEPTH: u8 = 3;

        fn random_string(rng: &mut StdRng, max_len: usize) -> String {
            const ALPHABET: &[u8] =
                b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_- /.:";
            let len = rng.gen_range(1..=max_len.max(1));
            (0..len)
                .map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())] as char)
                .collect()
        }

        fn random_value(rng: &mut StdRng, depth: u8) -> Value {
            if depth == 0 {
                return match rng.gen_range(0..5) {
                    0 => Value::String(random_string(rng, 16)),
                    1 => Value::Number((rng.gen_range(0..=9999_u64)).into()),
                    2 => Value::Bool(rng.gen_bool(0.5)),
                    3 => Value::Null,
                    _ => json!({"k": "v"}),
                };
            }

            match rng.gen_range(0..6) {
                0 => Value::String(random_string(rng, 24)),
                1 => Value::Array(
                    (0..rng.gen_range(0..=4))
                        .map(|_| random_value(rng, depth - 1))
                        .collect(),
                ),
                2 => {
                    let mut map = Map::new();
                    for _ in 0..rng.gen_range(0..=4) {
                        map.insert(random_string(rng, 12), random_value(rng, depth - 1));
                    }
                    Value::Object(map)
                }
                3 => Value::Bool(rng.gen_bool(0.5)),
                4 => Value::Number((rng.gen_range(0..=9999_u64)).into()),
                _ => Value::Null,
            }
        }

        fn random_event_type(rng: &mut StdRng) -> &'static str {
            const TYPES: &[&str] = &[
                "session.opened",
                "turn.started",
                "message.delta",
                "message.final",
                "reasoning.summary",
                "reasoning.trace",
                "reasoning.trace.final",
                "tool.started",
                "tool.output_delta",
                "tool.completed",
                "turn.completed",
                "session.notice",
                "session.gap",
            ];
            TYPES[rng.gen_range(0..TYPES.len())]
        }

        fn valid_message_delta(seq: u64) -> String {
            json!({
                "v": 1,
                "seq": seq,
                "channel": "control",
                "type": "message.delta",
                "session_id": "s",
                "turn_id": "t",
                "message_id": "m",
                "delta": "hello",
            })
            .to_string()
        }

        fn random_envelope_json(rng: &mut StdRng) -> String {
            if rng.gen_bool(0.15) {
                return random_value(rng, MAX_DEPTH).to_string();
            }

            let mut obj = Map::new();
            obj.insert(
                "seq".to_string(),
                Value::Number((rng.gen_range(0..=10_000_u64)).into()),
            );
            obj.insert(
                "channel".to_string(),
                Value::String(if rng.gen_bool(0.5) {
                    "control".to_string()
                } else {
                    "data".to_string()
                }),
            );
            obj.insert(
                "type".to_string(),
                Value::String(random_event_type(rng).to_string()),
            );
            obj.insert(
                "session_id".to_string(),
                Value::String(random_string(rng, 8)),
            );
            obj.insert("turn_id".to_string(), Value::String(random_string(rng, 8)));
            obj.insert(
                "message_id".to_string(),
                Value::String(random_string(rng, 8)),
            );
            obj.insert("delta".to_string(), Value::String(random_string(rng, 12)));
            obj.insert("content".to_string(), Value::String(random_string(rng, 16)));
            obj.insert(
                "summary_index".to_string(),
                Value::Number((rng.gen_range(0..=8_u64)).into()),
            );
            obj.insert("text".to_string(), Value::String(random_string(rng, 18)));
            obj.insert("chunk".to_string(), Value::String(random_string(rng, 18)));
            obj.insert(
                "tool_call_id".to_string(),
                Value::String(random_string(rng, 8)),
            );
            obj.insert(
                "tool_name".to_string(),
                Value::String(random_string(rng, 10)),
            );
            obj.insert(
                "status".to_string(),
                Value::String(if rng.gen_bool(0.5) {
                    "success".to_string()
                } else {
                    "error".to_string()
                }),
            );
            obj.insert("reason".to_string(), Value::String(random_string(rng, 12)));
            if rng.gen_bool(0.5) {
                obj.insert("details".to_string(), random_value(rng, MAX_DEPTH - 1));
            }
            Value::Object(obj).to_string()
        }

        fn try_parse_and_map(line: &str) {
            let parsed = catch_unwind(AssertUnwindSafe(|| {
                serde_json::from_str::<CrpEventEnvelope>(line)
            }));
            assert!(parsed.is_ok(), "panic while parsing CRP envelope");
            if let Ok(env) = parsed.expect("parse panic already checked") {
                let mut tool_output_cache: HashMap<String, String> = HashMap::new();
                let mut tool_input_cache: HashMap<String, CachedToolInput> = HashMap::new();
                let mapped = catch_unwind(AssertUnwindSafe(|| {
                    map_crp_event(
                        env.event,
                        env.channel,
                        env.seq,
                        &mut tool_output_cache,
                        &mut tool_input_cache,
                    )
                }));
                assert!(mapped.is_ok(), "panic while mapping CRP event");
            }
        }

        #[test]
        fn fuzz_crp_envelope_parsing_and_mapping_do_not_panic() {
            let mut rng = StdRng::seed_from_u64(0xAC1F_2026);

            for idx in 0..ITERATIONS {
                let line = if idx % 10 == 0 {
                    valid_message_delta((idx as u64) + 1)
                } else {
                    random_envelope_json(&mut rng)
                };
                try_parse_and_map(&line);

                let random_slash = if rng.gen_bool(0.5) {
                    format!(
                        "/{} {}",
                        random_string(&mut rng, 10),
                        random_string(&mut rng, 16)
                    )
                } else {
                    random_string(&mut rng, 24)
                };
                let slash =
                    catch_unwind(AssertUnwindSafe(|| parse_crp_slash_command(&random_slash)));
                assert!(slash.is_ok(), "panic while parsing slash command");
            }
        }

        #[test]
        fn fuzz_acp_corpus_replay_does_not_panic() {
            let corpus_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/acp");
            assert!(
                corpus_dir.is_dir(),
                "missing corpus dir: {}",
                corpus_dir.display()
            );

            let mut files = std::fs::read_dir(&corpus_dir)
                .expect("read_dir failed")
                .filter_map(|entry| entry.ok().map(|e| e.path()))
                .filter(|path| path.is_file())
                .collect::<Vec<_>>();
            files.sort();
            assert!(!files.is_empty(), "expected at least one corpus file");

            for file in files {
                let body = std::fs::read_to_string(&file).expect("read corpus file");
                for raw in body.lines() {
                    let line = raw.trim();
                    if line.is_empty() {
                        continue;
                    }
                    try_parse_and_map(line);
                }
            }
        }
    }
}
