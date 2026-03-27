use super::*;
use crate::daemon::AppState;
use ctx_core::models::{SandboxGuestIdentity, SandboxSubstrate, VcsKind};
use ctx_store::{Store, StoreManager};
use std::collections::HashMap;

fn git(args: &[&str], cwd: &StdPath) {
    let status = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .status()
        .expect("run git");
    assert!(status.success(), "git {:?} failed", args);
}

fn git_output(args: &[&str], cwd: &StdPath) -> String {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("run git output");
    assert!(output.status.success(), "git {:?} failed", args);
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn init_git_workspace(root: &StdPath) -> String {
    git(&["init", "-b", "main"], root);
    git(&["config", "user.email", "ctx@example.com"], root);
    git(&["config", "user.name", "Ctx Test"], root);
    std::fs::write(root.join("README.md"), "hello\n").expect("write readme");
    git(&["add", "README.md"], root);
    git(&["commit", "-m", "initial"], root);
    git_output(&["rev-parse", "HEAD"], root)
}

async fn test_state(data_root: &StdPath) -> Arc<AppState> {
    Arc::new(AppState::new(
        data_root.to_path_buf(),
        StoreManager::open(data_root).await.expect("open stores"),
        HashMap::new(),
        "http://127.0.0.1:4310".to_string(),
        None,
    ))
}

async fn save_test_execution_settings(
    state: &Arc<AppState>,
    execution: crate::settings::ExecutionSettings,
) {
    let settings = crate::settings::Settings {
        execution: Some(execution),
        ..Default::default()
    };
    crate::settings::save_settings(state.global_store(), &settings)
        .await
        .expect("save runtime settings");
}

struct EnvVarGuard {
    key: &'static str,
    prev: Option<String>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let prev = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, prev }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if let Some(prev) = self.prev.take() {
            std::env::set_var(self.key, prev);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

async fn insert_managed_worktree(
    store: &Store,
    data_root: &StdPath,
    workspace: &Workspace,
    owner_task_id: TaskId,
    repo_root: &StdPath,
    base_commit: &str,
) -> (Worktree, PathBuf) {
    let worktree_id = WorktreeId::new();
    let managed_root = managed_worktree_path(data_root, workspace.id, worktree_id);
    let branch_name = format!("ctx/{}/{}", owner_task_id.0, worktree_id.0);
    git(
        &[
            "worktree",
            "add",
            "-b",
            &branch_name,
            managed_root.to_string_lossy().as_ref(),
            base_commit,
        ],
        repo_root,
    );
    let worktree = store
        .insert_worktree(Worktree {
            id: worktree_id,
            workspace_id: workspace.id,
            root_path: managed_root.to_string_lossy().to_string(),
            base_commit_sha: base_commit.to_string(),
            git_branch: Some(branch_name),
            vcs_kind: Some(VcsKind::Git),
            base_revision: Some(base_commit.to_string()),
            vcs_ref: Some("".to_string()),
            created_at: Utc::now(),
            bootstrap_status: None,
            bootstrap_started_at: None,
            bootstrap_finished_at: None,
            bootstrap_exit_code: None,
            bootstrap_timeout_sec: None,
            bootstrap_error: None,
            bootstrap_log_path: None,
            bootstrap_log_truncated: None,
            bootstrap_command: None,
            bootstrap_script_path: None,
        })
        .await
        .expect("insert worktree");
    (worktree, managed_root)
}

#[tokio::test]
async fn archive_task_only_dematerializes_sandbox_state() {
    let _serial = crate::test_support::sandbox_cli_env_test_lock()
        .lock()
        .await;
    let temp = tempfile::tempdir().expect("tempdir");
    let repo_root = temp.path().join("repo");
    std::fs::create_dir_all(&repo_root).expect("create repo root");
    let base_commit = init_git_workspace(&repo_root);
    let state = test_state(temp.path()).await;
    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            repo_root.to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .expect("create workspace");
    let store = state
        .store_for_workspace(workspace.id)
        .await
        .expect("workspace store");
    let task = store
        .create_task(workspace.id, "task".to_string(), None)
        .await
        .expect("create task");
    state
        .global_store()
        .upsert_workspace_task_index(task.id, workspace.id)
        .await
        .expect("upsert task index");
    let (worktree, managed_root) = insert_managed_worktree(
        &store,
        temp.path(),
        &workspace,
        task.id,
        &repo_root,
        &base_commit,
    )
    .await;
    state
        .global_store()
        .upsert_workspace_worktree_index(worktree.id, workspace.id)
        .await
        .expect("upsert worktree index");
    store
        .set_task_primary_worktree(task.id, worktree.id)
        .await
        .expect("set primary worktree");

    let host_materialization_root = temp.path().join("host-shadow");
    std::fs::create_dir_all(&host_materialization_root).expect("create host shadow root");
    store
        .upsert_sandbox_binding(SandboxBinding {
            worktree_id: worktree.id,
            workspace_id: workspace.id,
            sandbox_instance_id: ctx_core::models::sandbox_instance_id_for_workspace(workspace.id),
            substrate: SandboxSubstrate::SharedVmContainer,
            guest_identity: SandboxGuestIdentity::linux_container_ubuntu(),
            profile: SandboxProfile::Standard,
            live_workspace_root: crate::harness_runtime::CTX_CONTAINER_WORKSPACE_ROOT.to_string(),
            live_worktree_root: crate::disk_isolated::container_worktree_root(worktree.id)
                .to_string_lossy()
                .to_string(),
            execution_settings_json: None,
            container_name: Some(crate::harness_runtime::workspace_container_name(
                workspace.id,
            )),
            host_materialization_root: Some(
                host_materialization_root.to_string_lossy().to_string(),
            ),
            created_at: Utc::now(),
        })
        .await
        .expect("insert sandbox binding");

    let log_path = temp.path().join("sandbox-cli.log");
    let sandbox_cli_path = crate::test_support::write_running_container_sandbox_cli_shim(
        temp.path(),
        &log_path,
        &crate::harness_runtime::workspace_container_name(workspace.id),
    );
    let _sandbox_cli = EnvVarGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli_path.to_string_lossy(),
    );
    let _sandbox_cli_available = EnvVarGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "1");

    let Json(_) = archive_task(State(Arc::clone(&state)), Path(task.id.0.to_string()))
        .await
        .expect("archive task");
    let archived_task = store
        .get_task(task.id)
        .await
        .expect("load archived task")
        .expect("archived task exists");
    assert!(
        archived_task.archived_at.is_some(),
        "task should be archived after archive_task"
    );
    assert!(
        tokio::fs::metadata(&managed_root).await.is_ok(),
        "archive should preserve the canonical managed worktree root"
    );
    assert!(
        branch_exists(
            &repo_root,
            worktree.git_branch.as_deref().expect("branch name"),
        )
        .await
        .expect("check branch"),
        "archive should preserve the task worktree branch"
    );
    assert!(
        store
            .get_worktree(worktree.id)
            .await
            .expect("load worktree")
            .is_some(),
        "archive should preserve the worktree row"
    );
    assert!(
        store
            .get_sandbox_binding(worktree.id)
            .await
            .expect("load binding")
            .is_some(),
        "archive should preserve the sandbox binding row for rematerialization"
    );
    assert!(
        tokio::fs::metadata(&host_materialization_root)
            .await
            .is_err(),
        "archive should remove the sandbox host materialization root"
    );
}

