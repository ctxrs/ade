use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::future::Future;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use clap::{Parser, ValueEnum};
use futures::{SinkExt, StreamExt};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::process::Command;
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};
use url::Url;

use ctx_client::{
    Client, ClientTelemetryBatch, ClientTelemetryMetric, CreateSessionRequest, CreateTaskRequest,
    DaemonConfig, PerfMetricKind, PostMessageRequest, TelemetrySummaryParams,
    WorkspaceActiveSnapshotParams,
};
use ctx_core::ids::{SessionId, TaskId, WorkspaceId};
use ctx_core::models::{
    SessionEventType, WorkspaceActiveSnapshotClientMessage,
    WorkspaceActiveSnapshotSessionSubscription,
};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
enum ScenarioMode {
    Daemon,
    ClientReplay,
    EndToEnd,
}

#[derive(Debug, Deserialize, Serialize)]
struct ScenarioSpec {
    #[serde(default = "default_version")]
    version: u32,
    name: String,
    #[serde(default = "default_mode")]
    mode: ScenarioMode,
    #[serde(default = "default_duration_ms")]
    duration_ms: u64,
    #[serde(default = "default_seed")]
    seed: u64,
    #[serde(default)]
    workload: WorkloadSpec,
    #[serde(default)]
    control_plane: ControlPlaneSpec,
}

#[derive(Debug, Deserialize, Serialize)]
struct WorkloadSpec {
    #[serde(default = "default_tasks")]
    tasks: u32,
    #[serde(default = "default_subagents_min")]
    subagents_min: u32,
    #[serde(default = "default_subagents_max")]
    subagents_max: u32,
    #[serde(default = "default_message_interval_ms")]
    message_interval_ms: u64,
    #[serde(default = "default_message_size")]
    message_size: usize,
    #[serde(default = "default_tool_calls")]
    tool_calls_per_message: u32,
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
struct ControlPlaneSpec {
    #[serde(default = "default_active_snapshot_interval_ms")]
    active_snapshot_interval_ms: u64,
    #[serde(default = "default_session_snapshot_interval_ms")]
    session_snapshot_interval_ms: u64,
    #[serde(default = "default_session_head_interval_ms")]
    session_head_interval_ms: u64,
    #[serde(default = "default_ws_replay_interval_ms")]
    ws_replay_interval_ms: u64,
    #[serde(default = "default_reconnect_interval_ms")]
    reconnect_interval_ms: u64,
    #[serde(default = "default_health_interval_ms")]
    health_interval_ms: u64,
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

const ACTIVE_SNAPSHOT_LIMIT: u32 = 50;
const SESSION_SNAPSHOT_LIMIT: u32 = 40;
const SESSION_HEAD_LIMIT: u32 = 40;
const WS_REPLAY_READ_TIMEOUT_MS: u64 = 500;
const HOT_READ_P95_TARGET_MS: f64 = 20.0;

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Cli {
    #[arg(long)]
    scenario: Option<PathBuf>,
    #[arg(long, default_value = "http://127.0.0.1:3000")]
    base_url: String,
    #[arg(long)]
    auth_token: Option<String>,
    #[arg(long)]
    workspace_id: Option<String>,
    #[arg(long)]
    out_dir: Option<PathBuf>,
    #[arg(long)]
    mode: Option<ScenarioMode>,
    #[arg(long)]
    duration_ms: Option<u64>,
    #[arg(long)]
    tasks: Option<u32>,
    #[arg(long)]
    subagents_min: Option<u32>,
    #[arg(long)]
    subagents_max: Option<u32>,
    #[arg(long)]
    message_interval_ms: Option<u64>,
    #[arg(long)]
    message_size: Option<usize>,
    #[arg(long, default_value = "fake")]
    provider_id: String,
    #[arg(long, default_value = "fake")]
    model_id: String,
    #[arg(long)]
    tool_calls_per_message: Option<u32>,
    #[arg(long)]
    active_snapshot_interval_ms: Option<u64>,
    #[arg(long)]
    session_snapshot_interval_ms: Option<u64>,
    #[arg(long)]
    session_head_interval_ms: Option<u64>,
    #[arg(long)]
    ws_replay_interval_ms: Option<u64>,
    #[arg(long)]
    reconnect_interval_ms: Option<u64>,
    #[arg(long)]
    health_interval_ms: Option<u64>,
    #[arg(long)]
    daemon_cmd: Option<String>,
    #[arg(long)]
    daemon_data_dir: Option<PathBuf>,
    #[arg(long, default_value = "30000")]
    daemon_ready_timeout_ms: u64,
    #[arg(long)]
    ui_interval_ms: Option<u64>,
}

#[derive(Default)]
struct Metrics {
    http_ms: Mutex<Vec<f64>>,
    first_chunk_ms: Mutex<Vec<f64>>,
    done_ms: Mutex<Vec<f64>>,
    sent: Mutex<u64>,
    done: Mutex<u64>,
    errors: Mutex<u64>,
}

#[derive(Default)]
struct ControlPlaneEndpointMetrics {
    api_ms: Mutex<Vec<f64>>,
    sent: Mutex<u64>,
    errors: Mutex<u64>,
}

#[derive(Default)]
struct ControlPlaneMetrics {
    totals: ControlPlaneEndpointMetrics,
    health: ControlPlaneEndpointMetrics,
    active_snapshot: ControlPlaneEndpointMetrics,
    session_snapshot: ControlPlaneEndpointMetrics,
    session_head: ControlPlaneEndpointMetrics,
    ws_replay: ControlPlaneEndpointMetrics,
    reconnect: ControlPlaneEndpointMetrics,
}

#[derive(Clone, Copy)]
enum ControlPlaneEndpoint {
    Health,
    ActiveSnapshot,
    SessionSnapshot,
    SessionHead,
    WsReplay,
    ReconnectCatchup,
}

impl ControlPlaneMetrics {
    fn endpoint_metrics(&self, endpoint: ControlPlaneEndpoint) -> &ControlPlaneEndpointMetrics {
        match endpoint {
            ControlPlaneEndpoint::Health => &self.health,
            ControlPlaneEndpoint::ActiveSnapshot => &self.active_snapshot,
            ControlPlaneEndpoint::SessionSnapshot => &self.session_snapshot,
            ControlPlaneEndpoint::SessionHead => &self.session_head,
            ControlPlaneEndpoint::WsReplay => &self.ws_replay,
            ControlPlaneEndpoint::ReconnectCatchup => &self.reconnect,
        }
    }
}

#[derive(Default)]
struct UiMetrics {
    api_ms: Mutex<Vec<f64>>,
    sent: Mutex<u64>,
    errors: Mutex<u64>,
}

#[derive(Default)]
struct PendingState {
    pending: HashMap<String, Instant>,
    first_chunked: HashSet<String>,
}

#[derive(Serialize)]
struct Summary {
    name: String,
    mode: ScenarioMode,
    duration_ms: u64,
    sent: u64,
    done: u64,
    errors: u64,
    http_ms: Percentiles,
    first_chunk_ms: Percentiles,
    done_ms: Percentiles,
    #[serde(skip_serializing_if = "Option::is_none")]
    golden: Option<GoldenSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    control_plane: Option<ControlPlaneSummary>,
}

#[derive(Serialize)]
struct ControlPlaneSummary {
    sent: u64,
    errors: u64,
    api_ms: Percentiles,
    endpoints: ControlPlaneEndpointSummaryBreakdown,
}

#[derive(Serialize)]
struct GoldenSummary {
    #[serde(skip_serializing_if = "Option::is_none")]
    hot_read_p95_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hot_read_p95_target_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hot_read_p95_ok: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reconnect_catchup_p95_ms: Option<f64>,
}

#[derive(Serialize)]
struct UiSummary {
    sent: u64,
    errors: u64,
    api_ms: Percentiles,
}

#[derive(Serialize)]
struct ControlPlaneEndpointSummary {
    sent: u64,
    errors: u64,
    api_ms: Percentiles,
}

#[derive(Serialize)]
struct ControlPlaneEndpointSummaryBreakdown {
    health: ControlPlaneEndpointSummary,
    active_snapshot: ControlPlaneEndpointSummary,
    session_snapshot: ControlPlaneEndpointSummary,
    session_head: ControlPlaneEndpointSummary,
    ws_replay: ControlPlaneEndpointSummary,
    reconnect: ControlPlaneEndpointSummary,
}

#[derive(Serialize)]
struct CombinedSummary {
    name: String,
    mode: ScenarioMode,
    duration_ms: u64,
    run_id: Option<String>,
    daemon: Summary,
    ui: Option<UiSummary>,
    ui_telemetry: Option<Value>,
}

#[derive(Serialize, Default)]
struct Percentiles {
    p50: f64,
    p95: f64,
    p99: f64,
    max: f64,
}

#[derive(Serialize)]
struct EventRecord<'a> {
    ts_ms: u128,
    kind: &'a str,
    value_ms: Option<f64>,
    detail: Option<String>,
}

struct DaemonRunOutput {
    out_dir: PathBuf,
    summary_path: PathBuf,
    summary: Summary,
    tasks: usize,
    sessions: usize,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut scenario = load_scenario(cli.scenario.as_ref())?;
    apply_overrides(&mut scenario, &cli);

