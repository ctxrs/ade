use std::sync::Arc;

use super::super::app_server::{fetch_codex_account_details, wait_for_codex_login_completion};
use super::{CodexLoginCompletion, CodexLoginProcess};
use crate::daemon::AppState;

pub(in crate::api::providers::login::codex) async fn monitor_codex_login(
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
        if let Err(err) = crate::daemon::providers::persist_successful_codex_login(
            &state,
            &account_id,
            label,
            email,
            plan_type,
        )
        .await
        {
            status.success = false;
            status.error = Some(err.to_string());
            let _ = tokio::fs::remove_dir_all(&login.account_dir).await;
        }
    } else {
        let _ = tokio::fs::remove_dir_all(&login.account_dir).await;
    }

    state
        .providers
        .with_codex_login_sessions(|map| {
            if let Some(entry) = map.get_mut(&account_id) {
                entry.status = if status.success {
                    "success".to_string()
                } else {
                    "failed".to_string()
                };
                entry.completion_token = None;
                entry.error = status.error;
            }
        })
        .await;

    let _ = login.child.kill().await;
}