#[tokio::test]
async fn unarchive_task_recreates_managed_root_and_keeps_binding_snapshot_runtime() {
    let _serial = crate::test_support::sandbox_cli_env_test_lock()
        .lock()
        .await;
    let temp = tempfile::tempdir().expect("tempdir");
    let repo_root = temp.path().join("repo");
    std::fs::create_dir_all(&repo_root).expect("create repo root");
    let base_commit = init_git_workspace(&repo_root);
    let state = test_state(temp.path()).await;
    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            repo_root.to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .expect("create workspace");
    let store = state
        .store_for_workspace(workspace.id)
        .await
        .expect("workspace store");
    let task = store
        .create_task(workspace.id, "task".to_string(), None)
        .await
        .expect("create task");
    state
        .global_store()
        .upsert_workspace_task_index(task.id, workspace.id)
        .await
        .expect("upsert task index");
    let (worktree, managed_root) = insert_managed_worktree(
        &store,
        temp.path(),
        &workspace,
        task.id,
        &repo_root,
        &base_commit,
    )
    .await;
    state
        .global_store()
        .upsert_workspace_worktree_index(worktree.id, workspace.id)
        .await
        .expect("upsert worktree index");
    store
        .set_task_primary_worktree(task.id, worktree.id)
        .await
        .expect("set primary worktree");

    let persisted_snapshot = crate::settings::ExecutionSettings {
        mode: crate::settings::ExecutionMode::Sandbox,
        container: crate::settings::ContainerExecutionSettings {
            runtime: crate::settings::ContainerRuntimeKind::NativeContainer,
            network_mode: crate::settings::ContainerNetworkMode::Allowlist,
            allowlist: vec!["github.com".to_string()],
            image: Some("registry.example/sandbox:v1".to_string()),
            ..crate::settings::ContainerExecutionSettings::default()
        },
    };
    store
        .upsert_sandbox_binding(SandboxBinding {
            worktree_id: worktree.id,
            workspace_id: workspace.id,
            sandbox_instance_id: ctx_core::models::sandbox_instance_id_for_workspace(workspace.id),
            substrate: SandboxSubstrate::NativeContainer,
            guest_identity: SandboxGuestIdentity::linux_container_ubuntu(),
            profile: SandboxProfile::Standard,
            live_workspace_root: crate::harness_runtime::CTX_CONTAINER_WORKSPACE_ROOT.to_string(),
            live_worktree_root: crate::disk_isolated::container_worktree_root(worktree.id)
                .to_string_lossy()
                .to_string(),
            execution_settings_json: Some(
                serde_json::to_string(&persisted_snapshot).expect("serialize binding snapshot"),
            ),
            container_name: Some(crate::harness_runtime::workspace_container_name(
                workspace.id,
            )),
            host_materialization_root: None,
            created_at: Utc::now(),
        })
        .await
        .expect("insert sandbox binding");

    let log_path = temp.path().join("sandbox-cli.log");
    let sandbox_cli_path = crate::test_support::write_running_container_sandbox_cli_shim(
        temp.path(),
        &log_path,
        &crate::harness_runtime::workspace_container_name(workspace.id),
    );
    let _sandbox_cli = EnvVarGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli_path.to_string_lossy(),
    );
    let _sandbox_cli_available = EnvVarGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "1");

    let Json(_) = archive_task(State(Arc::clone(&state)), Path(task.id.0.to_string()))
        .await
        .expect("archive task");
    tokio::fs::remove_dir_all(&managed_root)
        .await
        .expect("remove managed root after archive");

    save_test_execution_settings(
        &state,
        crate::settings::ExecutionSettings {
            mode: crate::settings::ExecutionMode::Sandbox,
            container: crate::settings::ContainerExecutionSettings {
                runtime: crate::settings::ContainerRuntimeKind::SharedVmContainer,
                network_mode: crate::settings::ContainerNetworkMode::All,
                allowlist: Vec::new(),
                image: Some("registry.example/sandbox:v2".to_string()),
                ..crate::settings::ContainerExecutionSettings::default()
            },
        },
    )
    .await;

    let Json(unarchived_task) =
        unarchive_task(State(Arc::clone(&state)), Path(task.id.0.to_string()))
            .await
            .expect("unarchive task");

    assert!(
        unarchived_task.archived_at.is_none(),
        "task should no longer be archived after unarchive_task"
    );
    assert!(
        tokio::fs::metadata(&managed_root).await.is_ok(),
        "unarchive should recreate the canonical managed worktree root"
    );
    assert!(
        branch_exists(
            &repo_root,
            worktree.git_branch.as_deref().expect("branch name"),
        )
        .await
        .expect("check branch"),
        "unarchive should keep the existing managed worktree branch attached"
    );

    let current_effective =
        crate::execution_effective::effective_execution_settings(&state, workspace.id)
            .await
            .expect("load current effective settings");
    assert_eq!(
        current_effective.container.runtime,
        crate::settings::ContainerRuntimeKind::SharedVmContainer,
        "workspace defaults should now point at the new runtime"
    );

    let binding = store
        .get_sandbox_binding(worktree.id)
        .await
        .expect("load rematerialized binding")
        .expect("binding should remain present after unarchive");
    assert_eq!(binding.substrate, SandboxSubstrate::NativeContainer);
    assert_eq!(
        binding.sandbox_instance_id,
        ctx_core::models::sandbox_instance_id_for_workspace(workspace.id)
    );
    let parsed = crate::api::tasks::sandbox_execution_settings_from_binding(&binding)
        .expect("parse rematerialized binding snapshot");
    assert_eq!(
        parsed.container.runtime,
        crate::settings::ContainerRuntimeKind::NativeContainer,
        "rematerialized binding must preserve the original runtime snapshot"
    );
    assert_eq!(
        parsed.container.network_mode,
        crate::settings::ContainerNetworkMode::Allowlist
    );
    assert_eq!(parsed.container.allowlist, vec!["github.com".to_string()]);
    assert_eq!(
        parsed.container.image,
        Some("registry.example/sandbox:v1".to_string())
    );
}