    match scenario.mode {
        ScenarioMode::Daemon => run_daemon_mode(&cli, &scenario).await,
        ScenarioMode::ClientReplay => Err(anyhow!(
            "client_replay mode not implemented yet; use daemon mode first"
        )),
        ScenarioMode::EndToEnd => run_end_to_end_mode(&cli, &scenario).await,
    }
}

fn load_scenario(path: Option<&PathBuf>) -> Result<ScenarioSpec> {
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

fn apply_overrides(scenario: &mut ScenarioSpec, cli: &Cli) {
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

async fn run_daemon_mode(cli: &Cli, scenario: &ScenarioSpec) -> Result<()> {
    let out_dir = resolve_out_dir(&scenario.name, cli.out_dir.as_ref())?;
    let client = Arc::new(Client::new(DaemonConfig {
        base_url: cli.base_url.clone(),
        auth_token: cli.auth_token.clone(),
    })?);
    let workspace_id = resolve_workspace(&client, cli.workspace_id.as_deref()).await?;
    let output = run_daemon_workload(client, workspace_id, scenario, cli, out_dir).await?;

    println!("Load test complete: {}", scenario.name);
    println!("Tasks: {}, Sessions: {}", output.tasks, output.sessions);
    println!(
        "Sent: {}, Done: {}, Errors: {}",
        output.summary.sent, output.summary.done, output.summary.errors
    );
    println!("HTTP p95: {:.1}ms", output.summary.http_ms.p95);
    println!("Done p95: {:.1}ms", output.summary.done_ms.p95);
    if let Some(golden) = output.summary.golden.as_ref() {
        if let Some(hot_read) = golden.hot_read_p95_ms {
            let target = golden
                .hot_read_p95_target_ms
                .unwrap_or(HOT_READ_P95_TARGET_MS);
            println!("Hot read p95: {:.1}ms (target {:.1}ms)", hot_read, target);
        }
        if let Some(reconnect) = golden.reconnect_catchup_p95_ms {
            println!("Reconnect catch-up p95: {:.1}ms", reconnect);
        }
    }
    println!("Summary: {}", output.summary_path.display());

    Ok(())
}

async fn run_daemon_workload(
    client: Arc<Client>,
    workspace_id: WorkspaceId,
    scenario: &ScenarioSpec,
    cli: &Cli,
    out_dir: PathBuf,
) -> Result<DaemonRunOutput> {
    fs::create_dir_all(&out_dir).context("creating output dir")?;
    let events = EventWriter::new(out_dir.join("events.ndjson"))?;

    let mut rng = StdRng::seed_from_u64(scenario.seed);
    let (tasks, sessions) =
        setup_tasks_and_sessions(&client, workspace_id, scenario, &mut rng, cli).await?;

    let metrics = Arc::new(Metrics::default());
    let pending = Arc::new(Mutex::new(PendingState::default()));

    let ws_handle = spawn_ws_listener(
        &client,
        cli.auth_token.as_deref(),
        workspace_id,
        sessions.clone(),
        metrics.clone(),
        pending.clone(),
        events.clone(),
    )
    .await?;

    let duration = Duration::from_millis(scenario.duration_ms.max(1));
    let deadline = Instant::now() + duration;
    let mut workers = Vec::new();
    let control_plane_metrics = Arc::new(ControlPlaneMetrics::default());
    let mut control_workers = Vec::new();

    if scenario.control_plane.health_interval_ms > 0 {
        let interval = Duration::from_millis(scenario.control_plane.health_interval_ms.max(1));
        let metrics = control_plane_metrics.clone();
        let client = client.clone();
        let mut events = events.clone();
        let worker = tokio::spawn(async move {
            while Instant::now() < deadline {
                record_control_call(
                    &metrics,
                    &mut events,
                    ControlPlaneEndpoint::Health,
                    "control_health",
                    client.get_health(),
                )
                .await;
                sleep(interval).await;
            }
        });
        control_workers.push(worker);
    }

    if scenario.control_plane.active_snapshot_interval_ms > 0 {
        let interval =
            Duration::from_millis(scenario.control_plane.active_snapshot_interval_ms.max(1));
        let metrics = control_plane_metrics.clone();
        let client = client.clone();
        let mut events = events.clone();
        let worker = tokio::spawn(async move {
            let params = WorkspaceActiveSnapshotParams {
                limit: Some(ACTIVE_SNAPSHOT_LIMIT),
            };
            while Instant::now() < deadline {
                record_control_call(
                    &metrics,
                    &mut events,
                    ControlPlaneEndpoint::ActiveSnapshot,
                    "control_active_snapshot",
                    client.get_workspace_active_snapshot(workspace_id, &params),
                )
                .await;
                sleep(interval).await;
            }
        });
        control_workers.push(worker);
    }

    if scenario.control_plane.session_snapshot_interval_ms > 0 && !sessions.is_empty() {
        let interval =
            Duration::from_millis(scenario.control_plane.session_snapshot_interval_ms.max(1));
        let metrics = control_plane_metrics.clone();
        let client = client.clone();
        let mut events = events.clone();
        let sessions = sessions.clone();
        let worker = tokio::spawn(async move {
            let mut index = 0usize;
            while Instant::now() < deadline {
                let session_id = sessions[index % sessions.len()];
                index = index.wrapping_add(1);
                record_control_call(
                    &metrics,
                    &mut events,
                    ControlPlaneEndpoint::SessionSnapshot,
                    "control_session_snapshot",
                    client.get_session_snapshot(
                        session_id,
                        Some(SESSION_SNAPSHOT_LIMIT),
                        Some(false),
                    ),
                )
                .await;
                sleep(interval).await;
            }
        });
        control_workers.push(worker);
    }

    if scenario.control_plane.session_head_interval_ms > 0 && !sessions.is_empty() {
        let interval =
            Duration::from_millis(scenario.control_plane.session_head_interval_ms.max(1));
        let metrics = control_plane_metrics.clone();
        let client = client.clone();
        let mut events = events.clone();
        let sessions = sessions.clone();
        let worker = tokio::spawn(async move {
            let mut index = 0usize;
            while Instant::now() < deadline {
                let session_id = sessions[index % sessions.len()];
                index = index.wrapping_add(1);
                record_control_call(
                    &metrics,
                    &mut events,
                    ControlPlaneEndpoint::SessionHead,
                    "control_session_head",
                    client.get_session_head(session_id, Some(SESSION_HEAD_LIMIT), Some(false)),
                )
                .await;
                sleep(interval).await;
            }
        });
        control_workers.push(worker);
    }

    if scenario.control_plane.ws_replay_interval_ms > 0 && !sessions.is_empty() {
        let interval = Duration::from_millis(scenario.control_plane.ws_replay_interval_ms.max(1));
        let metrics = control_plane_metrics.clone();
        let client = client.clone();
        let mut events = events.clone();
        let sessions = sessions.clone();
        let auth_token = cli.auth_token.clone();
        let worker = tokio::spawn(async move {
            let mut index = 0usize;
            while Instant::now() < deadline {
                let session_id = sessions[index % sessions.len()];
                index = index.wrapping_add(1);
                let start = Instant::now();
                let result =
                    run_ws_replay_once(&client, auth_token.as_deref(), workspace_id, session_id)
                        .await;
                let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                record_control_plane_result(
                    &metrics,
                    ControlPlaneEndpoint::WsReplay,
                    &mut events,
                    "control_ws_replay",
                    elapsed,
                    result.is_ok(),
                );
                sleep(interval).await;
            }
        });
        control_workers.push(worker);
    }

    if scenario.control_plane.reconnect_interval_ms > 0 {
        let interval = Duration::from_millis(scenario.control_plane.reconnect_interval_ms.max(1));
        let metrics = control_plane_metrics.clone();
        let client = client.clone();
        let mut events = events.clone();
        let worker = tokio::spawn(async move {
            let params = WorkspaceActiveSnapshotParams {
                limit: Some(ACTIVE_SNAPSHOT_LIMIT),
            };
            while Instant::now() < deadline {
                record_control_call(
                    &metrics,
                    &mut events,
                    ControlPlaneEndpoint::ReconnectCatchup,
                    "control_reconnect_catchup",
                    run_reconnect_catchup_once(&client, workspace_id, &params),
                )
                .await;
                sleep(interval).await;
            }
        });
        control_workers.push(worker);
    }

    for (idx, session_id) in sessions.iter().enumerate() {
        let client = client.clone();
        let metrics = metrics.clone();
        let pending = pending.clone();
        let mut events = events.clone();
        let workload = &scenario.workload;
        let provider_id = cli.provider_id.clone();
        let model_id = cli.model_id.clone();
        let session_id = *session_id;
        let interval = Duration::from_millis(workload.message_interval_ms.max(1));
        let message_size = workload.message_size.max(8);
        let tool_calls = workload.tool_calls_per_message;
        let worker = tokio::spawn(async move {
            let mut counter = 0u64;
            while Instant::now() < deadline {
                counter += 1;
                let content = build_message_content(idx, counter, message_size, tool_calls);
                let start = Instant::now();
                let req = PostMessageRequest {
                    content: content.clone(),
                    delivery: None,
                    attachments: Vec::new(),
                };
                let result = client.post_message(session_id, &req).await;
                let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                metrics.http_ms.lock().unwrap().push(elapsed);
                if result.is_ok() {
                    *metrics.sent.lock().unwrap() += 1;
                    pending
                        .lock()
                        .unwrap()
                        .pending
                        .insert(content.clone(), Instant::now());
                    events
                        .write(EventRecord {
                            ts_ms: chrono::Utc::now().timestamp_millis() as u128,
                            kind: "message_sent",
                            value_ms: Some(elapsed),
                            detail: Some(content.clone()),
                        })
                        .ok();
                } else {
                    *metrics.errors.lock().unwrap() += 1;
                    events
                        .write(EventRecord {
                            ts_ms: chrono::Utc::now().timestamp_millis() as u128,
                            kind: "message_error",
                            value_ms: Some(elapsed),
                            detail: Some(format!("{provider_id}/{model_id}")),
                        })
                        .ok();
                }
                sleep(interval).await;
            }
        });
        workers.push(worker);
    }

    for worker in workers {
        let _ = worker.await;
    }

    for worker in control_workers {
        let _ = worker.await;
    }

    if let Some(handle) = ws_handle {
        handle.abort();
    }

    let control_plane_summary = if *control_plane_metrics.totals.sent.lock().unwrap() > 0 {
        Some(build_control_plane_summary(&control_plane_metrics))
    } else {
        None
    };
    let summary = build_summary(scenario, &metrics, control_plane_summary);
    let summary_path = out_dir.join("summary.json");
    fs::write(&summary_path, serde_json::to_vec_pretty(&summary)?)
        .context("writing summary.json")?;

    Ok(DaemonRunOutput {
        out_dir,
        summary_path,
        summary,
        tasks: tasks.len(),
        sessions: sessions.len(),
    })
}

async fn run_end_to_end_mode(cli: &Cli, scenario: &ScenarioSpec) -> Result<()> {
    let out_dir = resolve_out_dir(&scenario.name, cli.out_dir.as_ref())?;
    fs::create_dir_all(&out_dir).context("creating output dir")?;
    let mut daemon_child = spawn_daemon_if_needed(cli, &out_dir).await?;

    let result = async {
        let client = Arc::new(Client::new(DaemonConfig {
            base_url: cli.base_url.clone(),
            auth_token: cli.auth_token.clone(),
        })?);

        wait_for_daemon(
            &client,
            Duration::from_millis(cli.daemon_ready_timeout_ms.max(1)),
        )
        .await?;

        let workspace_id = resolve_workspace(&client, cli.workspace_id.as_deref()).await?;
        let run_id = uuid::Uuid::new_v4().to_string();
        let duration = Duration::from_millis(scenario.duration_ms.max(1));
        let ui_interval = Duration::from_millis(
            cli.ui_interval_ms
                .unwrap_or(scenario.workload.message_interval_ms)
                .max(1),
        );

        let ui_handle = tokio::spawn({
            let client = client.clone();
            let run_id = run_id.clone();
            async move {
                run_ui_workload(client, workspace_id, duration, ui_interval, run_id).await
            }
        });

        let output = run_daemon_workload(
            client.clone(),
            workspace_id,
            scenario,
            cli,
            out_dir.clone(),
        )
        .await;

        let ui_summary = match ui_handle.await {
            Ok(Ok(summary)) => Some(summary),
            Ok(Err(err)) => {
                eprintln!("ui workload failed: {err:#}");
                None
            }
            Err(err) => {
                eprintln!("ui workload join failed: {err:#}");
                None
            }
        };

        let output = output?;
        let ui_telemetry = client
            .get_telemetry_summary(&TelemetrySummaryParams {
                run_id: Some(run_id.clone()),
                ..TelemetrySummaryParams::default()
            })
            .await
            .ok();

        let combined = CombinedSummary {
            name: scenario.name.clone(),
            mode: scenario.mode,
            duration_ms: scenario.duration_ms,
            run_id: Some(run_id),
            daemon: output.summary,
            ui: ui_summary,
            ui_telemetry,
        };

        let combined_path = output.out_dir.join("combined_summary.json");
        fs::write(&combined_path, serde_json::to_vec_pretty(&combined)?)
            .context("writing combined_summary.json")?;

        println!("End-to-end load test complete: {}", scenario.name);
        println!("Tasks: {}, Sessions: {}", output.tasks, output.sessions);
        println!(
            "Sent: {}, Done: {}, Errors: {}",
            combined.daemon.sent, combined.daemon.done, combined.daemon.errors
        );
        println!("HTTP p95: {:.1}ms", combined.daemon.http_ms.p95);
        println!("Done p95: {:.1}ms", combined.daemon.done_ms.p95);
        if let Some(golden) = combined.daemon.golden.as_ref() {
            if let Some(hot_read) = golden.hot_read_p95_ms {
                let target = golden.hot_read_p95_target_ms.unwrap_or(HOT_READ_P95_TARGET_MS);
                println!(
                    "Hot read p95: {:.1}ms (target {:.1}ms)",
                    hot_read, target
                );
            }
            if let Some(reconnect) = golden.reconnect_catchup_p95_ms {
                println!("Reconnect catch-up p95: {:.1}ms", reconnect);
            }
        }
        if let Some(ui) = combined.ui.as_ref() {
            println!("UI p95: {:.1}ms", ui.api_ms.p95);
        }
        println!("Summary: {}", output.summary_path.display());
        println!("Combined Summary: {}", combined_path.display());

        Ok(())
    }
    .await;

    if let Some(mut child) = daemon_child.take() {
        let _ = child.kill().await;
    }

    result
}

async fn spawn_daemon_if_needed(
    cli: &Cli,
    out_dir: &Path,
) -> Result<Option<tokio::process::Child>> {
    let Some(cmd) = cli.daemon_cmd.as_ref() else {
        return Ok(None);
    };
    let data_dir = cli
        .daemon_data_dir
        .clone()
        .unwrap_or_else(|| out_dir.join("daemon-data"));
    fs::create_dir_all(&data_dir).context("creating daemon data dir")?;
    let log_path = out_dir.join("daemon.log");
    let log_file = File::create(&log_path).context("creating daemon log")?;
    let log_err = log_file.try_clone().context("cloning daemon log")?;
    let mut command = Command::new("bash");
    command
        .arg("-lc")
        .arg(cmd)
        .env("CTX_SHOW_FAKE_PROVIDER", "1")
        .env("CTX_DATA_DIR", &data_dir)
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(log_err));
    let child = command.spawn().context("spawning daemon command")?;
    Ok(Some(child))
}

async fn wait_for_daemon(client: &Client, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        if client.get_health().await.is_ok() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(anyhow!("daemon did not become healthy within {timeout:?}"));
        }
        sleep(Duration::from_millis(250)).await;
    }
}

