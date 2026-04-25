use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};

use anyhow::{Context, Result};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::{broadcast, Mutex};

use ctx_core::ids::WorkspaceId;
use ctx_core::models::Workspace;
use ctx_harness_setup::{
    HarnessSetupDownloadStatus, HarnessSetupLogLevel, HarnessSetupObserver, HarnessSetupPhase,
    HarnessSetupProgressUpdate,
};

use crate::ops_events::{OpsEvent, OpsEvents};
use crate::perf_telemetry::{PerfMetric, PerfMetricKind, PerfTelemetry};
use crate::settings::{ExecutionMode, ExecutionSettings};
use ctx_workspace_runtime::HarnessRuntimeManager;

mod coordinator;
mod launch_state;
mod progress;
mod runtime_prewarm;
mod startup_prewarm;
mod support;
#[cfg(test)]
mod tests;
mod warmup_coordination;
mod workspace_launch;

use launch_state::{
    seed_runtime_prewarm_initial_state, seed_workspace_launch_initial_state, CoordinatorState,
    LaunchJob, LaunchTerminalMutation, PrewarmGate, StartupPrewarmMetadata,
};
use progress::{
    bundled_image_fingerprint, format_error_chain, format_ts, needs_prewarm,
    normalize_container_engine_ready_for_gate, phase_label, read_prewarm_metadata,
    write_prewarm_metadata, LaunchObserver,
};
use warmup_coordination::{
    DefaultWarmupOperations, LaunchPrewarmCoordinator, PrewarmJobRegistry, RequestedPrewarmScope,
    SharedPrewarmLaunchJob, SharedWarmupOperations,
};

const JOB_LOG_CAP: usize = 400;
const JOB_HISTORY_CAP: usize = 128;
const LAUNCH_EVENT_CHANNEL_CAP: usize = 256;

fn runtime_prewarm_ready_phase_message(
    runtime_requested: bool,
    runtime_kind: &crate::settings::ContainerRuntimeKind,
    launch_ready: bool,
) -> &'static str {
    if runtime_requested {
        ctx_harness_runtime::runtime_prewarm_ready_message(runtime_kind, launch_ready)
    } else {
        "container builder is ready"
    }
}

fn lock_or_recover<'a, T>(mutex: &'a StdMutex<T>, name: &str) -> std::sync::MutexGuard<'a, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::warn!(mutex = name, "mutex poisoned; recovering");
            poisoned.into_inner()
        }
    }
}

#[cfg(test)]
mod ready_message_tests {
    use super::runtime_prewarm_ready_phase_message;
    use crate::settings::ContainerRuntimeKind;

    #[test]
    fn runtime_prewarm_ready_phase_message_uses_runtime_specific_semantics() {
        assert_eq!(
            runtime_prewarm_ready_phase_message(
                true,
                &ContainerRuntimeKind::SharedVmContainer,
                false,
            ),
            "shared VM runtime artifacts are ready; launch image loads when the shared VM starts"
        );
        assert_eq!(
            runtime_prewarm_ready_phase_message(
                true,
                &ContainerRuntimeKind::SharedVmContainer,
                true,
            ),
            "shared VM substrate and launch image are ready"
        );
        assert_eq!(
            runtime_prewarm_ready_phase_message(true, &ContainerRuntimeKind::NativeContainer, true,),
            "local sandbox runtime and launch image are ready"
        );
    }

    #[test]
    fn runtime_prewarm_ready_phase_message_preserves_builder_message() {
        assert_eq!(
            runtime_prewarm_ready_phase_message(
                false,
                &ContainerRuntimeKind::SharedVmContainer,
                false,
            ),
            "container builder is ready"
        );
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionSetupJobKind {
    StartupPrewarm,
    WorkspaceLaunch,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum RuntimePrewarmScope {
    #[default]
    Runtime,
    LaunchReady,
    Builder,
    All,
}

impl RuntimePrewarmScope {
    fn includes_runtime(self) -> bool {
        matches!(self, Self::Runtime | Self::LaunchReady | Self::All)
    }

    fn includes_builder(self) -> bool {
        matches!(self, Self::Builder | Self::All)
    }

    fn requires_launch_ready_runtime(self) -> bool {
        matches!(self, Self::LaunchReady | Self::All)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionLaunchState {
    Running,
    Ready,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionLaunchLogLine {
    pub seq: u64,
    pub ts: String,
    pub phase: HarnessSetupPhase,
    pub level: HarnessSetupLogLevel,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionLaunchPhaseStatus {
    pub phase: HarnessSetupPhase,
    pub started_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionLaunchSnapshot {
    pub job_id: String,
    pub workspace_id: String,
    pub kind: ExecutionSetupJobKind,
    pub state: ExecutionLaunchState,
    pub created_at: String,
    pub started_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_phase: Option<HarnessSetupPhase>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_step_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress_pct: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eta_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_download: Option<HarnessSetupDownloadStatus>,
    pub phases: Vec<ExecutionLaunchPhaseStatus>,
    pub logs: Vec<ExecutionLaunchLogLine>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
// EXCEPTION: clippy::enum_variant_names — these stable serde tags are part of the
// execution launch stream API, so the shared `Launch*` prefix is deliberate.
#[allow(clippy::enum_variant_names)]
pub enum ExecutionLaunchStreamEvent {
    LaunchSnapshot {
        snapshot: ExecutionLaunchSnapshot,
    },
    LaunchLog {
        job_id: String,
        line: ExecutionLaunchLogLine,
    },
    LaunchComplete {
        snapshot: ExecutionLaunchSnapshot,
    },
    LaunchError {
        snapshot: ExecutionLaunchSnapshot,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum StartupPrewarmState {
    #[default]
    Idle,
    Running,
    Ready,
    Error,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StartupPrewarmSnapshot {
    pub state: StartupPrewarmState,
    pub target_image: String,
    pub needs_prewarm: bool,
    pub machine_ready: bool,
    pub image_present: bool,
    pub image_ref_changed: bool,
    pub bundled_image_digest_changed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_attempt_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_success_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub struct ExecutionSetupCoordinator {
    data_root: PathBuf,
    harness: Arc<HarnessRuntimeManager>,
    perf_telemetry: PerfTelemetry,
    ops_events: OpsEvents,
    inner: Mutex<CoordinatorState>,
    prewarm: LaunchPrewarmCoordinator,
}
