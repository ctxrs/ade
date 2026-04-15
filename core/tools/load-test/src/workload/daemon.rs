use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use rand::rngs::StdRng;
use rand::SeedableRng;
use tokio::time::sleep;

use ctx_client::{Client, DaemonConfig, PostMessageRequest, WorkspaceActiveSnapshotParams};
use ctx_core::ids::WorkspaceId;

use crate::metrics::{
    build_control_plane_summary, build_summary, ControlPlaneEndpoint, ControlPlaneMetrics, Metrics,
    PendingState, HOT_READ_P95_TARGET_MS,
};
use crate::output::{DaemonRunOutput, EventRecord, EventWriter};
use crate::scenario::{Cli, ScenarioSpec};
use crate::ws::{run_ws_replay_once, spawn_ws_listener};

use super::control_plane::{
    record_control_call, record_control_plane_result, run_reconnect_catchup_once,
    ACTIVE_SNAPSHOT_LIMIT, SESSION_HEAD_LIMIT, SESSION_SNAPSHOT_LIMIT,
};
use super::tasks::setup_tasks_and_sessions;

pub(crate) async fn run_daemon_mode(cli: &Cli, scenario: &ScenarioSpec) -> Result<()> {
    let out_dir = crate::output::resolve_out_dir(&scenario.name, cli.out_dir.as_ref())?;
    let client = Arc::new(Client::new(DaemonConfig {
        base_url: cli.base_url.clone(),
        auth_token: cli.auth_token.clone(),
    })?);
    let workspace_id =
        super::tasks::resolve_workspace(&client, cli.workspace_id.as_deref()).await?;
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
            println!("Hot read p95: {hot_read:.1}ms (target {target:.1}ms)");
        }
        if let Some(reconnect) = golden.reconnect_catchup_p95_ms {
            println!("Reconnect catch-up p95: {reconnect:.1}ms");
        }
    }
    println!("Summary: {}", output.summary_path.display());

    Ok(())
}

pub(crate) async fn run_daemon_workload(
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
                    id: None,
                    turn_id: None,
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
        summary_path,
        summary,
        tasks: tasks.len(),
        sessions: sessions.len(),
    })
}

fn build_message_content(
    session_index: usize,
    counter: u64,
    message_size: usize,
    tool_calls: u32,
) -> String {
    let base = format!("loadtest:{session_index}:{counter}:");
    let mut content = base.clone();
    while content.len() < message_size {
        content.push('x');
    }
    if tool_calls == 0 {
        return content;
    }
    let tools = build_tool_calls(tool_calls);
    format!("{content}\n[[tool_calls]]{tools}[[/tool_calls]]")
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