async fn run_ui_workload(
    client: Arc<Client>,
    workspace_id: WorkspaceId,
    duration: Duration,
    interval: Duration,
    run_id: String,
) -> Result<UiSummary> {
    let metrics = UiMetrics::default();
    let mut telemetry = Vec::new();
    let deadline = Instant::now() + duration;
    let params = WorkspaceActiveSnapshotParams { limit: Some(50) };

    while Instant::now() < deadline {
        record_ui_call(
            &metrics,
            &mut telemetry,
            &run_id,
            "/api/workspaces",
            "GET",
            client.list_workspaces(),
        )
        .await;

        record_ui_call(
            &metrics,
            &mut telemetry,
            &run_id,
            "/api/workspaces/:id/active_snapshot",
            "GET",
            client.get_workspace_active_snapshot(workspace_id, &params),
        )
        .await;

        if telemetry.len() >= 100 {
            flush_client_telemetry(&client, &mut telemetry).await;
        }

        sleep(interval).await;
    }

    flush_client_telemetry(&client, &mut telemetry).await;

    Ok(build_ui_summary(&metrics))
}

fn record_control_plane_metrics(
    metrics: &ControlPlaneEndpointMetrics,
    elapsed: f64,
    success: bool,
) {
    metrics.api_ms.lock().unwrap().push(elapsed);
    *metrics.sent.lock().unwrap() += 1;
    if !success {
        *metrics.errors.lock().unwrap() += 1;
    }
}

