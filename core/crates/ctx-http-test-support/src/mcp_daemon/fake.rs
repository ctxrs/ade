use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use ctx_core::models::{ExecutionEnvironment, Session, Task, VcsKind, Workspace, Worktree};
use ctx_daemon::test_support::TestDaemon;
use ctx_providers::adapters::ProviderAdapter;
use ctx_providers::fake::FakeProviderAdapter;
use ctx_store::StoreManager;

use super::{git, router, DaemonBackedParentSession};

pub(crate) async fn setup_fake_provider_parent_session() -> Result<DaemonBackedParentSession> {
    let repo = git::init_git_repo().await?;
    let data_dir = tempfile::tempdir().context("create daemon data dir")?;
    let stores = StoreManager::open(data_dir.path())
        .await
        .context("open store manager")?;
    let (listener, base_url) = router::bind_loopback_listener().await?;

    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));
    let daemon = TestDaemon::new(
        data_dir.path().to_path_buf(),
        stores.clone(),
        providers,
        base_url.clone(),
        Some("daemon-secret".to_string()),
    );
    daemon
        .upsert_provider_status(
            "fake".into(),
            FakeProviderAdapter::new()
                .inspect()
                .await
                .context("inspect fake provider")?,
        )
        .await;
    router::spawn_router(listener, daemon.handle());

    let workspace = create_workspace_record(&stores, repo.path()).await?;
    let store = stores
        .workspace(workspace.id)
        .await
        .context("open workspace store")?;
    let base_commit = git::run_git_output(repo.path(), &["rev-parse", "HEAD"]).await?;
    let worktree: Worktree = store
        .create_worktree(
            workspace.id,
            repo.path().to_string_lossy().to_string(),
            base_commit,
            None,
        )
        .await
        .context("create worktree")?;
    let task: Task = store
        .create_task(workspace.id, "task".into(), None)
        .await
        .context("create task")?;
    let session = store
        .create_session(
            task.id,
            workspace.id,
            worktree.id,
            ExecutionEnvironment::Host,
            "fake".into(),
            "fake-model".into(),
            "assistant".into(),
            None,
            None,
            None,
        )
        .await
        .context("create parent session")?;

    index_parent_entities(&daemon, &session, workspace.id, worktree.id, task.id).await?;
    let mcp_token = daemon
        .issue_provider_session_mcp_token(session.id, workspace.id, worktree.id)
        .await;

    Ok(DaemonBackedParentSession::new(
        repo, data_dir, daemon, base_url, session.id, mcp_token,
    ))
}

async fn create_workspace_record(
    stores: &StoreManager,
    repo_path: &std::path::Path,
) -> Result<Workspace> {
    stores
        .global()
        .create_workspace(
            "test".into(),
            repo_path.to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .context("create workspace")
}

async fn index_parent_entities(
    daemon: &TestDaemon,
    session: &Session,
    workspace_id: ctx_core::ids::WorkspaceId,
    worktree_id: ctx_core::ids::WorktreeId,
    task_id: ctx_core::ids::TaskId,
) -> Result<()> {
    daemon
        .global_store()
        .upsert_workspace_session_index(session.id, workspace_id)
        .await
        .context("index parent session")?;
    daemon
        .global_store()
        .upsert_workspace_worktree_index(worktree_id, workspace_id)
        .await
        .context("index parent worktree")?;
    daemon
        .global_store()
        .upsert_workspace_task_index(task_id, workspace_id)
        .await
        .context("index parent task")?;
    Ok(())
}
