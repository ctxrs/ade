use ctx_core::ids::WorkspaceId;
use ctx_core::models::{Worktree, WorktreeBootstrapNotice, WorktreeBootstrapStatus};
use ctx_store::WorktreeBootstrapResultUpdate;
use ctx_workspace_services::worktree_bootstrap::{
    prepare_bootstrap_log_for_storage, write_bootstrap_log, BootstrapReport,
};

use crate::daemon::AppState;

pub(super) async fn persist_bootstrap_report(
    state: &AppState,
    workspace_id: WorkspaceId,
    worktree: &Worktree,
    report: BootstrapReport,
) {
    let (log, log_truncated) = prepare_bootstrap_log_for_storage(&report.raw_log);
    let log_path = write_bootstrap_log(&state.core.data_root, worktree.id, &log)
        .await
        .ok();

    update_bootstrap_result(
        state,
        WorktreeBootstrapResultUpdate {
            worktree_id: worktree.id,
            status: report.status.clone(),
            started_at: report.started_at,
            finished_at: report.finished_at,
            exit_code: report.exit_code,
            timeout_sec: Some(report.timeout_sec),
            error: report.error.clone(),
            log_path: log_path.as_ref().map(|p| p.to_string_lossy().to_string()),
            log_truncated: Some(log_truncated),
            command: report.command.clone(),
            script_path: None,
        },
    )
    .await;

    if report.status != WorktreeBootstrapStatus::Success {
        let notice = WorktreeBootstrapNotice {
            worktree_id: worktree.id,
            worktree_root: worktree.root_path.clone(),
            status: report.status,
            started_at: report.started_at,
            finished_at: report.finished_at,
            exit_code: report.exit_code,
            timeout_sec: Some(report.timeout_sec),
            command: report.command,
            script_path: None,
            log_path: log_path.map(|p| p.to_string_lossy().to_string()),
            log_truncated: Some(log_truncated),
            error: report.error,
        };
        emit_failure_notice(state, workspace_id, notice).await;
    }
}

async fn update_bootstrap_result(state: &AppState, update: WorktreeBootstrapResultUpdate) {
    let store = match state.store_for_worktree(update.worktree_id).await {
        Ok(store) => store,
        Err(_) => return,
    };
    let _ = store.update_worktree_bootstrap_result(update).await;
}

async fn emit_failure_notice(
    state: &AppState,
    workspace_id: WorkspaceId,
    notice: WorktreeBootstrapNotice,
) {
    state
        .workspaces
        .workspace_active_snapshot
        .publish_worktree_bootstrap(workspace_id, notice)
        .await;
}