fn record_control_plane_result(
    metrics: &ControlPlaneMetrics,
    endpoint: ControlPlaneEndpoint,
    events: &mut EventWriter,
    kind: &str,
    elapsed: f64,
    success: bool,
) {
    record_control_plane_metrics(&metrics.totals, elapsed, success);
    record_control_plane_metrics(metrics.endpoint_metrics(endpoint), elapsed, success);
    events
        .write(EventRecord {
            ts_ms: chrono::Utc::now().timestamp_millis() as u128,
            kind,
            value_ms: Some(elapsed),
            detail: if success {
                None
            } else {
                Some("error".to_string())
            },
        })
        .ok();
}

async fn record_control_call<F, T>(
    metrics: &ControlPlaneMetrics,
    events: &mut EventWriter,
    endpoint: ControlPlaneEndpoint,
    kind: &str,
    future: F,
) where
    F: Future<Output = Result<T>>,
{
    let start = Instant::now();
    let result = future.await;
    let elapsed = start.elapsed().as_secs_f64() * 1000.0;
    record_control_plane_result(metrics, endpoint, events, kind, elapsed, result.is_ok());
}

async fn record_ui_call<F, T>(
    metrics: &UiMetrics,
    telemetry: &mut Vec<ClientTelemetryMetric>,
    run_id: &str,
    endpoint: &str,
    method: &str,
    future: F,
) where
    F: Future<Output = Result<T>>,
{
    let start = Instant::now();
    let result = future.await;
    let elapsed = start.elapsed().as_secs_f64() * 1000.0;
    metrics.api_ms.lock().unwrap().push(elapsed);
    *metrics.sent.lock().unwrap() += 1;

    let (status, success) = if result.is_ok() {
        ("200".to_string(), true)
    } else {
        *metrics.errors.lock().unwrap() += 1;
        ("error".to_string(), false)
    };

    telemetry.push(ClientTelemetryMetric {
        name: "client.api.duration_ms".to_string(),
        kind: PerfMetricKind::Histogram,
        unit: "ms".to_string(),
        value: elapsed,
        labels: Some(client_labels(endpoint, method, &status, success)),
        run_id: Some(run_id.to_string()),
    });

    if !success {
        telemetry.push(ClientTelemetryMetric {
            name: "client.api.error_count".to_string(),
            kind: PerfMetricKind::Counter,
            unit: "count".to_string(),
            value: 1.0,
            labels: Some(client_labels(endpoint, method, "error", false)),
            run_id: Some(run_id.to_string()),
        });
    }
}

