use anyhow::{Context, Result};

use crate::daemon::AppState;
use crate::resource_utilization::SystemSnapshot;
use crate::settings::{
    PublicToolLimitsLimits, PublicToolLimitsSettings, ResourceGovernanceMode, Settings,
    ToolLimitsSettings,
};

#[cfg(target_os = "linux")]
use tokio::process::Command;
#[cfg(target_os = "linux")]
use tokio::time::timeout;

#[cfg(target_os = "linux")]
const SYSTEMD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);
#[cfg(target_os = "linux")]
pub const TOOL_SLICE_UNIT: &str = "ctx-tools.slice";

const TOOL_MEMORY_MAX_FRACTION: f64 = 0.9;
const TOOL_MEMORY_HIGH_FRACTION: f64 = 0.9;
const TOOL_MEMORY_MIN_MAX_MB: u64 = 1024;
const TOOL_MEMORY_MIN_HIGH_MB: u64 = 512;

#[derive(Debug, Clone, PartialEq)]
pub struct EffectiveToolLimits {
    pub memory_high_mb: u32,
    pub memory_max_mb: u32,
}

pub fn compute_effective_limits(
    settings: &ToolLimitsSettings,
    system: &SystemSnapshot,
) -> Option<EffectiveToolLimits> {
    if !settings.enabled {
        return None;
    }

    let total_mb = (system.memory_total_bytes / (1024 * 1024)).max(1);
    let mut memory_max_mb = match settings.mode {
        ResourceGovernanceMode::Auto => {
            ((total_mb as f64) * TOOL_MEMORY_MAX_FRACTION).round() as u64
        }
        ResourceGovernanceMode::Custom => settings.memory_max_mb.unwrap_or(0) as u64,
    };
    if memory_max_mb == 0 {
        memory_max_mb = ((total_mb as f64) * TOOL_MEMORY_MAX_FRACTION).round() as u64;
    }
    memory_max_mb = memory_max_mb.max(TOOL_MEMORY_MIN_MAX_MB).min(total_mb);

    let mut memory_high_mb = match settings.mode {
        ResourceGovernanceMode::Auto => {
            ((memory_max_mb as f64) * TOOL_MEMORY_HIGH_FRACTION).round() as u64
        }
        ResourceGovernanceMode::Custom => settings.memory_high_mb.unwrap_or(0) as u64,
    };
    if memory_high_mb == 0 {
        memory_high_mb = ((memory_max_mb as f64) * TOOL_MEMORY_HIGH_FRACTION).round() as u64;
    }
    memory_high_mb = memory_high_mb
        .max(TOOL_MEMORY_MIN_HIGH_MB)
        .min(memory_max_mb);

    Some(EffectiveToolLimits {
        memory_high_mb: memory_high_mb as u32,
        memory_max_mb: memory_max_mb as u32,
    })
}

pub fn public_settings(
    settings: &ToolLimitsSettings,
    effective: Option<&EffectiveToolLimits>,
) -> PublicToolLimitsSettings {
    PublicToolLimitsSettings {
        enabled: settings.enabled,
        mode: settings.mode.clone(),
        memory_high_mb: settings.memory_high_mb,
        memory_max_mb: settings.memory_max_mb,
        effective: effective.map(|lim| PublicToolLimitsLimits {
            memory_high_mb: lim.memory_high_mb,
            memory_max_mb: lim.memory_max_mb,
        }),
    }
}

pub async fn build_public_settings(
    state: &AppState,
    settings: &Settings,
) -> Option<PublicToolLimitsSettings> {
    let cfg = settings.tool_limits.as_ref()?;
    let (system, _disks, _cache_age_ms) = {
        let mut sampler = state.resource_sampler.lock().await;
        sampler.system_snapshot()
    };
    let effective = compute_effective_limits(cfg, &system);
    Some(public_settings(cfg, effective.as_ref()))
}

pub async fn apply_settings(state: &AppState, settings: &Settings) -> Result<()> {
    let cfg = settings.tool_limits.clone().unwrap_or_default();
    if !cfg.enabled {
        return Ok(());
    }

    #[cfg(not(target_os = "linux"))]
    {
        tracing::warn!("tool cgroup limits are not supported on this OS");
        return Ok(());
    }

    #[cfg(target_os = "linux")]
    {
        if !systemd_user_available().await {
            tracing::warn!("systemd user session unavailable; tool cgroup limits not applied");
            return Ok(());
        }
        let (system, _disks, _cache_age_ms) = {
            let mut sampler = state.resource_sampler.lock().await;
            sampler.system_snapshot()
        };
        let Some(limits) = compute_effective_limits(&cfg, &system) else {
            return Ok(());
        };
        ensure_tool_slice(&limits).await?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
async fn ensure_tool_slice(limits: &EffectiveToolLimits) -> Result<()> {
    if let Err(err) = set_slice_properties(limits).await {
        tracing::debug!("tool slice property update failed; attempting bootstrap: {err:#}");
        bootstrap_slice().await?;
        set_slice_properties(limits).await?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
async fn systemd_user_available() -> bool {
    let systemd_run_ok = Command::new("systemd-run")
        .arg("--version")
        .output()
        .await
        .map(|out| out.status.success())
        .unwrap_or(false);
    if !systemd_run_ok {
        return false;
    }
    Command::new("systemctl")
        .arg("--user")
        .arg("show-environment")
        .output()
        .await
        .map(|out| out.status.success())
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
async fn set_slice_properties(limits: &EffectiveToolLimits) -> Result<()> {
    let mut cmd = Command::new("systemctl");
    cmd.arg("--user")
        .arg("set-property")
        .arg("--runtime")
        .arg(TOOL_SLICE_UNIT)
        .arg(format!("MemoryHigh={}M", limits.memory_high_mb))
        .arg(format!("MemoryMax={}M", limits.memory_max_mb));
    run_command(&mut cmd, "systemctl set-property").await
}

#[cfg(target_os = "linux")]
async fn bootstrap_slice() -> Result<()> {
    let mut cmd = Command::new("systemd-run");
    cmd.arg("--user")
        .arg("--quiet")
        .arg("--scope")
        .arg("--slice")
        .arg(TOOL_SLICE_UNIT)
        .arg("/bin/true");
    run_command(&mut cmd, "systemd-run").await
}

#[cfg(target_os = "linux")]
async fn run_command(cmd: &mut Command, label: &str) -> Result<()> {
    let output = timeout(SYSTEMD_TIMEOUT, cmd.output())
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
