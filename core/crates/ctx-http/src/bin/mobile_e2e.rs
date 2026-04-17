use std::io::ErrorKind;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use base64::Engine;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tokio_tungstenite::connect_async;
use url::Url;
use uuid::Uuid;

use ctx_core::models::{WorkspaceActiveSnapshotEvent, WorkspaceActiveSnapshotStreamMessage};
use ctx_transport_runtime::mobile_e2ee;

#[derive(Debug, Deserialize)]
struct EnableMobileAccessResp {
    qr_payload: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct EnableMobileAccessReq {
    supabase_token: String,
}

#[derive(Debug, Deserialize)]
struct DaemonAuthFile {
    token: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct SecureEnvelope {
    device_id: String,
    seq: i64,
    nonce: String,
    ciphertext: String,
}

#[derive(Debug, Serialize)]
struct PairMobileDeviceReq {
    pairing_token: String,
    device_id: String,
    device_label: Option<String>,
    platform: Option<String>,
    public_key: String,
    app_version: Option<String>,
}

#[derive(Debug, Serialize)]
struct SecureRequestPayload {
    method: String,
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    query: Option<String>,
    #[serde(default)]
    headers: Vec<(String, String)>,
    #[serde(default)]
    body_b64: String,
}

#[derive(Debug, Deserialize)]
struct SecureResponsePayload {
    status: u16,
    body_b64: String,
}

#[derive(Debug, Deserialize)]
struct WorkspaceSummary {
    id: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let supabase_token = std::env::var("CTX_SUPABASE_ACCESS_TOKEN")
        .or_else(|_| std::env::var("SUPABASE_ACCESS_TOKEN"))
        .context("missing CTX_SUPABASE_ACCESS_TOKEN")?;
    let control_plane_url = std::env::var("CTX_TUNNEL_CONTROL_PLANE_URL")
        .context("missing CTX_TUNNEL_CONTROL_PLANE_URL")?;

    std::env::set_var("CTX_TUNNEL_CONTROL_PLANE_URL", &control_plane_url);
    std::env::set_var("CTX_ACP_PREWARM_PROVIDERS", "");

    let home_dir = tempfile::tempdir().context("create temp home dir")?;
    std::env::set_var("HOME", home_dir.path());

    let data_dir = tempfile::tempdir().context("create temp data dir")?;
    let repo_dir = tempfile::tempdir().context("create temp repo dir")?;
    init_git_repo(repo_dir.path())?;

    let port = pick_port().context("pick daemon port")?;
    let bind = format!("127.0.0.1:{port}");
    let daemon_url = format!("http://127.0.0.1:{port}");

    let serve_task = tokio::spawn(ctx_http::daemon::serve(
        vec![bind],
        Some(data_dir.path().to_string_lossy().to_string()),
    ));

    let auth_token = read_daemon_auth_token(data_dir.path()).await?;
    let outcome = run_e2e(&daemon_url, &auth_token, &supabase_token, repo_dir.path()).await;