#[tokio::test]
async fn unarchive_task_fails_closed_for_corrupt_binding_snapshot() {
    let _serial = crate::test_support::sandbox_cli_env_test_lock()
        .lock()
        .await;
    let temp = tempfile::tempdir().expect("tempdir");
    let repo_root = temp.path().join("repo");
    std::fs::create_dir_all(&repo_root).expect("create repo root");
    let base_commit = init_git_workspace(&repo_root);
    let state = test_state(temp.path()).await;
    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            repo_root.to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .expect("create workspace");
    let store = state
        .store_for_workspace(workspace.id)
        .await
        .expect("workspace store");
    let task = store
        .create_task(workspace.id, "task".to_string(), None)
        .await
        .expect("create task");
    state
        .global_store()
        .upsert_workspace_task_index(task.id, workspace.id)
        .await
        .expect("upsert task index");
    let (worktree, managed_root) = insert_managed_worktree(
        &store,
        temp.path(),
        &workspace,
        task.id,
        &repo_root,
        &base_commit,
    )
    .await;
    state
        .global_store()
        .upsert_workspace_worktree_index(worktree.id, workspace.id)
        .await
        .expect("upsert worktree index");
    store
        .set_task_primary_worktree(task.id, worktree.id)
        .await
        .expect("set primary worktree");
    store
        .upsert_sandbox_binding(SandboxBinding {
            worktree_id: worktree.id,
            workspace_id: workspace.id,
            sandbox_instance_id: ctx_core::models::sandbox_instance_id_for_workspace(workspace.id),
            substrate: SandboxSubstrate::NativeContainer,
            guest_identity: SandboxGuestIdentity::linux_container_ubuntu(),
            profile: SandboxProfile::Standard,
            live_workspace_root: crate::harness_runtime::CTX_CONTAINER_WORKSPACE_ROOT.to_string(),
            live_worktree_root: crate::disk_isolated::container_worktree_root(worktree.id)
                .to_string_lossy()
                .to_string(),
            execution_settings_json: Some(
                serde_json::json!({
                    "mode": "host",
                    "container": {
                        "runtime": "native_container",
                        "mount_mode": "disk_isolated",
                        "network_mode": "all",
                        "allowlist": [],
                        "image": null
                    }
                })
                .to_string(),
            ),
            container_name: Some(crate::harness_runtime::workspace_container_name(
                workspace.id,
            )),
            host_materialization_root: None,
            created_at: Utc::now(),
        })
        .await
        .expect("insert corrupt sandbox binding");

    let log_path = temp.path().join("sandbox-cli.log");
    let sandbox_cli_path = crate::test_support::write_running_container_sandbox_cli_shim(
        temp.path(),
        &log_path,
        &crate::harness_runtime::workspace_container_name(workspace.id),
    );
    let _sandbox_cli = EnvVarGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli_path.to_string_lossy(),
    );
    let _sandbox_cli_available = EnvVarGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "1");

    let Json(_) = archive_task(State(Arc::clone(&state)), Path(task.id.0.to_string()))
        .await
        .expect("archive task");

    let status = unarchive_task(State(Arc::clone(&state)), Path(task.id.0.to_string()))
        .await
        .expect_err("corrupt binding snapshot should fail closed");
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        store
            .get_task(task.id)
            .await
            .expect("load task after failed unarchive")
            .expect("task should still exist")
            .archived_at
            .is_some(),
        "failed unarchive should leave the task archived"
    );
    assert!(
        tokio::fs::metadata(&managed_root).await.is_ok(),
        "failed unarchive should not destroy the canonical managed worktree root"
    );
    assert!(
        store
            .get_sandbox_binding(worktree.id)
            .await
            .expect("load binding after failed unarchive")
            .is_some(),
        "failed unarchive should preserve the persisted binding row for repair"
    );
}

