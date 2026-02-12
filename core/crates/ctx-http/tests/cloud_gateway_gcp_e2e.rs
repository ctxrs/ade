#![allow(unexpected_cfgs)]
#![cfg(feature = "cloud_gateway_e2e")]

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use gcp_auth::{provider as gcp_provider, TokenProvider};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use reqwest::{Certificate, Method, StatusCode};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::json;
use tokio::process::Command;
use tokio::time::{sleep, Instant};

use ctx_core::models::{MessageRole, SessionEventType, SessionHead, Task, Workspace};
use ctx_http::settings::Settings as DaemonSettings;
use ctx_http::settings::{CloudGatewaySettings, CloudWorkersSettings, GcpCloudWorkersSettings};
use ctx_worker_protocol::{WorkerInfo as GatewayWorkerInfo, WorkerState as GatewayWorkerState};

const REQUIRED_TIER: &str = "3";
const ASSISTANT_PROMPT: &str = "Reply with the exact text: cloud_gateway_gcp_e2e_ok";
const ASSISTANT_EXPECTED: &str = "cloud_gateway_gcp_e2e_ok";
const DEFAULT_GCP_ZONE: &str = "us-central1-a";
const DEFAULT_GCP_MACHINE_TYPE: &str = "e2-standard-2";
const DEFAULT_GCP_IMAGE: &str = "projects/debian-cloud/global/images/family/debian-12";
const DEFAULT_GCP_DISK_SIZE_GB: i64 = 50;
const DEFAULT_GCP_DISK_TYPE: &str = "pd-standard";

#[derive(Debug, Deserialize)]
struct WorkspaceCatchupSnapshot {
    active: WorkspaceCatchupActive,
}

#[derive(Debug, Deserialize)]
struct WorkspaceCatchupActive {
    tasks: Vec<WorkspaceCatchupTaskSummary>,
}

#[derive(Debug, Deserialize)]
struct WorkspaceCatchupTaskSummary {
    task: Task,
    tracks: Vec<WorkspaceCatchupTrackSummary>,
}

#[derive(Debug, Deserialize)]
struct WorkspaceCatchupTrackSummary {
    track: WorkspaceCatchupTrack,
}

#[derive(Debug, Deserialize)]
struct WorkspaceCatchupTrack {
    id: String,
}

#[derive(Debug, Deserialize)]
struct TrackWorker {
    track_id: String,
    worker_id: String,
}