async fn flush_client_telemetry(client: &Client, telemetry: &mut Vec<ClientTelemetryMetric>) {
    if telemetry.is_empty() {
        return;
    }
    let batch = ClientTelemetryBatch {
        events: std::mem::take(telemetry),
    };
    let _ = client.post_client_telemetry(&batch).await;
}

fn client_labels(
    endpoint: &str,
    method: &str,
    status: &str,
    success: bool,
) -> HashMap<String, String> {
    let mut labels = HashMap::new();
    labels.insert("endpoint".to_string(), endpoint.to_string());
    labels.insert("method".to_string(), method.to_string());
    labels.insert("status".to_string(), status.to_string());
    labels.insert("success".to_string(), success.to_string());
    labels.insert("source".to_string(), "client".to_string());
    labels
}

fn build_ui_summary(metrics: &UiMetrics) -> UiSummary {
    let sent = *metrics.sent.lock().unwrap();
    let errors = *metrics.errors.lock().unwrap();
    UiSummary {
        sent,
        errors,
        api_ms: summarize(metrics.api_ms.lock().unwrap().as_slice()),
    }
}

fn build_control_plane_summary(metrics: &ControlPlaneMetrics) -> ControlPlaneSummary {
    let totals = summarize_control_plane_endpoint(&metrics.totals);
    ControlPlaneSummary {
        sent: totals.sent,
        errors: totals.errors,
        api_ms: totals.api_ms,
        endpoints: ControlPlaneEndpointSummaryBreakdown {
            health: summarize_control_plane_endpoint(&metrics.health),
            active_snapshot: summarize_control_plane_endpoint(&metrics.active_snapshot),
            session_snapshot: summarize_control_plane_endpoint(&metrics.session_snapshot),
            session_head: summarize_control_plane_endpoint(&metrics.session_head),
            ws_replay: summarize_control_plane_endpoint(&metrics.ws_replay),
            reconnect: summarize_control_plane_endpoint(&metrics.reconnect),
        },
    }
}