#[tokio::test]
async fn delete_task_preserves_worktree_for_archived_sibling_session_reference() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo_root = temp.path().join("repo");
    std::fs::create_dir_all(&repo_root).expect("create repo root");
    let base_commit = init_git_workspace(&repo_root);
    let state = test_state(temp.path()).await;
    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            repo_root.to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .expect("create workspace");
    let store = state
        .store_for_workspace(workspace.id)
        .await
        .expect("workspace store");
    let active_task = store
        .create_task(workspace.id, "active".to_string(), None)
        .await
        .expect("create active task");
    let archived_task = store
        .create_task(workspace.id, "archived".to_string(), None)
        .await
        .expect("create archived task");
    state
        .global_store()
        .upsert_workspace_task_index(active_task.id, workspace.id)
        .await
        .expect("upsert active task index");
    state
        .global_store()
        .upsert_workspace_task_index(archived_task.id, workspace.id)
        .await
        .expect("upsert archived task index");
    let (worktree, managed_root) = insert_managed_worktree(
        &store,
        temp.path(),
        &workspace,
        active_task.id,
        &repo_root,
        &base_commit,
    )
    .await;
    state
        .global_store()
        .upsert_workspace_worktree_index(worktree.id, workspace.id)
        .await
        .expect("upsert worktree index");
    store
        .set_task_primary_worktree(active_task.id, worktree.id)
        .await
        .expect("set active primary worktree");
    store
        .create_session(
            archived_task.id,
            workspace.id,
            worktree.id,
            ExecutionEnvironment::Sandbox,
            "fake".to_string(),
            "model".to_string(),
            "archived".to_string(),
            None,
            None,
            None,
        )
        .await
        .expect("create archived task session");
    store
        .archive_task(archived_task.id)
        .await
        .expect("archive sibling task");

    let status = delete_task(
        State(Arc::clone(&state)),
        Path(active_task.id.0.to_string()),
    )
    .await
    .expect("delete active task");
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(
        store
            .get_task(archived_task.id)
            .await
            .expect("load archived sibling")
            .expect("archived sibling exists")
            .archived_at
            .is_some(),
        "archived sibling should remain archived after deleting the active task"
    );
    assert!(
        store
            .get_worktree(worktree.id)
            .await
            .expect("load worktree")
            .is_some(),
        "delete should preserve the worktree row while an archived sibling still references it"
    );
    assert!(
        state
            .global_store()
            .get_workspace_id_for_worktree(worktree.id)
            .await
            .expect("load worktree index")
            .is_some(),
        "delete should preserve the worktree index while an archived sibling still references it"
    );
    assert!(
        tokio::fs::metadata(&managed_root).await.is_ok(),
        "delete should preserve the managed worktree root for archived sibling history"
    );
    assert!(
        branch_exists(
            &repo_root,
            worktree.git_branch.as_deref().expect("branch name"),
        )
        .await
        .expect("check branch"),
        "delete should preserve the branch while an archived sibling still references the worktree"
    );
}

