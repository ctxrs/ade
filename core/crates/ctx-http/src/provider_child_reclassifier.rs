use std::collections::{HashMap, HashSet};
#[cfg(target_os = "linux")]
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

#[cfg(target_os = "linux")]
use anyhow::Context;
use anyhow::Result;
use sysinfo::{Pid, System};
use tokio::time::MissedTickBehavior;

use crate::daemon::AppState;
#[cfg(target_os = "linux")]
use crate::tool_cgroup::TOOL_SLICE_UNIT;

const DEFAULT_INTERVAL_MS: u64 = 2_000;
const DEFAULT_CHILD_LIMIT: usize = 2_000;

#[cfg(target_os = "linux")]
const SYSTEMD_TIMEOUT: Duration = Duration::from_secs(3);
#[derive(Debug, Clone)]
struct ReclassifierConfig {
    interval: Duration,
    child_limit: usize,
}

impl ReclassifierConfig {
    fn from_env() -> Self {
        let interval_ms =
            env_u64("CTX_TOOL_RECLASSIFIER_INTERVAL_MS").unwrap_or(DEFAULT_INTERVAL_MS);
        let child_limit = env_u64("CTX_TOOL_RECLASSIFIER_CHILD_LIMIT")
            .map(|v| v as usize)
            .unwrap_or(DEFAULT_CHILD_LIMIT);
        Self {
            interval: Duration::from_millis(interval_ms),
            child_limit,
        }
    }

    fn enabled(&self) -> bool {
        !self.interval.is_zero()
    }
}

#[derive(Debug)]
enum Backend {
    #[cfg(target_os = "linux")]
    SystemdRun,
    #[cfg(target_os = "linux")]
    CgroupProcs {
        tool_slice: PathBuf,
    },
    Disabled {
        reason: String,
    },
}

pub fn spawn_provider_child_reclassifier(state: Arc<AppState>) {
    let cfg = ReclassifierConfig::from_env();
    if !cfg.enabled() {
        return;
    }

    #[cfg(not(target_os = "linux"))]
    {
        tracing::info!("provider child reclassifier disabled: linux only");
        let _ = state;
        return;
    }

    #[cfg(target_os = "linux")]
    {
        let mut shutdown_rx = state.shutdown_tx.subscribe();
        tokio::spawn(async move {
            let mut system = System::new_all();
            let mut classified: HashSet<u32> = HashSet::new();
            let mut backend = detect_backend().await;

            if let Backend::Disabled { reason } = &backend {
                tracing::info!("provider child reclassifier disabled: {reason}");
                return;
            }

            let mut ticker = tokio::time::interval(cfg.interval);
            ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

            if let Err(err) =
                reclassify_once(&state, &mut system, &cfg, &mut classified, &mut backend).await
            {
                tracing::warn!("provider child reclassifier tick failed: {err:#}");
            }

            loop {
                tokio::select! {
                    _ = shutdown_rx.recv() => break,
                    _ = ticker.tick() => {
                        if let Err(err) =
                            reclassify_once(&state, &mut system, &cfg, &mut classified, &mut backend).await
                        {
                            tracing::warn!("provider child reclassifier tick failed: {err:#}");
                        }
                    }
                }
            }
        });
    }
}

async fn reclassify_once(
    state: &Arc<AppState>,
    system: &mut System,
    cfg: &ReclassifierConfig,
    classified: &mut HashSet<u32>,
    backend: &mut Backend,
) -> Result<()> {
    if matches!(backend, Backend::Disabled { .. }) {
        return Ok(());
    }

    let provider_pids = list_provider_pids(state).await;
    if provider_pids.is_empty() {
        classified.clear();
        return Ok(());
    }

    system.refresh_processes();
    let child_pids = collect_child_pids(system, &provider_pids, cfg.child_limit);

    classified.retain(|pid| child_pids.contains(pid));

    for pid in child_pids.iter() {
        if classified.contains(pid) {
            continue;
        }
        if pid_in_tool_slice(*pid) {
            classified.insert(*pid);
            continue;
        }
        if let Err(err) = classify_pid(*pid, backend).await {
            tracing::warn!(
                pid = *pid,
                "failed to reclassify provider child pid: {err:#}"
            );
            if matches!(backend, Backend::Disabled { .. }) {
                break;
            }
        } else {
            classified.insert(*pid);
        }
    }

    Ok(())
}

async fn list_provider_pids(state: &Arc<AppState>) -> Vec<u32> {
    let providers = {
        let providers = state.providers.lock().await;
        providers.values().cloned().collect::<Vec<_>>()
    };
    let mut pids = Vec::new();
    for adapter in providers {
        pids.extend(
            adapter
                .list_processes()
                .await
                .into_iter()
                .map(|proc| proc.pid),
        );
    }
    pids
}

fn collect_child_pids(system: &System, provider_pids: &[u32], limit: usize) -> HashSet<u32> {
    if provider_pids.is_empty() || limit == 0 {
        return HashSet::new();
    }

    let mut task_pids = HashSet::new();
    for (pid, process) in system.processes() {
        if let Some(tasks) = process.tasks() {
            for task_pid in tasks {
                if task_pid != pid {
                    task_pids.insert(*task_pid);
                }
            }
        }
    }

    let mut children: HashMap<Pid, Vec<Pid>> = HashMap::new();
    for (pid, process) in system.processes() {
        if task_pids.contains(pid) {
            continue;
        }
        if let Some(parent) = process.parent() {
            if task_pids.contains(&parent) {
                continue;
            }
            children.entry(parent).or_default().push(*pid);
        }
    }

    let mut output = HashSet::new();
    let mut stack: Vec<Pid> = provider_pids
        .iter()
        .map(|pid| Pid::from_u32(*pid))
        .collect();
    while let Some(parent) = stack.pop() {
        if let Some(kids) = children.get(&parent) {
            for child in kids {
                let child_pid = child.as_u32();
                if output.insert(child_pid) {
                    if output.len() >= limit {
                        return output;
                    }
                    stack.push(*child);
                }
            }
        }
    }

    output
}

