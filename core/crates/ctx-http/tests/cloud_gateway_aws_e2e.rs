use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result};
use aws_config::Region;
use aws_credential_types::Credentials;
use aws_sdk_ec2::Client as Ec2Client;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::json;
use tokio::process::Command;
use tokio::time::{sleep, Instant};

use ctx_core::models::{
    MessageRole, SessionEventType, SessionHead, Task, TrackWorker, Workspace,
    WorkspaceCatchupSnapshot,
};
use ctx_http::settings::{AwsCloudWorkersSettings, CloudGatewaySettings, CloudWorkersSettings};
use ctx_http::settings::{Settings as DaemonSettings};

const REQUIRED_TIER: &str = "3";
const ASSISTANT_PROMPT: &str = "Reply with the exact text: cloud_gateway_aws_e2e_ok";
const ASSISTANT_EXPECTED: &str = "cloud_gateway_aws_e2e_ok";

#[derive(Debug, Deserialize)]
struct AwsGatewayLaunchResp {
    gateway: CloudGatewaySettings,
}

fn env_trim(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn e2e_tier_enabled() -> bool {
    env_trim("CTX_E2E_TIER").as_deref() == Some(REQUIRED_TIER)
}

fn pick_free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .ok()
        .and_then(|listener| listener.local_addr().ok().map(|addr| addr.port()))
        .unwrap_or(0)
}

async fn wait_for_daemon(base: &str) -> Result<()> {
    let client = reqwest::Client::new();
    for _ in 0..60 {
        if let Ok(resp) = client.get(format!("{base}/api/health")).send().await {
            if resp.status().is_success() {
                return Ok(());
            }
        }
        sleep(Duration::from_millis(250)).await;
    }
    anyhow::bail!("daemon did not become healthy")
}

async fn git_remote_origin(root: &Path) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["remote", "get-url", "origin"])
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!url.is_empty()).then_some(url)
}

async fn write_settings(data_root: &Path, aws: AwsCloudWorkersSettings) -> Result<()> {
    let settings = DaemonSettings {
        cloud_workers: Some(CloudWorkersSettings {
            gateway: None,
            aws: Some(aws),
        }),
        ..DaemonSettings::default()
    };
    let path = data_root.join("settings.json");
    let serialized = serde_json::to_string_pretty(&settings).context("serializing settings")?;
    tokio::fs::write(&path, serialized)
        .await
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

async fn send_json<T: DeserializeOwned>(request: reqwest::RequestBuilder) -> Result<T> {
    let resp = request.send().await.context("request failed")?;
    let resp = resp.error_for_status().context("request failed")?;
    resp.json::<T>().await.context("parsing response json")
}

async fn wait_for_assistant_reply(
    client: &reqwest::Client,
    base: &str,
    session_id: &str,
) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        let head: SessionHead = send_json(
            client
                .get(format!("{base}/api/sessions/{session_id}/head"))
                .query(&[("limit", "20"), ("include_events", "true")]),
        )
        .await?;

        if head.messages.iter().any(|msg| {
            msg.role == MessageRole::Assistant && msg.content.contains(ASSISTANT_EXPECTED)
        }) {
            return Ok(());
        }

        if head
            .events
            .iter()
            .any(|event| matches!(event.event_type, SessionEventType::Error))
        {
            anyhow::bail!("saw error event while waiting for assistant reply");
        }

        if Instant::now() >= deadline {
            anyhow::bail!("timed out waiting for assistant reply");
        }
        sleep(Duration::from_secs(2)).await;
    }
}

async fn aws_sdk_config(
    region: &str,
    access_key_id: &str,
    secret_access_key: &str,
    session_token: Option<&str>,
) -> aws_config::SdkConfig {
    let creds = Credentials::new(
        access_key_id,
        secret_access_key,
        session_token.map(|v| v.to_string()),
        None,
        "ctx-e2e",
    );
    aws_config::from_env()
        .region(Region::new(region.to_string()))
        .credentials_provider(creds)
        .load()
        .await
}

