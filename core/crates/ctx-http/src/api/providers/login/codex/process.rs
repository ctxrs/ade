use super::app_server::{
    fetch_codex_account_details, send_codex_jsonrpc, spawn_codex_app_server,
    wait_for_codex_login_completion, wait_for_codex_response,
};
use super::*;

pub(super) struct CodexLoginProcess {
    pub(super) login_id: String,
    pub(super) auth_url: String,
    account_dir: PathBuf,
    child: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    reader: tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
}

pub(super) struct CodexLoginCompletion {
    pub(super) success: bool,
    pub(super) error: Option<String>,
}

pub(super) async fn start_codex_login_process(
    account_dir: &PathBuf,
    codex_bin: &str,
) -> anyhow::Result<CodexLoginProcess> {
    let mut child = spawn_codex_app_server(account_dir, codex_bin)?;
    let stdout = child
        .stdout
        .take()
        .context("codex app-server stdout unavailable")?;
    let mut reader = BufReader::new(stdout).lines();
    let mut stdin = child
        .stdin
        .take()
        .context("codex app-server stdin unavailable")?;

    let init_request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "clientInfo": {
                "name": "ctx",
                "title": "ctx",
                "version": env!("CARGO_PKG_VERSION")
            }
        }
    });
    send_codex_jsonrpc(&mut stdin, &init_request).await?;
    wait_for_codex_response(&mut reader, 1, CODEX_LOGIN_RPC_TIMEOUT).await?;
    send_codex_jsonrpc(
        &mut stdin,
        &serde_json::json!({"jsonrpc": "2.0", "method": "initialized"}),
    )
    .await?;

    let login_request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "account/login/start",
        "params": { "type": "chatgpt" }
    });
    send_codex_jsonrpc(&mut stdin, &login_request).await?;
    let response = wait_for_codex_response(&mut reader, 2, CODEX_LOGIN_RPC_TIMEOUT).await?;
    let result = response
        .get("result")
        .and_then(|v| v.as_object())
        .context("codex login missing result")?;
    let auth_url = result
        .get("authUrl")
        .or_else(|| result.get("auth_url"))
        .and_then(|v| v.as_str())
        .context("codex login missing auth_url")?
        .to_string();
    let login_id = result
        .get("loginId")
        .or_else(|| result.get("login_id"))
        .and_then(|v| v.as_str())
        .context("codex login missing login_id")?
        .to_string();

    Ok(CodexLoginProcess {
        login_id,
        auth_url,
        account_dir: account_dir.clone(),
        child,
        stdin,
        reader,
    })
}

pub(super) async fn monitor_codex_login(
    state: Arc<AppState>,
    account_id: String,
    label: String,
    mut login: CodexLoginProcess,
) {
    let completion = wait_for_codex_login_completion(&mut login.reader, &login.login_id).await;
    let mut status = match completion {
        Ok(completion) => completion,
        Err(err) => CodexLoginCompletion {
            success: false,
            error: Some(err.to_string()),
        },
    };

    if status.success {
        let (email, plan_type) = fetch_codex_account_details(&mut login.stdin, &mut login.reader)
            .await
            .unwrap_or((None, None));
        if let Err(err) =
            persist_successful_codex_login(&state, &account_id, label, email, plan_type).await
        {
            status.success = false;
            status.error = Some(err.to_string());
            let _ = tokio::fs::remove_dir_all(&login.account_dir).await;
        }
    } else {
        let _ = tokio::fs::remove_dir_all(&login.account_dir).await;
    }

    {
        let mut map = state.providers.codex_login_sessions.lock().await;
        if let Some(entry) = map.get_mut(&account_id) {
            entry.status = if status.success {
                "success".to_string()
            } else {
                "failed".to_string()
            };
            entry.completion_token = None;
            entry.error = status.error;
        }
    }

    let _ = login.child.kill().await;
}

pub(super) async fn persist_successful_codex_login(
    state: &Arc<AppState>,
    account_id: &str,
    label: String,
    email: Option<String>,
    plan_type: Option<String>,
) -> anyhow::Result<()> {
    let entry = provider_accounts::CodexAccountEntry {
        id: account_id.to_string(),
        label,
        kind: provider_accounts::CODEX_CREDENTIAL_KIND_OAUTH.to_string(),
        email,
        plan_type,
        created_at: Utc::now(),
        last_used_at: Some(Utc::now()),
        secret_ref: None,
        endpoint_profile: provider_accounts::CodexEndpointProfile::default(),
    };
    provider_accounts::upsert_codex_account(&state.core.data_root, entry)
        .await
        .with_context(|| format!("persisting codex account {account_id}"))?;

    let persist_result = async {
        let ingested = provider_accounts::ingest_codex_account_auth_to_secret_store(
            &state.core.data_root,
            account_id,
        )
        .await
        .with_context(|| format!("ingesting codex auth for account {account_id}"))?;
        if !ingested {
            anyhow::bail!("missing persisted codex auth file for account {account_id}");
        }
        provider_accounts::remove_codex_account_home_auth_if_present(
            &state.core.data_root,
            account_id,
        )
        .await
        .with_context(|| format!("removing account-home codex auth for account {account_id}"))?;
        provider_accounts::set_active_codex_account(
            &state.core.data_root,
            Some(account_id.to_string()),
        )
        .await
        .with_context(|| format!("setting active codex account {account_id}"))?;
        restarts::restart_codex_providers_for_auth_change(state, "codex auth updated")
            .await
            .context("restarting codex providers after auth change")?;
        Ok(())
    }
    .await;

    if let Err(err) = persist_result {
        let _ = provider_accounts::remove_codex_account(&state.core.data_root, account_id).await;
        return Err(err);
    }

    Ok(())
}
