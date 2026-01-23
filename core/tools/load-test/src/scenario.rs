use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ScenarioMode {
    Daemon,
    ClientReplay,
    EndToEnd,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct ScenarioSpec {
    #[serde(default = "default_version")]
    pub(crate) version: u32,
    pub(crate) name: String,
    #[serde(default = "default_mode")]
    pub(crate) mode: ScenarioMode,
    #[serde(default = "default_duration_ms")]
    pub(crate) duration_ms: u64,
    #[serde(default = "default_seed")]
    pub(crate) seed: u64,
    #[serde(default)]
    pub(crate) workload: WorkloadSpec,
    #[serde(default)]
    pub(crate) control_plane: ControlPlaneSpec,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct WorkloadSpec {
    #[serde(default = "default_tasks")]
    pub(crate) tasks: u32,
    #[serde(default = "default_subagents_min")]
    pub(crate) subagents_min: u32,
    #[serde(default = "default_subagents_max")]
    pub(crate) subagents_max: u32,
    #[serde(default = "default_message_interval_ms")]
    pub(crate) message_interval_ms: u64,
    #[serde(default = "default_message_size")]
    pub(crate) message_size: usize,
    #[serde(default = "default_tool_calls")]
    pub(crate) tool_calls_per_message: u32,
}

impl Default for WorkloadSpec {
    fn default() -> Self {
        Self {
            tasks: default_tasks(),
            subagents_min: default_subagents_min(),
            subagents_max: default_subagents_max(),
            message_interval_ms: default_message_interval_ms(),
            message_size: default_message_size(),
            tool_calls_per_message: default_tool_calls(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct ControlPlaneSpec {
    #[serde(default = "default_active_snapshot_interval_ms")]
    pub(crate) active_snapshot_interval_ms: u64,
    #[serde(default = "default_session_snapshot_interval_ms")]
    pub(crate) session_snapshot_interval_ms: u64,
    #[serde(default = "default_session_head_interval_ms")]
    pub(crate) session_head_interval_ms: u64,
    #[serde(default = "default_ws_replay_interval_ms")]
    pub(crate) ws_replay_interval_ms: u64,
    #[serde(default = "default_reconnect_interval_ms")]
    pub(crate) reconnect_interval_ms: u64,
    #[serde(default = "default_health_interval_ms")]
    pub(crate) health_interval_ms: u64,
}

impl Default for ControlPlaneSpec {
    fn default() -> Self {
        Self {
            active_snapshot_interval_ms: default_active_snapshot_interval_ms(),
            session_snapshot_interval_ms: default_session_snapshot_interval_ms(),
            session_head_interval_ms: default_session_head_interval_ms(),
            ws_replay_interval_ms: default_ws_replay_interval_ms(),
            reconnect_interval_ms: default_reconnect_interval_ms(),
            health_interval_ms: default_health_interval_ms(),
        }
    }
}

fn default_version() -> u32 {
    1
}

fn default_mode() -> ScenarioMode {
    ScenarioMode::Daemon
}

fn default_duration_ms() -> u64 {
    120_000
}

fn default_seed() -> u64 {
    1
}

fn default_tasks() -> u32 {
    20
}

fn default_subagents_min() -> u32 {
    3
}

fn default_subagents_max() -> u32 {
    5
}

fn default_message_interval_ms() -> u64 {
    1000
}

fn default_message_size() -> usize {
    64
}

fn default_tool_calls() -> u32 {
    1
}

fn default_active_snapshot_interval_ms() -> u64 {
    0
}

fn default_session_snapshot_interval_ms() -> u64 {
    0
}

fn default_ws_replay_interval_ms() -> u64 {
    0
}

fn default_session_head_interval_ms() -> u64 {
    0
}

fn default_reconnect_interval_ms() -> u64 {
    0
}

fn default_health_interval_ms() -> u64 {
    0
}

#[derive(Parser, Debug)]
#[command(author, version, about)]
pub(crate) struct Cli {
    #[arg(long)]
    pub(crate) scenario: Option<PathBuf>,
    #[arg(long, default_value = "http://127.0.0.1:3000")]
    pub(crate) base_url: String,
    #[arg(long)]
    pub(crate) auth_token: Option<String>,
    #[arg(long)]
    pub(crate) workspace_id: Option<String>,
    #[arg(long)]
    pub(crate) out_dir: Option<PathBuf>,
    #[arg(long)]
    pub(crate) mode: Option<ScenarioMode>,
    #[arg(long)]
    pub(crate) duration_ms: Option<u64>,
    #[arg(long)]
    pub(crate) tasks: Option<u32>,
    #[arg(long)]
    pub(crate) subagents_min: Option<u32>,
    #[arg(long)]
    pub(crate) subagents_max: Option<u32>,
    #[arg(long)]
    pub(crate) message_interval_ms: Option<u64>,
    #[arg(long)]
    pub(crate) message_size: Option<usize>,
    #[arg(long, default_value = "fake")]
    pub(crate) provider_id: String,
    #[arg(long, default_value = "fake")]
    pub(crate) model_id: String,
    #[arg(long)]
    pub(crate) tool_calls_per_message: Option<u32>,
    #[arg(long)]
    pub(crate) active_snapshot_interval_ms: Option<u64>,
    #[arg(long)]
    pub(crate) session_snapshot_interval_ms: Option<u64>,
    #[arg(long)]
    pub(crate) session_head_interval_ms: Option<u64>,
    #[arg(long)]
    pub(crate) ws_replay_interval_ms: Option<u64>,
    #[arg(long)]
    pub(crate) reconnect_interval_ms: Option<u64>,
    #[arg(long)]
    pub(crate) health_interval_ms: Option<u64>,
    #[arg(long)]
    pub(crate) daemon_cmd: Option<String>,
    #[arg(long)]
    pub(crate) daemon_data_dir: Option<PathBuf>,
    #[arg(long, default_value = "30000")]
    pub(crate) daemon_ready_timeout_ms: u64,
    #[arg(long)]
    pub(crate) ui_interval_ms: Option<u64>,
}

pub(crate) fn load_scenario(path: Option<&PathBuf>) -> Result<ScenarioSpec> {
    if let Some(path) = path {
        let raw = fs::read_to_string(path).with_context(|| format!("reading {path:?}"))?;
        let mut scenario: ScenarioSpec =
            serde_json::from_str(&raw).context("parsing scenario json")?;
        if scenario.workload.tasks == 0 {
            scenario.workload = WorkloadSpec::default();
        }
        if scenario.workload.subagents_max < scenario.workload.subagents_min {
            scenario.workload.subagents_max = scenario.workload.subagents_min;
        }
        return Ok(scenario);
    }

    Ok(ScenarioSpec {
        version: default_version(),
        name: "baseline".to_string(),
        mode: default_mode(),
        duration_ms: default_duration_ms(),
        seed: default_seed(),
        workload: WorkloadSpec::default(),
        control_plane: ControlPlaneSpec::default(),
    })
}

pub(crate) fn apply_overrides(scenario: &mut ScenarioSpec, cli: &Cli) {
    if let Some(mode) = cli.mode {
        scenario.mode = mode;
    }
    if let Some(duration_ms) = cli.duration_ms {
        scenario.duration_ms = duration_ms;
    }
    if let Some(tasks) = cli.tasks {
        scenario.workload.tasks = tasks;
    }
    if let Some(subagents_min) = cli.subagents_min {
        scenario.workload.subagents_min = subagents_min;
    }
    if let Some(subagents_max) = cli.subagents_max {
        scenario.workload.subagents_max = subagents_max;
    }
    if let Some(message_interval_ms) = cli.message_interval_ms {
        scenario.workload.message_interval_ms = message_interval_ms;
    }
    if let Some(message_size) = cli.message_size {
        scenario.workload.message_size = message_size;
    }
    if let Some(tool_calls) = cli.tool_calls_per_message {
        scenario.workload.tool_calls_per_message = tool_calls;
    }
    if let Some(interval_ms) = cli.active_snapshot_interval_ms {
        scenario.control_plane.active_snapshot_interval_ms = interval_ms;
    }
    if let Some(interval_ms) = cli.session_snapshot_interval_ms {
        scenario.control_plane.session_snapshot_interval_ms = interval_ms;
    }
    if let Some(interval_ms) = cli.session_head_interval_ms {
        scenario.control_plane.session_head_interval_ms = interval_ms;
    }
    if let Some(interval_ms) = cli.ws_replay_interval_ms {
        scenario.control_plane.ws_replay_interval_ms = interval_ms;
    }
    if let Some(interval_ms) = cli.reconnect_interval_ms {
        scenario.control_plane.reconnect_interval_ms = interval_ms;
    }
    if let Some(interval_ms) = cli.health_interval_ms {
        scenario.control_plane.health_interval_ms = interval_ms;
    }
}