#[tokio::test]
async fn delete_task_cleanup_errors_preserve_worktree_row_and_index() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo_root = temp.path().join("repo");
    std::fs::create_dir_all(&repo_root).expect("create repo root");
    let base_commit = init_git_workspace(&repo_root);
    let state = test_state(temp.path()).await;
    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            repo_root.to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .expect("create workspace");
    let store = state
        .store_for_workspace(workspace.id)
        .await
        .expect("workspace store");
    let task = store
        .create_task(workspace.id, "task".to_string(), None)
        .await
        .expect("create task");
    state
        .global_store()
        .upsert_workspace_task_index(task.id, workspace.id)
        .await
        .expect("upsert task index");
    let worktree_id = WorktreeId::new();
    let managed_root = managed_worktree_path(temp.path(), workspace.id, worktree_id);
    std::fs::create_dir_all(
        managed_root
            .parent()
            .expect("managed worktree parent exists"),
    )
    .expect("create managed worktree parent");
    std::fs::write(&managed_root, "not-a-directory").expect("create managed worktree file");
    let worktree = store
        .insert_worktree(Worktree {
            id: worktree_id,
            workspace_id: workspace.id,
            root_path: managed_root.to_string_lossy().to_string(),
            base_commit_sha: base_commit.clone(),
            git_branch: Some(format!("ctx/{}/{}", task.id.0, worktree_id.0)),
            vcs_kind: Some(VcsKind::Git),
            base_revision: Some(base_commit),
            vcs_ref: Some("".to_string()),
            created_at: Utc::now(),
            bootstrap_status: None,
            bootstrap_started_at: None,
            bootstrap_finished_at: None,
            bootstrap_exit_code: None,
            bootstrap_timeout_sec: None,
            bootstrap_error: None,
            bootstrap_log_path: None,
            bootstrap_log_truncated: None,
            bootstrap_command: None,
            bootstrap_script_path: None,
        })
        .await
        .expect("insert worktree");
    state
        .global_store()
        .upsert_workspace_worktree_index(worktree.id, workspace.id)
        .await
        .expect("upsert worktree index");
    store
        .set_task_primary_worktree(task.id, worktree.id)
        .await
        .expect("set primary worktree");

    let status = delete_task(State(Arc::clone(&state)), Path(task.id.0.to_string()))
        .await
        .expect("delete task");
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(
        store
            .get_worktree(worktree.id)
            .await
            .expect("load worktree")
            .is_some(),
        "delete cleanup errors must not drop the worktree row"
    );
    assert!(
        state
            .global_store()
            .get_workspace_id_for_worktree(worktree.id)
            .await
            .expect("load worktree index")
            .is_some(),
        "delete cleanup errors must not drop the worktree index"
    );
    assert!(
        tokio::fs::metadata(&managed_root).await.is_ok(),
        "failed cleanup should leave the managed worktree root in place"
    );
}

