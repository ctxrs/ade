use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result};
use azure_core::auth::TokenCredential;
use azure_identity::{DefaultAzureCredential, TokenCredentialOptions};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use reqwest::{Certificate, StatusCode};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::json;
use tokio::process::Command;
use tokio::time::{sleep, Instant};

use ctx_core::models::{
    MessageRole, SessionEventType, SessionHead, Task, TrackWorker, Workspace,
    WorkspaceCatchupSnapshot,
};
use ctx_http::settings::{
    AzureCloudWorkersSettings, CloudGatewaySettings, CloudWorkersSettings,
    Settings as DaemonSettings,
};

const REQUIRED_TIER: &str = "3";
const ASSISTANT_PROMPT: &str = "Reply with the exact text: cloud_gateway_azure_e2e_ok";
const ASSISTANT_EXPECTED: &str = "cloud_gateway_azure_e2e_ok";

#[derive(Debug, Deserialize)]
struct AzureGatewayLaunchResp {
    gateway: CloudGatewaySettings,
}

fn env_trim(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn env_bool(key: &str) -> Option<bool> {
    env_trim(key).and_then(|value| match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" => Some(true),
        "0" | "false" | "no" => Some(false),
        _ => None,
    })
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

async fn write_settings(data_root: &Path, azure: AzureCloudWorkersSettings) -> Result<()> {
    let settings = DaemonSettings {
        cloud_workers: Some(CloudWorkersSettings {
            gateway: None,
            aws: None,
            gcp: None,
            azure: Some(azure),
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

struct AzureArmClient {
    credential: DefaultAzureCredential,
    http: reqwest::Client,
    subscription_id: String,
}

impl AzureArmClient {
    fn new(subscription_id: String) -> Result<Self> {
        Ok(Self {
            credential: DefaultAzureCredential::create(TokenCredentialOptions::default())
                .context("azure credential init")?,
            http: reqwest::Client::new(),
            subscription_id,
        })
    }

    fn api_base(&self) -> String {
        format!(
            "https://management.azure.com/subscriptions/{}",
            self.subscription_id
        )
    }

    async fn token(&self) -> Result<String> {
        let token = self
            .credential
            .get_token(&["https://management.azure.com/.default"])
            .await
            .context("azure token")?;
        Ok(token.token.secret().to_string())
    }

    async fn delete_resource(&self, url: &str) -> Result<()> {
        for _ in 0..5 {
            let token = self.token().await?;
            let resp = self.http.delete(url).bearer_auth(&token).send().await?;
            let status = resp.status();
            if status.is_success() || status == StatusCode::NOT_FOUND {
                return Ok(());
            }
            if matches!(status, StatusCode::CONFLICT | StatusCode::TOO_MANY_REQUESTS) {
                sleep(Duration::from_secs(5)).await;
                continue;
            }
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("azure delete failed ({status}): {body}");
        }
        anyhow::bail!("azure delete failed after retries");
    }
}

fn azure_sanitize_name(prefix: &str, id: &str) -> String {
    let raw = format!("{}-{}", prefix, id);
    let mut out = String::new();
    for ch in raw.chars() {
        let ch = ch.to_ascii_lowercase();
        if ch.is_ascii_alphanumeric() || ch == '-' {
            out.push(ch);
        } else {
            out.push('-');
        }
    }
    if out.len() > 63 {
        out.truncate(63);
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

fn azure_gateway_resource_names(instance_id: &str) -> Option<(String, String, String, String)> {
    let gateway_id = instance_id.strip_prefix("ctx-gateway-")?;
    Some((
        azure_sanitize_name("ctx-gateway", gateway_id),
        azure_sanitize_name("ctx-gateway-nic", gateway_id),
        azure_sanitize_name("ctx-gateway-ip", gateway_id),
        azure_sanitize_name("ctx-gateway-nsg", gateway_id),
    ))
}

async fn delete_azure_gateway(
    arm: &AzureArmClient,
    resource_group: &str,
    instance_id: &str,
) -> Result<()> {
    let api_base = arm.api_base();
    let (vm_name, nic_name, public_ip_name, nsg_name) = azure_gateway_resource_names(instance_id)
        .unwrap_or_else(|| {
            (
                instance_id.to_string(),
                String::new(),
                String::new(),
                String::new(),
            )
        });

    let vm_url = format!(
        "{}/resourceGroups/{}/providers/Microsoft.Compute/virtualMachines/{}?api-version=2023-07-01",
        api_base, resource_group, vm_name
    );
    arm.delete_resource(&vm_url).await?;

    if !nic_name.is_empty() {
        let nic_url = format!(
            "{}/resourceGroups/{}/providers/Microsoft.Network/networkInterfaces/{}?api-version=2023-09-01",
            api_base, resource_group, nic_name
        );
        arm.delete_resource(&nic_url).await?;
    }

    if !public_ip_name.is_empty() {
        let public_ip_url = format!(
            "{}/resourceGroups/{}/providers/Microsoft.Network/publicIPAddresses/{}?api-version=2023-09-01",
            api_base, resource_group, public_ip_name
        );
        arm.delete_resource(&public_ip_url).await?;
    }

    if !nsg_name.is_empty() {
        let nsg_url = format!(
            "{}/resourceGroups/{}/providers/Microsoft.Network/networkSecurityGroups/{}?api-version=2023-09-01",
            api_base, resource_group, nsg_name
        );
        arm.delete_resource(&nsg_url).await?;
    }

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires CTX_E2E_TIER=3 and Azure credentials"]
async fn cloud_gateway_azure_e2e() -> Result<()> {
    if !e2e_tier_enabled() {
        eprintln!("skipping: set CTX_E2E_TIER=3 to run this test");
        return Ok(());
    }

    let Some(subscription_id) = env_trim("AZURE_SUBSCRIPTION_ID") else {
        eprintln!("skipping: missing AZURE_SUBSCRIPTION_ID");
        return Ok(());
    };
    let Some(resource_group) = env_trim("AZURE_RESOURCE_GROUP") else {
        eprintln!("skipping: missing AZURE_RESOURCE_GROUP");
        return Ok(());
    };
    let Some(vnet) = env_trim("AZURE_VNET") else {
        eprintln!("skipping: missing AZURE_VNET");
        return Ok(());
    };
    let Some(subnet) = env_trim("AZURE_SUBNET") else {
        eprintln!("skipping: missing AZURE_SUBNET");
        return Ok(());
    };
    let ssh_public_key = if let Some(value) = env_trim("AZURE_SSH_PUBLIC_KEY") {
        value
    } else if let Some(path) = env_trim("AZURE_SSH_PUBLIC_KEY_PATH") {
        let contents = std::fs::read_to_string(&path)
            .with_context(|| format!("reading AZURE_SSH_PUBLIC_KEY_PATH {}", path))?;
        let trimmed = contents.trim().to_string();
        if trimmed.is_empty() {
            anyhow::bail!("AZURE_SSH_PUBLIC_KEY_PATH file was empty");
        }
        trimmed
    } else {
        eprintln!("skipping: missing AZURE_SSH_PUBLIC_KEY or AZURE_SSH_PUBLIC_KEY_PATH");
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
    let disk_size_gb = env_trim("AZURE_DISK_SIZE_GB")
        .map(|value| value.parse::<i32>().context("invalid AZURE_DISK_SIZE_GB"))
        .transpose()?
        .unwrap_or_default();
    let azure_settings = AzureCloudWorkersSettings {
        subscription_id: subscription_id.clone(),
        resource_group: resource_group.clone(),
        location: env_trim("AZURE_LOCATION").unwrap_or_default(),
        vm_size: env_trim("AZURE_VM_SIZE").unwrap_or_default(),
        image: env_trim("AZURE_IMAGE").unwrap_or_default(),
        vnet,
        subnet,
        admin_username: env_trim("AZURE_ADMIN_USERNAME").unwrap_or_default(),
        ssh_public_key,
        disk_size_gb,
        disk_sku: env_trim("AZURE_DISK_SKU").unwrap_or_default(),
        delete_disk_on_pause: env_bool("AZURE_DELETE_DISK_ON_PAUSE").unwrap_or(false),
        use_public_ip: env_bool("AZURE_USE_PUBLIC_IP").unwrap_or(false),
        artifact_storage_account: env_trim("AZURE_ARTIFACT_STORAGE_ACCOUNT"),
        artifact_container: env_trim("AZURE_ARTIFACT_CONTAINER"),
    };
    write_settings(data_dir.path(), azure_settings).await?;

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
            &json!({"root_path": workspace_root.to_string_lossy(), "name": "azure-gateway-e2e"}),
        ))
        .await?;

        let task: Task = send_json(
            client
                .post(format!("{base_url}/api/workspaces/{}/tasks", ws.id.0))
                .json(&json!({"title": "azure-gateway-e2e"})),
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

        let gateway_resp: AzureGatewayLaunchResp = send_json(
            client
                .post(format!("{base_url}/api/cloud_workers/azure/gateway/launch"))
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
                    track.track.id.0
                ))
                .json(&json!({
                    "provider_id": &provider_id,
                    "model_id": &model_id,
                })),
        )
        .await?;
        eprintln!(
            "worker started track_id={} worker_id={}",
            track_worker.track_id.0, track_worker.worker_id
        );
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
                .post(format!("{base_url}/api/sessions/{}/messages", session.id.0))
                .json(&json!({ "content": ASSISTANT_PROMPT })),
        )
        .await?;

        wait_for_assistant_reply(&client, &base_url, &session_id).await?;

        Ok(())
    }
    .await;

    let mut cleanup_errors = Vec::new();
    if !keep_resources {
        if let (Some(track_worker), Some(client)) = (worker.as_ref(), client.as_ref()) {
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
                match AzureArmClient::new(subscription_id.clone()) {
                    Ok(arm) => {
                        if let Err(err) =
                            delete_azure_gateway(&arm, &resource_group, instance_id).await
                        {
                            cleanup_errors.push(format!(
                                "failed to delete gateway resources {instance_id}: {err:#}"
                            ));
                        }
                    }
                    Err(err) => cleanup_errors.push(format!(
                        "failed to initialize azure client for cleanup: {err:#}"
                    )),
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