fn summarize_control_plane_endpoint(
    metrics: &ControlPlaneEndpointMetrics,
) -> ControlPlaneEndpointSummary {
    ControlPlaneEndpointSummary {
        sent: *metrics.sent.lock().unwrap(),
        errors: *metrics.errors.lock().unwrap(),
        api_ms: summarize(metrics.api_ms.lock().unwrap().as_slice()),
    }
}

async fn resolve_workspace(client: &Client, id: Option<&str>) -> Result<WorkspaceId> {
    if let Some(id) = id {
        let parsed = uuid::Uuid::parse_str(id).context("invalid workspace id")?;
        return Ok(WorkspaceId(parsed));
    }
    let workspaces = client.list_workspaces().await?;
    let first = workspaces
        .first()
        .ok_or_else(|| anyhow!("no workspaces found; create one first"))?;
    Ok(first.id)
}

async fn setup_tasks_and_sessions(
    client: &Client,
    workspace_id: WorkspaceId,
    scenario: &ScenarioSpec,
    rng: &mut StdRng,
    cli: &Cli,
) -> Result<(Vec<TaskId>, Vec<SessionId>)> {
    let mut tasks = Vec::new();
    let mut sessions = Vec::new();
    for idx in 0..scenario.workload.tasks {
        let title = format!("Load test {} task {}", scenario.name, idx + 1);
        let task = client
            .create_task(
                workspace_id,
                &CreateTaskRequest {
                    title,
                    description: None,
                    create_default_session: Some(false),
                },
            )
            .await
            .with_context(|| "creating task")?;
        tasks.push(task.id);

        let subagents =
            rng.gen_range(scenario.workload.subagents_min..=scenario.workload.subagents_max);
        for _ in 0..subagents {
            let session = client
                .create_session(
                    task.id,
                    &CreateSessionRequest {
                        provider_id: cli.provider_id.clone(),
                        model_id: cli.model_id.clone(),
                        parent_session_id: None,
                        relationship: None,
                        env_target: None,
                        worktree_id: None,
                        initial_prompt: None,
                    },
                )
                .await
                .with_context(|| "creating session (ensure CTX_SHOW_FAKE_PROVIDER=1)")?;
            sessions.push(session.session.id);
        }
    }
    Ok((tasks, sessions))
}