#[tokio::test]
async fn delete_task_removes_unused_worktree_rows_and_indexes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo_root = temp.path().join("repo");
    std::fs::create_dir_all(&repo_root).expect("create repo root");
    let base_commit = init_git_workspace(&repo_root);
    let state = test_state(temp.path()).await;
    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            repo_root.to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .expect("create workspace");
    let store = state
        .store_for_workspace(workspace.id)
        .await
        .expect("workspace store");
    let task = store
        .create_task(workspace.id, "task".to_string(), None)
        .await
        .expect("create task");
    state
        .global_store()
        .upsert_workspace_task_index(task.id, workspace.id)
        .await
        .expect("upsert task index");

    let worktree_id = WorktreeId::new();
    let managed_root = managed_worktree_path(temp.path(), workspace.id, worktree_id);
    let branch_name = format!("ctx/{}/{}", task.id.0, worktree_id.0);
    git(
        &[
            "worktree",
            "add",
            "-b",
            &branch_name,
            managed_root.to_string_lossy().as_ref(),
            &base_commit,
        ],
        &repo_root,
    );
    let worktree = store
        .insert_worktree(Worktree {
            id: worktree_id,
            workspace_id: workspace.id,
            root_path: managed_root.to_string_lossy().to_string(),
            base_commit_sha: base_commit.clone(),
            git_branch: Some(branch_name),
            vcs_kind: Some(VcsKind::Git),
            base_revision: Some(base_commit.clone()),
            vcs_ref: Some("".to_string()),
            created_at: Utc::now(),
            bootstrap_status: None,
            bootstrap_started_at: None,
            bootstrap_finished_at: None,
            bootstrap_exit_code: None,
            bootstrap_timeout_sec: None,
            bootstrap_error: None,
            bootstrap_log_path: None,
            bootstrap_log_truncated: None,
            bootstrap_command: None,
            bootstrap_script_path: None,
        })
        .await
        .expect("insert worktree");
    state
        .global_store()
        .upsert_workspace_worktree_index(worktree.id, workspace.id)
        .await
        .expect("upsert worktree index");
    store
        .set_task_primary_worktree(task.id, worktree.id)
        .await
        .expect("set primary worktree");

    let status = delete_task(State(Arc::clone(&state)), Path(task.id.0.to_string()))
        .await
        .expect("delete task");
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(
        store
            .get_worktree(worktree.id)
            .await
            .expect("load worktree")
            .is_none(),
        "unused worktree row should be removed on task delete"
    );
    assert!(
        state
            .global_store()
            .get_workspace_id_for_worktree(worktree.id)
            .await
            .expect("load worktree index")
            .is_none(),
        "unused worktree index should be removed on task delete"
    );
    assert!(
        tokio::fs::metadata(&managed_root).await.is_err(),
        "managed worktree root should be removed on task delete"
    );
}