#[cfg(target_os = "linux")]
fn pid_in_tool_slice(pid: u32) -> bool {
    let path = format!("/proc/{pid}/cgroup");
    std::fs::read_to_string(path)
        .ok()
        .map(|contents| contents.lines().any(|line| line.contains(TOOL_SLICE_UNIT)))
        .unwrap_or(false)
}

#[cfg(not(target_os = "linux"))]
fn pid_in_tool_slice(_pid: u32) -> bool {
    false
}

#[cfg(target_os = "linux")]
async fn classify_pid(pid: u32, backend: &mut Backend) -> Result<()> {
    match backend {
        Backend::SystemdRun => match attach_via_systemd(pid).await {
            Ok(()) => Ok(()),
            Err(err) => {
                if let Some(path) = tool_slice_path() {
                    if let Ok(()) = attach_via_cgroup(path.clone(), pid) {
                        *backend = Backend::CgroupProcs { tool_slice: path };
                        return Ok(());
                    }
                }
                *backend = Backend::Disabled {
                    reason: format!("systemd-run failed: {err:#}"),
                };
                Err(err)
            }
        },
        Backend::CgroupProcs { tool_slice } => attach_via_cgroup(tool_slice.clone(), pid),
        Backend::Disabled { .. } => Ok(()),
    }
}

#[cfg(not(target_os = "linux"))]
async fn classify_pid(_pid: u32, _backend: &mut Backend) -> Result<()> {
    Ok(())
}

#[cfg(target_os = "linux")]
async fn detect_backend() -> Backend {
    if systemd_run_supports_pid().await {
        return Backend::SystemdRun;
    }
    if let Some(path) = tool_slice_path() {
        return Backend::CgroupProcs { tool_slice: path };
    }
    Backend::Disabled {
        reason: "systemd-run lacks --pid and no cgroup v2 tool slice available".to_string(),
    }
}

#[cfg(not(target_os = "linux"))]
async fn detect_backend() -> Backend {
    Backend::Disabled {
        reason: "unsupported OS".to_string(),
    }
}

#[cfg(target_os = "linux")]
async fn attach_via_systemd(pid: u32) -> Result<()> {
    let unit = format!("ctx-tool-{}", pid);
    let mut cmd = tokio::process::Command::new("systemd-run");
    cmd.arg("--user")
        .arg("--scope")
        .arg("--unit")
        .arg(&unit)
        .arg("--slice")
        .arg(TOOL_SLICE_UNIT)
        .arg("--pid")
        .arg(pid.to_string());
    run_command(&mut cmd, "systemd-run").await
}

#[cfg(target_os = "linux")]
fn attach_via_cgroup(tool_slice: PathBuf, pid: u32) -> Result<()> {
    if !tool_slice.exists() {
        std::fs::create_dir_all(&tool_slice)
            .with_context(|| format!("create tool slice {}", tool_slice.display()))?;
    }
    let procs_path = tool_slice.join("cgroup.procs");
    std::fs::write(&procs_path, format!("{pid}\n"))
        .with_context(|| format!("write {}", procs_path.display()))?;
    Ok(())
}

#[cfg(target_os = "linux")]
async fn run_command(cmd: &mut tokio::process::Command, label: &str) -> Result<()> {
    let output = tokio::time::timeout(SYSTEMD_TIMEOUT, cmd.output())
        .await
        .context("command timed out")?
        .with_context(|| format!("running {label}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    anyhow::bail!(
        "{label} failed ({}): {}{}",
        output.status,
        stderr,
        if stdout.trim().is_empty() {
            String::new()
        } else {
            format!(" ({stdout})")
        }
    );
}

#[cfg(target_os = "linux")]
async fn systemd_run_supports_pid() -> bool {
    let mut cmd = tokio::process::Command::new("systemd-run");
    cmd.arg("--help");
    let output = tokio::time::timeout(SYSTEMD_TIMEOUT, cmd.output()).await;
    let Ok(Ok(output)) = output else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let help = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
    .to_lowercase();
    help.contains("--pid")
}

#[cfg(target_os = "linux")]
fn tool_slice_path() -> Option<PathBuf> {
    let contents = std::fs::read_to_string("/proc/self/cgroup").ok()?;
    let rel = contents
        .lines()
        .find_map(|line| line.strip_prefix("0::"))?
        .trim();
    if rel.is_empty() {
        return None;
    }
    if !cgroup_v2_available() {
        return None;
    }
    let mut path = PathBuf::from("/sys/fs/cgroup").join(rel.trim_start_matches('/'));
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    if file_name.ends_with(".scope") {
        path.pop();
    }
    path.push(TOOL_SLICE_UNIT);
    Some(path)
}

#[cfg(target_os = "linux")]
fn cgroup_v2_available() -> bool {
    Path::new("/sys/fs/cgroup/cgroup.controllers").exists()
}

fn env_u64(key: &str) -> Option<u64> {
    std::env::var(key).ok()?.parse::<u64>().ok()
}