async fn spawn_ws_listener(
    client: &Client,
    auth_token: Option<&str>,
    workspace_id: WorkspaceId,
    sessions: Vec<SessionId>,
    metrics: Arc<Metrics>,
    pending: Arc<Mutex<PendingState>>,
    mut events: EventWriter,
) -> Result<Option<JoinHandle<()>>> {
    if sessions.is_empty() {
        return Ok(None);
    }
    let mut url =
        Url::parse(&client.workspace_stream_url(workspace_id)?).context("parsing ws url")?;
    if let Some(token) = auth_token {
        url.query_pairs_mut().append_pair("token", token);
    }

    let (ws_stream, _) = connect_async(url.to_string())
        .await
        .context("connecting ws")?;
    let (mut write, mut read) = ws_stream.split();

    let message = WorkspaceActiveSnapshotClientMessage::Subscribe {
        session_ids: sessions,
        sessions: Vec::new(),
        include_active_heads: false,
    };
    let payload = serde_json::to_string(&message)?;
    write.send(WsMessage::Text(payload.into())).await?;

    let handle = tokio::spawn(async move {
        while let Some(Ok(msg)) = read.next().await {
            let text = match msg {
                WsMessage::Text(text) => text.to_string(),
                WsMessage::Binary(bytes) => String::from_utf8(bytes.to_vec()).unwrap_or_default(),
                _ => continue,
            };
            let evt: ctx_core::models::WorkspaceActiveSnapshotEvent =
                match serde_json::from_str(&text) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
            if let ctx_core::models::WorkspaceActiveSnapshotEvent::SessionHeadDelta {
                delta, ..
            } = evt
            {
                if let Some(ev) = delta.event.as_ref() {
                    let content = extract_event_content(&ev.event_type, &ev.payload_json);
                    if let Some(content) = content {
                        if matches!(&ev.event_type, SessionEventType::AssistantChunk) {
                            let input = content.strip_prefix("echo: ").unwrap_or(&content);
                            let mut guard = pending.lock().unwrap();
                            if guard.pending.contains_key(input)
                                && !guard.first_chunked.contains(input)
                            {
                                guard.first_chunked.insert(input.to_string());
                                if let Some(start) = guard.pending.get(input) {
                                    let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                                    metrics.first_chunk_ms.lock().unwrap().push(elapsed);
                                    events
                                        .write(EventRecord {
                                            ts_ms: chrono::Utc::now().timestamp_millis() as u128,
                                            kind: "first_chunk",
                                            value_ms: Some(elapsed),
                                            detail: Some(input.to_string()),
                                        })
                                        .ok();
                                }
                            }
                        }
                        if matches!(&ev.event_type, SessionEventType::AssistantComplete) {
                            let input = content.strip_prefix("done: ").unwrap_or(&content);
                            let mut guard = pending.lock().unwrap();
                            if let Some(start) = guard.pending.remove(input) {
                                let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                                metrics.done_ms.lock().unwrap().push(elapsed);
                                *metrics.done.lock().unwrap() += 1;
                                events
                                    .write(EventRecord {
                                        ts_ms: chrono::Utc::now().timestamp_millis() as u128,
                                        kind: "message_done",
                                        value_ms: Some(elapsed),
                                        detail: Some(input.to_string()),
                                    })
                                    .ok();
                            }
                        }
                    }
                }
            }
        }
    });

    Ok(Some(handle))
}

async fn run_reconnect_catchup_once(
    client: &Client,
    workspace_id: WorkspaceId,
    params: &WorkspaceActiveSnapshotParams,
) -> Result<()> {
    let _ = client
        .get_workspace_active_snapshot(workspace_id, params)
        .await?;
    let _ = client.get_workspace_active_heads(workspace_id).await?;
    Ok(())
}