#[derive(Debug, Deserialize)]
struct GcpGatewayLaunchResp {
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

fn parse_bool_env(key: &str) -> Option<bool> {
    env_trim(key).map(|value| matches!(value.to_ascii_lowercase().as_str(), "1" | "true"))
}

fn parse_scopes_env() -> Option<Vec<String>> {
    env_trim("GCP_SCOPES").map(|value| {
        value
            .split(',')
            .map(|entry| entry.trim().to_string())
            .filter(|entry| !entry.is_empty())
            .collect::<Vec<_>>()
    })
}

fn parse_i64_env(key: &str) -> Option<i64> {
    env_trim(key).and_then(|value| value.parse::<i64>().ok())
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

async fn load_daemon_auth_token(data_root: &Path) -> Result<String> {
    let path = data_root.join("daemon_auth.json");
    for _ in 0..50 {
        if let Ok(contents) = tokio::fs::read_to_string(&path).await {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&contents) {
                if let Some(token) = value.get("token").and_then(|v| v.as_str()) {
                    if !token.trim().is_empty() {
                        return Ok(token.trim().to_string());
                    }
                }
            }
        }
        sleep(Duration::from_millis(200)).await;
    }
    anyhow::bail!("daemon auth token not found")
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

async fn write_settings(data_root: &Path, gcp: GcpCloudWorkersSettings) -> Result<()> {
    let settings = DaemonSettings {
        cloud_workers: Some(CloudWorkersSettings {
            gateway: None,
            aws: None,
            gcp: Some(gcp),
            azure: None,
        }),
        ..DaemonSettings::default()
    };
    let db_path = data_root.join("db").join("db.sqlite");
    let store = ctx_store::Store::open_sqlite(&db_path, None)
        .await
        .with_context(|| format!("opening {}", db_path.display()))?;
    ctx_http::settings::save_settings(&store, &settings)
        .await
        .context("saving settings")?;
    store.close().await;
    Ok(())
}

async fn send_json<T: DeserializeOwned>(request: reqwest::RequestBuilder) -> Result<T> {
    let resp = request.send().await.context("request failed")?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("request failed ({status}): {body}");
    }
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
            matches!(msg.role, MessageRole::Assistant) && msg.content.contains(ASSISTANT_EXPECTED)
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

async fn wait_for_gateway_health(gateway: &CloudGatewaySettings) -> Result<()> {
    let health_url = format!("{}/health", gateway.gateway_url.trim_end_matches('/'));
    let mut builder = reqwest::Client::builder();
    if let Some(pem) = gateway.gateway_ca_pem.as_deref() {
        let cert = Certificate::from_pem(pem.as_bytes()).context("gateway CA PEM invalid")?;
        builder = builder
            .add_root_certificate(cert)
            .danger_accept_invalid_hostnames(true);
    }
    let client = builder.build().context("building gateway client")?;
    for _ in 0..60 {
        if let Ok(resp) = client.get(&health_url).send().await {
            if resp.status().is_success() {
                return Ok(());
            }
        }
        sleep(Duration::from_secs(2)).await;
    }
    anyhow::bail!("gateway did not become healthy")
}

async fn wait_for_worker_ready(
    gateway: &CloudGatewaySettings,
    worker_id: &str,
) -> Result<GatewayWorkerInfo> {
    let base_url = gateway.gateway_url.trim_end_matches('/');
    let url = format!("{base_url}/workers/{worker_id}");

    let mut builder = reqwest::Client::builder();
    if let Some(pem) = gateway.gateway_ca_pem.as_deref() {
        let cert = Certificate::from_pem(pem.as_bytes()).context("gateway CA PEM invalid")?;
        builder = builder
            .add_root_certificate(cert)
            .danger_accept_invalid_hostnames(true);
    }
    let client = builder.build().context("building gateway worker client")?;

    let deadline = Instant::now() + Duration::from_secs(300);
    loop {
        if let Ok(resp) = client.get(&url).send().await {
            if resp.status().is_success() {
                let info: GatewayWorkerInfo = resp.json().await.context("parsing worker info")?;
                if matches!(info.state, GatewayWorkerState::Running) {
                    return Ok(info);
                }
            }
        }

        if Instant::now() >= deadline {
            anyhow::bail!("timed out waiting for worker readiness");
        }
        sleep(Duration::from_secs(2)).await;
    }
}

struct GcpTestClient {
    auth: Arc<dyn TokenProvider>,
    http: reqwest::Client,
}

impl GcpTestClient {
    fn new(auth: Arc<dyn TokenProvider>) -> Self {
        Self {
            auth,
            http: reqwest::Client::new(),
        }
    }

    async fn token(&self) -> Result<String> {
        let scopes = ["https://www.googleapis.com/auth/cloud-platform"];
        let token = self.auth.token(&scopes).await.context("gcp token")?;
        Ok(token.as_str().to_string())
    }

    async fn request_json(
        &self,
        method: Method,
        url: &str,
        body: Option<serde_json::Value>,
    ) -> Result<serde_json::Value> {
        let token = self.token().await?;
        let mut req = self.http.request(method, url).bearer_auth(token);
        if let Some(body) = body {
            req = req.json(&body);
        }
        let resp = req.send().await.context("gcp request")?;
        let status = resp.status();
        let text = resp.text().await.context("gcp response text")?;
        if !status.is_success() {
            anyhow::bail!("gcp request failed: {status} {text}");
        }
        if text.trim().is_empty() {
            return Ok(serde_json::Value::Null);
        }
        serde_json::from_str(&text).context("gcp response json")
    }
}

fn gcp_compute_base(project_id: &str) -> String {
    format!("https://compute.googleapis.com/compute/v1/projects/{project_id}")
}

async fn gcp_wait_zone_operation(
    client: &GcpTestClient,
    project_id: &str,
    zone: &str,
    op_name: &str,
) -> Result<()> {
    let url = format!(
        "{}/zones/{}/operations/{}",
        gcp_compute_base(project_id),
        zone,
        op_name
    );
    for _ in 0..120 {
        let resp = client.request_json(Method::GET, &url, None).await?;
        let status = resp.get("status").and_then(|v| v.as_str());
        if status == Some("DONE") {
            if let Some(error) = resp.get("error") {
                anyhow::bail!("gcp operation error: {}", error);
            }
            return Ok(());
        }
        sleep(Duration::from_secs(5)).await;
    }
    anyhow::bail!("gcp operation timeout")
}

async fn gcp_delete_instance(
    client: &GcpTestClient,
    project_id: &str,
    zone: &str,
    instance_id: &str,
) -> Result<()> {
    let url = format!(
        "{}/zones/{}/instances/{}",
        gcp_compute_base(project_id),
        zone,
        instance_id
    );
    let token = client.token().await?;
    let resp = client
        .http
        .request(Method::DELETE, &url)
        .bearer_auth(token)
        .send()
        .await
        .context("gcp delete request")?;
    if resp.status() == StatusCode::NOT_FOUND {
        return Ok(());
    }
    let status = resp.status();
    let text = resp.text().await.context("gcp delete response text")?;
    if !status.is_success() {
        anyhow::bail!("gcp delete failed: {status} {text}");
    }
    if text.trim().is_empty() {
        return Ok(());
    }
    let resp =
        serde_json::from_str::<serde_json::Value>(&text).context("gcp delete response json")?;
    let op_name = resp
        .get("name")
        .and_then(|v| v.as_str())
        .context("missing gcp delete op name")?;
    gcp_wait_zone_operation(client, project_id, zone, op_name).await
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires CTX_E2E_TIER=3 and GCP credentials"]
async fn cloud_gateway_gcp_e2e() -> Result<()> {
    if !e2e_tier_enabled() {
        eprintln!("skipping: set CTX_E2E_TIER=3 to run this test");
        return Ok(());
    }
    if let Err(err) = rustls::crypto::aws_lc_rs::default_provider().install_default() {
        eprintln!("warning: failed to install rustls crypto provider: {err:?}");
    }

    let Some(project_id) = env_trim("GCP_PROJECT_ID") else {
        eprintln!("skipping: missing GCP_PROJECT_ID");
        return Ok(());
    };
    let service_account =
        match env_trim("GCP_SERVICE_ACCOUNT").or_else(|| env_trim("GCP_SERVICE_ACCOUNT_EMAIL")) {
            Some(value) => value,
            None => {
                eprintln!("skipping: missing GCP_SERVICE_ACCOUNT");
                return Ok(());
            }
        };
    let gcp_auth = match gcp_provider().await {
        Ok(auth) => auth,
        Err(err) => {
            eprintln!("skipping: gcp auth unavailable: {err:#}");
            return Ok(());
        }
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
    let zone = env_trim("GCP_ZONE").unwrap_or_else(|| DEFAULT_GCP_ZONE.to_string());
    let machine_type =
        env_trim("GCP_MACHINE_TYPE").unwrap_or_else(|| DEFAULT_GCP_MACHINE_TYPE.to_string());
    let image = env_trim("GCP_IMAGE").unwrap_or_else(|| DEFAULT_GCP_IMAGE.to_string());
    let disk_size_gb = parse_i64_env("GCP_DISK_SIZE_GB")
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_GCP_DISK_SIZE_GB);
    let disk_type = env_trim("GCP_DISK_TYPE").unwrap_or_else(|| DEFAULT_GCP_DISK_TYPE.to_string());
    let delete_disk_on_pause = parse_bool_env("GCP_DELETE_DISK_ON_PAUSE").unwrap_or(false);
    let scopes = parse_scopes_env().filter(|value| !value.is_empty());
    let gcp_settings = GcpCloudWorkersSettings {
        project_id: project_id.clone(),
        zone: zone.clone(),
        machine_type: machine_type.clone(),
        image,
        network: env_trim("GCP_NETWORK"),
        subnetwork: env_trim("GCP_SUBNETWORK"),
        service_account: Some(service_account.clone()),
        scopes,
        disk_size_gb: Some(disk_size_gb),
        disk_type: Some(disk_type),
        ssh_user: env_trim("GCP_SSH_USER"),
        delete_disk_on_pause: Some(delete_disk_on_pause),
        artifact_bucket: env_trim("GCP_ARTIFACT_BUCKET"),
    };
    write_settings(data_dir.path(), gcp_settings).await?;

    let port = pick_free_port();
    if port == 0 {
        anyhow::bail!("failed to find free port for daemon");
    }

    let mut daemon = None;
    let mut gateway = None;
    let mut worker = None;
    let base_url = format!("http://127.0.0.1:{port}");
    let mut client: Option<reqwest::Client> = None;
    let keep_resources = std::env::var("CTX_E2E_KEEP_RESOURCES")
        .ok()
        .is_some_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true"));

    let test_result: Result<()> = async {
        let ctx_bin = env!("CARGO_BIN_EXE_ctx");
        let mut daemon_cmd = Command::new(ctx_bin);
        daemon_cmd
            .arg("serve")
            .arg("--bind")
            .arg(format!("127.0.0.1:{port}"))
            .arg("--data-dir")
            .arg(data_dir.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        if provider_id == "fake" {
            daemon_cmd.env("CTX_SHOW_FAKE_PROVIDER", "1");
        }
        let child = daemon_cmd.spawn().context("spawning ctx daemon")?;
        daemon = Some(child);

        wait_for_daemon(&base_url).await?;

        let auth_token = load_daemon_auth_token(data_dir.path()).await?;
        let mut headers = HeaderMap::new();
        let header_value = format!("Bearer {}", auth_token);
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&header_value).context("invalid auth header")?,
        );
        let client_inner = reqwest::Client::builder()
            .default_headers(headers)
            .build()
            .context("building http client")?;
        client = Some(client_inner);
        let client = client.as_ref().context("http client missing")?;

        let ws: Workspace = send_json(client.post(format!("{base_url}/api/workspaces")).json(
            &json!({"root_path": workspace_root.to_string_lossy(), "name": "gcp-gateway-e2e"}),
        ))
        .await?;

        let task: Task = send_json(
            client
                .post(format!("{base_url}/api/workspaces/{}/tasks", ws.id.0))
                .json(&json!({"title": "gcp-gateway-e2e"})),
        )
        .await?;

        let snapshot: WorkspaceCatchupSnapshot =
            send_json(client.get(format!("{base_url}/api/workspaces/{}/catchup", ws.id.0))).await?;
        let track = snapshot
            .active
            .tasks
            .iter()
            .find(|summary| summary.task.id == task.id)
            .and_then(|summary| summary.tracks.first())
            .context("default track missing")?;

        let gateway_resp: GcpGatewayLaunchResp = send_json(
            client
                .post(format!("{base_url}/api/cloud_workers/gcp/gateway/launch"))
                .json(&json!({ "workspace_id": ws.id.0 })),
        )
        .await?;
        gateway = Some(gateway_resp.gateway.clone());
        if let Some(instance_id) = gateway_resp.gateway.instance_id.as_deref() {
            eprintln!(
                "gateway instance_id={} url={}",
                instance_id, gateway_resp.gateway.gateway_url
            );
        }
        wait_for_gateway_health(&gateway_resp.gateway).await?;

        let track_worker: TrackWorker = send_json(
            client
                .post(format!(
                    "{base_url}/api/tracks/{}/cloud_worker",
                    track.track.id
                ))
                .json(&json!({
                    "provider_id": &provider_id,
                    "model_id": &model_id,
                })),
        )
        .await?;
        eprintln!(
            "worker started track_id={} worker_id={}",
            track_worker.track_id, track_worker.worker_id
        );
        let worker_id = track_worker.worker_id.clone();
        worker = Some(track_worker);
        let _ = wait_for_worker_ready(&gateway_resp.gateway, &worker_id).await?;

        let session: ctx_core::models::Session = send_json(
            client
                .post(format!("{base_url}/api/tracks/{}/sessions", track.track.id))
                .json(&json!({
                    "provider_id": &provider_id,
                    "model_id": &model_id,
                })),
        )
        .await?;
        let session_id = session.id.0.to_string();

        let _: ctx_core::models::Message = send_json(
            client
                .post(format!("{base_url}/api/sessions/{}/messages", session.id.0))
                .json(&json!({ "content": ASSISTANT_PROMPT })),
        )
        .await?;

        wait_for_assistant_reply(client, &base_url, &session_id).await?;

        Ok(())
    }
    .await;

    let mut cleanup_errors = Vec::new();
    if !keep_resources {
        if let (Some(track_worker), Some(client)) = (worker.as_ref(), client.as_ref()) {
            let resp = client
                .delete(format!(
                    "{base_url}/api/tracks/{}/worker",
                    track_worker.track_id
                ))
                .send()
                .await;
            if let Err(err) = resp {
                cleanup_errors.push(format!("failed to stop worker: {err:#}"));
            }
        }

        if let Some(gateway) = gateway.as_ref() {
            if let Some(instance_id) = gateway.instance_id.as_deref() {
                let gcp_client = GcpTestClient::new(gcp_auth.clone());
                if let Err(err) =
                    gcp_delete_instance(&gcp_client, &project_id, &zone, instance_id).await
                {
                    cleanup_errors.push(format!(
                        "failed to delete gateway instance {instance_id}: {err:#}"
                    ));
                }
            } else {
                cleanup_errors.push("gateway instance id missing in launch response".to_string());
            }
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
