use std::fs::{self, File};
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use tokio::process::Command;
use tokio::time::sleep;

use ctx_client::{Client, DaemonConfig, TelemetrySummaryParams};

use crate::metrics::{CombinedSummary, HOT_READ_P95_TARGET_MS};
use crate::scenario::{Cli, ScenarioSpec};

use super::daemon::run_daemon_workload;
use super::tasks::resolve_workspace;
use super::ui::run_ui_workload;

pub(crate) async fn run_end_to_end_mode(cli: &Cli, scenario: &ScenarioSpec) -> Result<()> {
    let out_dir = crate::output::resolve_out_dir(&scenario.name, cli.out_dir.as_ref())?;
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
            async move { run_ui_workload(client, workspace_id, duration, ui_interval, run_id).await }
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

        let combined_path = out_dir.join("combined_summary.json");
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
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if client.get_health().await.is_ok() {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            return Err(anyhow!("daemon did not become healthy within {timeout:?}"));
        }
        sleep(Duration::from_millis(250)).await;
    }
}
