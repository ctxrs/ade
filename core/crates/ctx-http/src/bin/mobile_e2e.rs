use std::path::Path;

use anyhow::{anyhow, Context, Result};
use futures::StreamExt;
use tokio_tungstenite::connect_async;
use uuid::Uuid;

use ctx_core::models::{WorkspaceActiveSnapshotEvent, WorkspaceActiveSnapshotStreamMessage};
use ctx_transport_runtime::mobile_e2ee;

use crypto::{
    build_ws_url, decode_body_b64, encrypt_pairing_request, encrypt_secure_request,
    parse_qr_payload,
};
use dto::{
    EnableMobileAccessReq, EnableMobileAccessResp, PairMobileDevicePayload, SecureEnvelope,
    SecureResponsePayload,
};
use harness::{
    create_workspace, init_git_repo, pick_port, read_daemon_auth_token, wait_for_health,
    wait_for_public_tunnel,
};

#[path = "mobile_e2e/crypto.rs"]
mod crypto;
#[path = "mobile_e2e/dto.rs"]
mod dto;
#[path = "mobile_e2e/harness.rs"]
mod harness;

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
    repo_root: &Path,
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
    wait_for_public_tunnel(&client, &base_url).await?;

    let (device_public, device_secret) = mobile_e2ee::generate_keypair();
    let device_id = Uuid::new_v4().to_string();
    let key = mobile_e2ee::derive_client_key(&device_id, &device_secret, &daemon_public_key)?;
    let pair_req = encrypt_pairing_request(
        &key,
        &device_id,
        &device_public,
        PairMobileDevicePayload {
            pairing_token,
            device_label: Some("e2e-harness".to_string()),
            platform: Some("harness".to_string()),
            app_version: Some("0.0.0".to_string()),
        },
    )?;

    let pair_env = client
        .post(format!("{base_url}/api/mobile/pair"))
        .json(&pair_req)
        .send()
        .await?
        .error_for_status()
        .context("pair device")?
        .json::<SecureEnvelope>()
        .await?;

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
    if !health.is_object() {
        return Err(anyhow!("secure response health body is not a JSON object"));
    }

    let ws_url = build_ws_url(&base_url, &expected_workspace_id, &device_id, &key)?;
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