async fn run_ws_replay_once(
    client: &Client,
    auth_token: Option<&str>,
    workspace_id: WorkspaceId,
    session_id: SessionId,
) -> Result<()> {
    let mut url =
        Url::parse(&client.workspace_stream_url(workspace_id)?).context("parsing ws url")?;
    if let Some(token) = auth_token {
        url.query_pairs_mut().append_pair("token", token);
    }
    let (ws_stream, _) = connect_async(url.to_string())
        .await
        .context("connecting ws")?;
    let (mut write, mut read) = ws_stream.split();

    let message = WorkspaceActiveSnapshotClientMessage::Subscribe {
        session_ids: Vec::new(),
        sessions: vec![WorkspaceActiveSnapshotSessionSubscription {
            session_id,
            after_seq: Some(0),
        }],
        include_active_heads: false,
    };
    let payload = serde_json::to_string(&message)?;
    write.send(WsMessage::Text(payload.into())).await?;

    let deadline = Instant::now() + Duration::from_millis(WS_REPLAY_READ_TIMEOUT_MS);
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        let wait = remaining.min(Duration::from_millis(200));
        match tokio::time::timeout(wait, read.next()).await {
            Ok(Some(Ok(_))) => break,
            Ok(Some(Err(err))) => return Err(anyhow!(err)),
            Ok(None) => break,
            Err(_) => {}
        }
    }

    Ok(())
}

fn extract_event_content(event_type: &SessionEventType, payload: &Value) -> Option<String> {
    match event_type {
        SessionEventType::AssistantChunk | SessionEventType::AssistantComplete => payload
            .get("content")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| {
                payload
                    .get("content_fragment")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            }),
        _ => None,
    }
}

fn build_message_content(
    session_index: usize,
    counter: u64,
    message_size: usize,
    tool_calls: u32,
) -> String {
    let base = format!("loadtest:{}:{}:", session_index, counter);
    let mut content = base.clone();
    while content.len() < message_size {
        content.push('x');
    }
    if tool_calls == 0 {
        return content;
    }
    let tools = build_tool_calls(tool_calls);
    format!("{}\n[[tool_calls]]{}[[/tool_calls]]", content, tools)
}

fn build_tool_calls(count: u32) -> String {
    let mut calls = Vec::new();
    for idx in 0..count {
        calls.push(serde_json::json!({
            "kind": "execute",
            "title": format!("tool-{}", idx + 1),
            "input": { "cmd": "echo", "args": ["ok"] },
            "output_text": "ok"
        }));
    }
    serde_json::json!({ "tool_calls": calls }).to_string()
}

fn resolve_out_dir(name: &str, override_dir: Option<&PathBuf>) -> Result<PathBuf> {
    if let Some(dir) = override_dir {
        return Ok(dir.clone());
    }
    let stamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    Ok(PathBuf::from(format!(
        "/tmp/ctx-load-test/{}_{}",
        name, stamp
    )))
}

fn build_summary(
    scenario: &ScenarioSpec,
    metrics: &Metrics,
    control_plane: Option<ControlPlaneSummary>,
) -> Summary {
    let sent = *metrics.sent.lock().unwrap();
    let done = *metrics.done.lock().unwrap();
    let errors = *metrics.errors.lock().unwrap();
    let golden = build_golden_summary(control_plane.as_ref());
    Summary {
        name: scenario.name.clone(),
        mode: scenario.mode,
        duration_ms: scenario.duration_ms,
        sent,
        done,
        errors,
        http_ms: summarize(metrics.http_ms.lock().unwrap().as_slice()),
        first_chunk_ms: summarize(metrics.first_chunk_ms.lock().unwrap().as_slice()),
        done_ms: summarize(metrics.done_ms.lock().unwrap().as_slice()),
        golden,
        control_plane,
    }
}

fn build_golden_summary(control_plane: Option<&ControlPlaneSummary>) -> Option<GoldenSummary> {
    let control_plane = control_plane?;
    let active_snapshot = &control_plane.endpoints.active_snapshot;
    let session_head = &control_plane.endpoints.session_head;
    let hot_read_p95 = if active_snapshot.sent > 0 && session_head.sent > 0 {
        Some(active_snapshot.api_ms.p95.max(session_head.api_ms.p95))
    } else {
        None
    };
    let reconnect_p95 = if control_plane.endpoints.reconnect.sent > 0 {
        Some(control_plane.endpoints.reconnect.api_ms.p95)
    } else {
        None
    };

    if hot_read_p95.is_none() && reconnect_p95.is_none() {
        return None;
    }

    Some(GoldenSummary {
        hot_read_p95_ms: hot_read_p95,
        hot_read_p95_target_ms: hot_read_p95.map(|_| HOT_READ_P95_TARGET_MS),
        hot_read_p95_ok: hot_read_p95.map(|value| value <= HOT_READ_P95_TARGET_MS),
        reconnect_catchup_p95_ms: reconnect_p95,
    })
}

fn summarize(values: &[f64]) -> Percentiles {
    if values.is_empty() {
        return Percentiles::default();
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Percentiles {
        p50: percentile(&sorted, 0.50),
        p95: percentile(&sorted, 0.95),
        p99: percentile(&sorted, 0.99),
        max: *sorted.last().unwrap_or(&0.0),
    }
}

fn percentile(sorted: &[f64], pct: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = ((sorted.len() - 1) as f64 * pct).round() as usize;
    sorted.get(rank).cloned().unwrap_or(0.0)
}

#[derive(Clone)]
struct EventWriter {
    writer: Arc<Mutex<BufWriter<File>>>,
}

impl EventWriter {
    fn new(path: PathBuf) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("creating event dir")?;
        }
        let file = File::create(path).context("creating event log")?;
        Ok(Self {
            writer: Arc::new(Mutex::new(BufWriter::new(file))),
        })
    }

    fn write(&mut self, record: EventRecord<'_>) -> Result<()> {
        let line = serde_json::to_string(&record)?;
        let mut guard = self.writer.lock().unwrap();
        guard.write_all(line.as_bytes())?;
        guard.write_all(b"\n")?;
        Ok(())
    }
}