    serve_task.abort();
    outcome
}

async fn run_e2e(
    daemon_url: &str,
    auth_token: &str,
    supabase_token: &str,
    repo_root: &std::path::Path,
) -> Result<()> {
    let client = reqwest::Client::new();

    wait_for_health(&client, daemon_url).await?;

    let expected_workspace_id =
        create_workspace(&client, daemon_url, auth_token, repo_root).await?;

    let enable_resp = client
        .post(format!("{daemon_url}/api/mobile/access/enable"))
        .bearer_auth(auth_token)
        .json(&EnableMobileAccessReq {
            supabase_token: supabase_token.to_string(),
        })
        .send()
        .await?
        .error_for_status()
        .context("enable mobile access")?
        .json::<EnableMobileAccessResp>()
        .await?;

    let (base_url, pairing_token, daemon_public_key) = parse_qr_payload(&enable_resp.qr_payload)?;

    let (device_public, device_secret) = mobile_e2ee::generate_keypair();
    let device_id = Uuid::new_v4().to_string();

    let pair_env = client
        .post(format!("{base_url}/api/mobile/pair"))
        .json(&PairMobileDeviceReq {
            pairing_token,
            device_id: device_id.clone(),
            device_label: Some("e2e-harness".to_string()),
            platform: Some("harness".to_string()),
            public_key: device_public,
            app_version: Some("0.0.0".to_string()),
        })
        .send()
        .await?
        .error_for_status()
        .context("pair device")?
        .json::<SecureEnvelope>()
        .await?;

    let key = mobile_e2ee::derive_client_key(&device_id, &device_secret, &daemon_public_key)?;

    let pair_payload = mobile_e2ee::decrypt(
        &key,
        &device_id,
        pair_env.seq,
        &pair_env.nonce,
        &pair_env.ciphertext,
    )?;
    let pair_value: serde_json::Value = serde_json::from_slice(&pair_payload)?;
    if pair_value.get("paired").and_then(|v| v.as_bool()) != Some(true) {
        return Err(anyhow!("pairing response missing paired=true"));
    }

    let secure_env = encrypt_secure_request(&key, &device_id, 1, "/api/health")?;
    let secure_resp = client
        .post(format!("{base_url}/api/mobile/secure"))
        .json(&secure_env)
        .send()
        .await?
        .error_for_status()
        .context("mobile secure request")?
        .json::<SecureEnvelope>()
        .await?;

    let secure_payload = mobile_e2ee::decrypt(
        &key,
        &device_id,
        secure_resp.seq,
        &secure_resp.nonce,
        &secure_resp.ciphertext,
    )?;
    let secure_resp_payload: SecureResponsePayload = serde_json::from_slice(&secure_payload)?;
    if secure_resp_payload.status != 200 {
        return Err(anyhow!(
            "secure response status {}",
            secure_resp_payload.status
        ));
    }
    let body = decode_body_b64(&secure_resp_payload.body_b64)?;
    let health: serde_json::Value = serde_json::from_slice(&body)?;
    if health.get("daemon_url").is_none() {
        return Err(anyhow!("secure response missing daemon_url"));
    }

    let ws_url = build_ws_url(&base_url, &expected_workspace_id, &device_id)?;
    let (mut ws, _) = connect_async(ws_url.as_str()).await?;
    let msg = ws
        .next()
        .await
        .ok_or_else(|| anyhow!("missing ws message"))??;
    let text = match msg {
        tokio_tungstenite::tungstenite::Message::Text(text) => text,
        other => return Err(anyhow!("unexpected ws message: {other:?}")),
    };
    let ws_env: SecureEnvelope = serde_json::from_str(&text)?;
    let ws_payload = mobile_e2ee::decrypt(
        &key,
        &device_id,
        ws_env.seq,
        &ws_env.nonce,
        &ws_env.ciphertext,
    )?;
    let message: WorkspaceActiveSnapshotStreamMessage = serde_json::from_slice(&ws_payload)?;
    match message {
        WorkspaceActiveSnapshotStreamMessage::Event { event, .. } => match event.as_ref() {
            WorkspaceActiveSnapshotEvent::Ready { workspace_id, .. } => {
                if workspace_id.0.to_string() != expected_workspace_id {
                    return Err(anyhow!("workspace id mismatch in ws ready event"));
                }
            }
            other => return Err(anyhow!("unexpected ws message: {other:?}")),
        },
        other => return Err(anyhow!("unexpected ws message: {other:?}")),
    }

    println!("mobile e2e ok");
    Ok(())
}

fn parse_qr_payload(payload: &serde_json::Value) -> Result<(String, String, String)> {
    let base_url = payload
        .get("base_url")
        .and_then(|v| v.as_str())
        .map(|v| v.trim_end_matches('/').to_string())
        .context("qr payload missing base_url")?;
    let pairing_token = payload
        .get("pairing_token")
        .and_then(|v| v.as_str())
        .context("qr payload missing pairing_token")?
        .to_string();
    let daemon_public_key = payload
        .get("daemon_public_key")
        .and_then(|v| v.as_str())
        .context("qr payload missing daemon_public_key")?
        .to_string();
    Ok((base_url, pairing_token, daemon_public_key))
}

async fn create_workspace(
    client: &reqwest::Client,
    daemon_url: &str,
    auth_token: &str,
    repo_root: &std::path::Path,
) -> Result<String> {
    let resp = client
        .post(format!("{daemon_url}/api/workspaces"))
        .bearer_auth(auth_token)
        .json(&serde_json::json!({
            "root_path": repo_root.to_string_lossy(),
            "name": "e2e",
        }))
        .send()
        .await?
        .error_for_status()
        .context("create workspace")?
        .json::<WorkspaceSummary>()
        .await?;
    Ok(resp.id)
}

fn encrypt_secure_request(
    key: &mobile_e2ee::E2eeKey,
    device_id: &str,
    seq: i64,
    path: &str,
) -> Result<SecureEnvelope> {
    let req_payload = SecureRequestPayload {
        method: "GET".to_string(),
        path: path.to_string(),
        query: None,
        headers: Vec::new(),
        body_b64: String::new(),
    };
    let payload_bytes = serde_json::to_vec(&req_payload)?;
    let enc = mobile_e2ee::encrypt(key, device_id, seq, &payload_bytes)?;
    Ok(SecureEnvelope {
        device_id: enc.device_id,
        seq: enc.seq,
        nonce: enc.nonce_b64,
        ciphertext: enc.ciphertext_b64,
    })
}

fn build_ws_url(base_url: &str, workspace_id: &str, device_id: &str) -> Result<Url> {
    let mut url = Url::parse(base_url)?;
    let ws_scheme = match url.scheme() {
        "https" => "wss",
        "http" => "ws",
        other => return Err(anyhow!("unsupported base url scheme: {other}")),
    };
    url.set_scheme(ws_scheme)
        .map_err(|_| anyhow!("failed to set ws scheme"))?;
    let prefix = url.path().trim_end_matches('/');
    let path = if prefix.is_empty() {
        format!("/api/mobile/secure/workspaces/{workspace_id}/stream")
    } else {
        format!("{prefix}/api/mobile/secure/workspaces/{workspace_id}/stream")
    };
    url.set_path(&path);
    url.set_query(Some(&format!("device_id={device_id}")));
    Ok(url)
}

fn decode_body_b64(value: &str) -> Result<Vec<u8>> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let mut normalized = trimmed.replace('-', "+").replace('_', "/");
    while !normalized.len().is_multiple_of(4) {
        normalized.push('=');
    }
    base64::engine::general_purpose::STANDARD
        .decode(normalized.as_bytes())
        .map_err(|_| anyhow!("invalid base64 body"))
}

async fn wait_for_health(client: &reqwest::Client, daemon_url: &str) -> Result<()> {
    for _ in 0..60 {
        match client.get(format!("{daemon_url}/api/health")).send().await {
            Ok(resp) if resp.status().is_success() => return Ok(()),
            _ => tokio::time::sleep(std::time::Duration::from_millis(500)).await,
        }
    }
    Err(anyhow!("timed out waiting for daemon health"))
}

async fn read_daemon_auth_token(data_dir: &Path) -> Result<String> {
    let path = data_dir.join("daemon_auth.json");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match tokio::fs::read(&path).await {
            Ok(bytes) => {
                let auth: DaemonAuthFile = serde_json::from_slice(&bytes)
                    .with_context(|| format!("parsing daemon auth file {}", path.display()))?;
                if auth.token.trim().is_empty() {
                    return Err(anyhow!(
                        "daemon auth file {} contains empty token",
                        path.display()
                    ));
                }
                return Ok(auth.token);
            }
            Err(err) if err.kind() == ErrorKind::NotFound => {}
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("reading daemon auth file {}", path.display()));
            }
        }
        if Instant::now() > deadline {
            return Err(anyhow!("daemon auth file not found at {}", path.display()));
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

fn pick_port() -> Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    Ok(port)
}

fn init_git_repo(path: &std::path::Path) -> Result<()> {
    run_cmd(
        std::process::Command::new("git")
            .arg("-C")
            .arg(path)
            .arg("init"),
    )?;
    run_cmd(std::process::Command::new("git").arg("-C").arg(path).args([
        "config",
        "user.email",
        "e2e@example.com",
    ]))?;
    run_cmd(std::process::Command::new("git").arg("-C").arg(path).args([
        "config",
        "user.name",
        "E2E",
    ]))?;
    std::fs::write(path.join("README.md"), "e2e\n")?;
    run_cmd(
        std::process::Command::new("git")
            .arg("-C")
            .arg(path)
            .args(["add", "."]),
    )?;
    run_cmd(
        std::process::Command::new("git")
            .arg("-C")
            .arg(path)
            .args(["commit", "-m", "init"]),
    )?;
    Ok(())
}

fn run_cmd(cmd: &mut std::process::Command) -> Result<()> {
    let output = cmd.output()?;
    if output.status.success() {
        return Ok(());
    }
    Err(anyhow!(
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    ))
}