async fn terminate_gateway_instance(
    region: &str,
    access_key_id: &str,
    secret_access_key: &str,
    session_token: Option<&str>,
    instance_id: &str,
) -> Result<()> {
    let config = aws_sdk_config(region, access_key_id, secret_access_key, session_token).await;
    let ec2 = Ec2Client::new(&config);
    ec2.terminate_instances()
        .instance_ids(instance_id)
        .send()
        .await
        .context("terminating gateway instance")?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires CTX_E2E_TIER=3 and AWS credentials"]
async fn cloud_gateway_aws_e2e() -> Result<()> {
    if !e2e_tier_enabled() {
        eprintln!("skipping: set CTX_E2E_TIER=3 to run this test");
        return Ok(());
    }

    let Some(access_key_id) = env_trim("AWS_ACCESS_KEY_ID") else {
        eprintln!("skipping: missing AWS_ACCESS_KEY_ID");
        return Ok(());
    };
    let Some(secret_access_key) = env_trim("AWS_SECRET_ACCESS_KEY") else {
        eprintln!("skipping: missing AWS_SECRET_ACCESS_KEY");
        return Ok(());
    };
    let region = match env_trim("AWS_REGION").or_else(|| env_trim("AWS_DEFAULT_REGION")) {
        Some(region) => region,
        None => {
            eprintln!("skipping: missing AWS_REGION or AWS_DEFAULT_REGION");
            return Ok(());
        }
    };
    let Some(subnet_id) = env_trim("AWS_SUBNET_ID") else {
        eprintln!("skipping: missing AWS_SUBNET_ID");
        return Ok(());
    };
    let Some(security_group_id) = env_trim("AWS_SECURITY_GROUP_ID") else {
        eprintln!("skipping: missing AWS_SECURITY_GROUP_ID");
        return Ok(());
    };
    if env_trim("CTX_WORKER_GATEWAY_BIN").is_none() {
        eprintln!("skipping: missing CTX_WORKER_GATEWAY_BIN");
        return Ok(());
    }
    if env_trim("CTX_WORKER_SHIM_BIN").is_none() {
        eprintln!("skipping: missing CTX_WORKER_SHIM_BIN");
        return Ok(());
    }
    let Some(provider_id) = env_trim("CTX_E2E_PROVIDER_ID") else {
        eprintln!("skipping: missing CTX_E2E_PROVIDER_ID");
        return Ok(());
    };
    let Some(model_id) = env_trim("CTX_E2E_MODEL_ID") else {
        eprintln!("skipping: missing CTX_E2E_MODEL_ID");
        return Ok(());
    };

    let workspace_root = env_trim("CTX_E2E_WORKSPACE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().expect("current dir unavailable"));
    if git_remote_origin(&workspace_root).await.is_none() {
        eprintln!("skipping: workspace root missing git remote origin");
        return Ok(());
    }

    let data_dir = tempfile::tempdir().context("creating temp data dir")?;
    let default_ami = env_trim("AWS_AMI_ID");
    let aws_settings = AwsCloudWorkersSettings {
        access_key_id: access_key_id.clone(),
        secret_access_key: secret_access_key.clone(),
        region: region.clone(),
        gateway_instance_type: env_trim("AWS_GATEWAY_INSTANCE_TYPE").unwrap_or_default(),
        worker_instance_type: env_trim("AWS_WORKER_INSTANCE_TYPE").unwrap_or_default(),
        subnet_id: Some(subnet_id),
        security_group_id: Some(security_group_id),
        ssh_key_name: env_trim("AWS_SSH_KEY_NAME"),
        worker_ami_id: env_trim("AWS_WORKER_AMI_ID").or_else(|| default_ami.clone()),
        gateway_ami_id: env_trim("AWS_GATEWAY_AMI_ID").or(default_ami),
        ssh_user: env_trim("AWS_SSH_USER"),
        artifact_bucket: env_trim("AWS_ARTIFACT_BUCKET"),
    };
    write_settings(data_dir.path(), aws_settings).await?;

    let port = pick_free_port();
    if port == 0 {
        anyhow::bail!("failed to find free port for daemon");
    }

    let mut daemon = None;
    let mut gateway = None;
    let mut worker = None;
    let base_url = format!("http://127.0.0.1:{port}");
    let client = reqwest::Client::new();
    let session_token = env_trim("AWS_SESSION_TOKEN");

    let test_result: Result<()> = async {
        let ctx_bin = env!("CARGO_BIN_EXE_ctx");
        let child = Command::new(ctx_bin)
            .arg("serve")
            .arg("--bind")
            .arg(format!("127.0.0.1:{port}"))
            .arg("--data-dir")
            .arg(data_dir.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .context("spawning ctx daemon")?;
        daemon = Some(child);

        wait_for_daemon(&base_url).await?;

        let ws: Workspace = send_json(
            client
                .post(format!("{base_url}/api/workspaces"))
                .json(&json!({"root_path": workspace_root.to_string_lossy(), "name": "aws-gateway-e2e"})),
        )
        .await?;

        let task: Task = send_json(
            client
                .post(format!("{base_url}/api/workspaces/{}/tasks", ws.id.0))
                .json(&json!({"title": "aws-gateway-e2e"})),
        )
        .await?;

        let snapshot: WorkspaceCatchupSnapshot = send_json(
            client.get(format!("{base_url}/api/workspaces/{}/catchup", ws.id.0)),
        )
        .await?;
        let track = snapshot
            .active
            .tasks
            .iter()
            .find(|summary| summary.task.id == task.id)
            .and_then(|summary| summary.tracks.first())
            .context("default track missing")?;

        let gateway_resp: AwsGatewayLaunchResp = send_json(
            client
                .post(format!("{base_url}/api/cloud_workers/aws/gateway/launch"))
                .json(&json!({})),
        )
        .await?;
        gateway = Some(gateway_resp.gateway.clone());

        let track_worker: TrackWorker = send_json(
            client
                .post(format!(
                    "{base_url}/api/tracks/{}/cloud_worker",
                    track.track.id.0
                ))
                .json(&json!({
                    "provider_id": &provider_id,
                    "model_id": &model_id,
                })),
        )
        .await?;
        worker = Some(track_worker);

        let session: ctx_core::models::Session = send_json(
            client
                .post(format!(
                    "{base_url}/api/tracks/{}/sessions",
                    track.track.id.0
                ))
                .json(&json!({
                    "provider_id": &provider_id,
                    "model_id": &model_id,
                })),
        )
        .await?;
        let session_id = session.id.0.to_string();

        let _: ctx_core::models::Message = send_json(
            client
                .post(format!(
                    "{base_url}/api/sessions/{}/messages",
                    session.id.0
                ))
                .json(&json!({ "content": ASSISTANT_PROMPT })),
        )
        .await?;

        wait_for_assistant_reply(&client, &base_url, &session_id).await?;

        Ok(())
    }
    .await;

    let mut cleanup_errors = Vec::new();
    if let Some(track_worker) = worker.as_ref() {
        let resp = client
            .delete(format!(
                "{base_url}/api/tracks/{}/worker",
                track_worker.track_id.0
            ))
            .send()
            .await;
        if let Err(err) = resp {
            cleanup_errors.push(format!("failed to stop worker: {err:#}"));
        }
    }

    if let Some(gateway) = gateway.as_ref() {
        if let Some(instance_id) = gateway.instance_id.as_deref() {
            let region = gateway
                .region
                .as_deref()
                .unwrap_or(region.as_str());
            if let Err(err) = terminate_gateway_instance(
                region,
                &access_key_id,
                &secret_access_key,
                session_token.as_deref(),
                instance_id,
            )
            .await
            {
                cleanup_errors.push(format!(
                    "failed to terminate gateway instance {instance_id}: {err:#}"
                ));
            }
        } else {
            cleanup_errors.push("gateway instance id missing in launch response".to_string());
        }
    }

    if let Some(mut child) = daemon {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }

    if let Err(err) = test_result {
        if !cleanup_errors.is_empty() {
            eprintln!("cleanup warnings: {}", cleanup_errors.join("; "));
        }
        return Err(err);
    }

    if !cleanup_errors.is_empty() {
        anyhow::bail!("cleanup failures: {}", cleanup_errors.join("; "));
    }

    Ok(())
}