#[tokio::test]
async fn delete_task_removes_standalone_managed_worktree_when_workspace_root_is_missing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo_root = temp.path().join("repo");
    std::fs::create_dir_all(&repo_root).expect("create repo root");
    let base_commit = init_git_workspace(&repo_root);
    let state = test_state(temp.path()).await;
    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            repo_root.to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .expect("create workspace");
    let store = state
        .store_for_workspace(workspace.id)
        .await
        .expect("workspace store");
    let task = store
        .create_task(workspace.id, "task".to_string(), None)
        .await
        .expect("create task");
    state
        .global_store()
        .upsert_workspace_task_index(task.id, workspace.id)
        .await
        .expect("upsert task index");

    let (worktree, managed_root) = insert_managed_worktree(
        &store,
        temp.path(),
        &workspace,
        task.id,
        &repo_root,
        &base_commit,
    )
    .await;
    ctx_fs::worktrees::standaloneize_worktree_git_dir(&managed_root)
        .await
        .expect("standaloneize managed worktree");
    tokio::fs::remove_dir_all(&repo_root)
        .await
        .expect("remove source workspace root");

    state
        .global_store()
        .upsert_workspace_worktree_index(worktree.id, workspace.id)
        .await
        .expect("upsert worktree index");
    store
        .set_task_primary_worktree(task.id, worktree.id)
        .await
        .expect("set primary worktree");

    let status = delete_task(State(Arc::clone(&state)), Path(task.id.0.to_string()))
        .await
        .expect("delete task");
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(
        store
            .get_worktree(worktree.id)
            .await
            .expect("load worktree")
            .is_none(),
        "unused worktree row should be removed on task delete"
    );
    assert!(
        state
            .global_store()
            .get_workspace_id_for_worktree(worktree.id)
            .await
            .expect("load worktree index")
            .is_none(),
        "unused worktree index should be removed on task delete"
    );
    assert!(
        tokio::fs::metadata(&managed_root).await.is_err(),
        "standalone managed worktree root should be removed on task delete"
    );
}
