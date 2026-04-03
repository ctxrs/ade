use anyhow::{bail, Context, Result};
use ctx_core::ids::{TaskId, WorkspaceId, WorktreeId};
use ctx_core::models::{VcsKind, Workspace, Worktree};
use std::path::{Path, PathBuf};
use tokio::process::Command;

use crate::execution_effective;
use crate::harness_runtime::{sandbox_container_command, workspace_container_name};
use crate::settings::{ContainerRuntimeKind, ExecutionMode};
use crate::worktree_data_plane::{
    apply_data_plane_to_execution_settings, resolve_worktree_data_plane,
};

const COMMIT_MSG_HOOK: &str = r#"#!/bin/sh
set -e

msg_file="$1"
if [ -z "$msg_file" ] || [ ! -f "$msg_file" ]; then
  exit 0
fi

if grep -q "Task-Id:" "$msg_file"; then
  exit 0
fi

task_id="$(git config --worktree --get ctx.taskId || true)"
if [ -z "$task_id" ]; then
  echo "ctx: missing ctx.taskId for commit; set ctx.taskId or create a task." >&2
  exit 1
fi

printf '\nTask-Id: %s\n' "$task_id" >> "$msg_file"
"#;

const CORE_HOOKS_PATH_KEY: &str = "core.hooksPath";
const CTX_TASK_ID_KEY: &str = "ctx.taskId";
const CTX_PREV_HOOKS_PATH_KEY: &str = "ctx.prevHooksPath";

pub fn vcs_hooks_root(data_root: &Path) -> PathBuf {
    data_root.join("vcs-hooks")
}

pub fn worktree_hooks_dir(
    data_root: &Path,
    workspace_id: WorkspaceId,
    worktree_id: WorktreeId,
) -> PathBuf {
    vcs_hooks_root(data_root)
        .join(workspace_id.0.to_string())
        .join(worktree_id.0.to_string())
}

async fn install_commit_hook_script(hooks_dir: &Path) -> Result<PathBuf> {
    tokio::fs::create_dir_all(hooks_dir)
        .await
        .context("creating vcs hooks dir")?;

    let hook_path = hooks_dir.join("commit-msg");
    tokio::fs::write(&hook_path, COMMIT_MSG_HOOK)
        .await
        .context("writing commit-msg hook")?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut perms = tokio::fs::metadata(&hook_path)
            .await
            .context("loading commit-msg hook metadata")?
            .permissions();
        perms.set_mode(0o755);
        tokio::fs::set_permissions(&hook_path, perms)
            .await
            .context("setting commit-msg hook permissions")?;
    }

    Ok(hook_path)
}

async fn configure_task_commit_hook_host(
    worktree_root: &Path,
    hooks_path: &str,
    task_id: TaskId,
) -> Result<()> {
    if let Some(existing) = get_git_config(worktree_root, CORE_HOOKS_PATH_KEY).await? {
        if existing != hooks_path {
            set_git_config(worktree_root, CTX_PREV_HOOKS_PATH_KEY, &existing).await?;
        }
    }
    set_git_config(worktree_root, CORE_HOOKS_PATH_KEY, hooks_path).await?;
    set_git_config(worktree_root, CTX_TASK_ID_KEY, &task_id.0.to_string()).await?;
    Ok(())
}

async fn configure_task_commit_hook_sandbox(
    state: &crate::daemon::AppState,
    workspace: &Workspace,
    worktree: &Worktree,
    settings: &crate::settings::ExecutionSettings,
    hooks_path: &str,
    task_id: TaskId,
) -> Result<()> {
    if let Some(existing) =
        sandbox_git_config_get(state, workspace, worktree, settings, CORE_HOOKS_PATH_KEY).await?
    {
        if existing != hooks_path {
            sandbox_git_config_set(
                state,
                workspace,
                worktree,
                settings,
                CTX_PREV_HOOKS_PATH_KEY,
                &existing,
            )
            .await?;
        }
    }
    sandbox_git_config_set(
        state,
        workspace,
        worktree,
        settings,
        CORE_HOOKS_PATH_KEY,
        hooks_path,
    )
    .await?;
    sandbox_git_config_set(
        state,
        workspace,
        worktree,
        settings,
        CTX_TASK_ID_KEY,
        &task_id.0.to_string(),
    )
    .await?;
    Ok(())
}

async fn ensure_task_commit_hook_host(
    data_root: &Path,
    workspace_id: WorkspaceId,
    worktree_id: WorktreeId,
    worktree_root: &Path,
    vcs_kind: Option<VcsKind>,
    task_id: TaskId,
) -> Result<()> {
    if vcs_kind != Some(VcsKind::Git) {
        return Ok(());
    }
    if tokio::fs::metadata(worktree_root).await.is_err() {
        return Ok(());
    }

    let hooks_dir = worktree_hooks_dir(data_root, workspace_id, worktree_id);
    install_commit_hook_script(&hooks_dir).await?;
    let hooks_path = hooks_dir.to_string_lossy().to_string();
    configure_task_commit_hook_host(worktree_root, &hooks_path, task_id).await
}

async fn ensure_task_commit_hook_sandbox(
    state: &crate::daemon::AppState,
    workspace: &Workspace,
    worktree: &Worktree,
    settings: &crate::settings::ExecutionSettings,
    task_id: TaskId,
) -> Result<()> {
    let hooks_dir = worktree_hooks_dir(&state.core.data_root, workspace.id, worktree.id);
    install_commit_hook_script(&hooks_dir).await?;
    let hooks_path = hooks_dir.to_string_lossy().to_string();
    configure_task_commit_hook_sandbox(state, workspace, worktree, settings, &hooks_path, task_id)
        .await
}

pub async fn ensure_task_commit_hook(
    state: &crate::daemon::AppState,
    workspace: &Workspace,
    worktree: &Worktree,
    task_id: TaskId,
) -> Result<()> {
    let data_plane = resolve_worktree_data_plane(state, worktree).await?;
    let settings = execution_effective::effective_execution_settings(state, workspace.id).await?;
    let settings = apply_data_plane_to_execution_settings(&settings, &data_plane)?;
    if matches!(settings.mode, ExecutionMode::Host) {
        return ensure_task_commit_hook_host(
            &state.core.data_root,
            workspace.id,
            worktree.id,
            Path::new(&worktree.root_path),
            worktree.vcs_kind.clone(),
            task_id,
        )
        .await;
    }
    if worktree.vcs_kind != Some(VcsKind::Git) {
        return Ok(());
    }

    state
        .execution
        .harness
        .ensure_workspace_container_for_worktree(
            workspace,
            worktree,
            &settings,
            &state.core.daemon_url,
        )
        .await?;

    let _ = data_plane;
    ensure_task_commit_hook_sandbox(state, workspace, worktree, &settings, task_id).await
}

pub async fn cleanup_worktree_hooks(
    state: &crate::daemon::AppState,
    workspace: &Workspace,
    worktree: &Worktree,
) -> Result<()> {
    let hooks_dir = worktree_hooks_dir(&state.core.data_root, workspace.id, worktree.id);
    let hooks_path = hooks_dir.to_string_lossy().to_string();
    if worktree.vcs_kind == Some(VcsKind::Git) {
        let data_plane = resolve_worktree_data_plane(state, worktree).await?;
        let settings =
            execution_effective::effective_execution_settings(state, workspace.id).await?;
        let settings = apply_data_plane_to_execution_settings(&settings, &data_plane)?;
        if matches!(settings.mode, ExecutionMode::Host) {
            let root = Path::new(&worktree.root_path);
            if tokio::fs::metadata(root).await.is_ok() {
                cleanup_git_config_host(root, &hooks_path).await?;
            }
        } else {
            cleanup_git_config_sandbox(state, workspace, worktree, &settings, &hooks_path).await?;
        }
    }
    if tokio::fs::metadata(&hooks_dir).await.is_ok() {
        tokio::fs::remove_dir_all(&hooks_dir)
            .await
            .context("removing vcs hooks dir")?;
    }
    Ok(())
}

pub async fn cleanup_workspace_hooks(data_root: &Path, workspace_id: WorkspaceId) -> Result<()> {
    let hooks_dir = vcs_hooks_root(data_root).join(workspace_id.0.to_string());
    if tokio::fs::metadata(&hooks_dir).await.is_ok() {
        tokio::fs::remove_dir_all(&hooks_dir)
            .await
            .context("removing workspace vcs hooks dir")?;
    }
    Ok(())
}

async fn set_git_config(worktree_root: &Path, key: &str, value: &str) -> Result<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(worktree_root)
        .arg("config")
        .arg("--worktree")
        .arg(key)
        .arg(value)
        .output()
        .await
        .context("running git config")?;
    if !output.status.success() {
        bail!(
            "git config --worktree failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

async fn cleanup_git_config_host(worktree_root: &Path, hooks_path: &str) -> Result<()> {
    if let Some(current) = get_git_config(worktree_root, CORE_HOOKS_PATH_KEY).await? {
        if current == hooks_path {
            if let Some(prev) = get_git_config(worktree_root, CTX_PREV_HOOKS_PATH_KEY).await? {
                set_git_config(worktree_root, CORE_HOOKS_PATH_KEY, &prev).await?;
            } else {
                unset_git_config(worktree_root, CORE_HOOKS_PATH_KEY).await?;
            }
            unset_git_config(worktree_root, CTX_TASK_ID_KEY).await?;
            unset_git_config(worktree_root, CTX_PREV_HOOKS_PATH_KEY).await?;
        }
    }
    Ok(())
}

async fn cleanup_git_config_sandbox(
    state: &crate::daemon::AppState,
    workspace: &Workspace,
    worktree: &Worktree,
    settings: &crate::settings::ExecutionSettings,
    hooks_path: &str,
) -> Result<()> {
    if let Some(current) =
        sandbox_git_config_get(state, workspace, worktree, settings, CORE_HOOKS_PATH_KEY).await?
    {
        if current == hooks_path {
            if let Some(prev) = sandbox_git_config_get(
                state,
                workspace,
                worktree,
                settings,
                CTX_PREV_HOOKS_PATH_KEY,
            )
            .await?
            {
                sandbox_git_config_set(
                    state,
                    workspace,
                    worktree,
                    settings,
                    CORE_HOOKS_PATH_KEY,
                    &prev,
                )
                .await?;
            } else {
                sandbox_git_config_unset(state, workspace, worktree, settings, CORE_HOOKS_PATH_KEY)
                    .await?;
            }
            sandbox_git_config_unset(state, workspace, worktree, settings, CTX_TASK_ID_KEY).await?;
            sandbox_git_config_unset(
                state,
                workspace,
                worktree,
                settings,
                CTX_PREV_HOOKS_PATH_KEY,
            )
            .await?;
        }
    }
    Ok(())
}

async fn sandbox_command(
    state: &crate::daemon::AppState,
    workspace: &Workspace,
    worktree: &Worktree,
    settings: &crate::settings::ExecutionSettings,
    command: &str,
    args: &[String],
) -> Result<Command> {
    let data_plane = resolve_worktree_data_plane(state, worktree).await?;
    let live_worktree_root = data_plane.live_worktree_root;
    match settings.container.runtime {
        ContainerRuntimeKind::NativeContainer => {
            let mut cmd = sandbox_container_command(&state.core.data_root)?;
            cmd.arg("exec")
                .arg("--interactive")
                .arg("--workdir")
                .arg(&live_worktree_root)
                .arg(workspace_container_name(workspace.id))
                .arg(command);
            cmd.args(args);
            Ok(cmd)
        }
        ContainerRuntimeKind::SharedVmContainer => {
            crate::workspace_runtime::build_avf_linux_guest_exec_command(
                &state.core.data_root,
                workspace.id,
                worktree.id,
                &live_worktree_root,
                command,
                args,
                &std::collections::HashMap::new(),
                None,
                false,
            )
        }
    }
}

async fn sandbox_git_config_get(
    state: &crate::daemon::AppState,
    workspace: &Workspace,
    worktree: &Worktree,
    settings: &crate::settings::ExecutionSettings,
    key: &str,
) -> Result<Option<String>> {
    let mut cmd = sandbox_command(
        state,
        workspace,
        worktree,
        settings,
        "git",
        &[
            "config".to_string(),
            "--worktree".to_string(),
            "--get".to_string(),
            key.to_string(),
        ],
    )
    .await?;
    let output = cmd
        .output()
        .await
        .context("running sandbox git config --get")?;
    if output.status.success() {
        return Ok(Some(
            String::from_utf8_lossy(&output.stdout).trim().to_string(),
        ));
    }
    if output.status.code() == Some(1) {
        return Ok(None);
    }
    bail!(
        "sandbox git config --get failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    )
}

async fn sandbox_git_config_set(
    state: &crate::daemon::AppState,
    workspace: &Workspace,
    worktree: &Worktree,
    settings: &crate::settings::ExecutionSettings,
    key: &str,
    value: &str,
) -> Result<()> {
    let mut cmd = sandbox_command(
        state,
        workspace,
        worktree,
        settings,
        "git",
        &[
            "config".to_string(),
            "--worktree".to_string(),
            key.to_string(),
            value.to_string(),
        ],
    )
    .await?;
    let output = cmd.output().await.context("running sandbox git config")?;
    if !output.status.success() {
        bail!(
            "sandbox git config --worktree failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

async fn sandbox_git_config_unset(
    state: &crate::daemon::AppState,
    workspace: &Workspace,
    worktree: &Worktree,
    settings: &crate::settings::ExecutionSettings,
    key: &str,
) -> Result<()> {
    let mut cmd = sandbox_command(
        state,
        workspace,
        worktree,
        settings,
        "git",
        &[
            "config".to_string(),
            "--worktree".to_string(),
            "--unset-all".to_string(),
            key.to_string(),
        ],
    )
    .await?;
    let output = cmd
        .output()
        .await
        .context("running sandbox git config --unset-all")?;
    if output.status.success() || matches!(output.status.code(), Some(1)) {
        return Ok(());
    }
    bail!(
        "sandbox git config --unset-all failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    )
}

async fn get_git_config(worktree_root: &Path, key: &str) -> Result<Option<String>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(worktree_root)
        .arg("config")
        .arg("--worktree")
        .arg("--get")
        .arg(key)
        .output()
        .await
        .context("running git config --get")?;
    if output.status.success() {
        let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if value.is_empty() {
            return Ok(None);
        }
        return Ok(Some(value));
    }
    if matches!(output.status.code(), Some(1)) {
        return Ok(None);
    }
    bail!(
        "git config --get failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn unset_git_config(worktree_root: &Path, key: &str) -> Result<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(worktree_root)
        .arg("config")
        .arg("--worktree")
        .arg("--unset-all")
        .arg(key)
        .output()
        .await
        .context("running git config --unset-all")?;
    if output.status.success() || matches!(output.status.code(), Some(1)) {
        return Ok(());
    }
    bail!(
        "git config --unset-all failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::AppState;
    use crate::settings::{
        save_settings, ContainerExecutionSettings, ExecutionMode, ExecutionSettings, Settings,
    };
    use chrono::Utc;
    use ctx_core::ids::TaskId;
    use ctx_core::models::{
        sandbox_instance_id_for_workspace, SandboxBinding, SandboxGuestIdentity, SandboxProfile,
        SandboxSubstrate,
    };
    use ctx_store::StoreManager;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Arc;

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

    fn git(args: &[&str], cwd: &Path) {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .status()
            .expect("run git");
        assert!(status.success(), "git {:?} failed", args);
    }

    fn git_output(args: &[&str], cwd: &Path) -> String {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("run git");
        assert!(output.status.success(), "git {:?} failed", args);
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn init_git_workspace(root: &Path) -> String {
        git(&["init", "-b", "main"], root);
        git(&["config", "extensions.worktreeConfig", "true"], root);
        git(&["config", "user.email", "ctx@example.com"], root);
        git(&["config", "user.name", "Ctx Test"], root);
        std::fs::write(root.join("README.md"), "hello\n").expect("write readme");
        git(&["add", "README.md"], root);
        git(&["commit", "-m", "initial"], root);
        git_output(&["rev-parse", "HEAD"], root)
    }

    async fn test_state(data_root: &Path) -> Arc<AppState> {
        Arc::new(AppState::new(
            data_root.to_path_buf(),
            StoreManager::open(data_root).await.expect("open stores"),
            HashMap::new(),
            "http://127.0.0.1:4310".to_string(),
            None,
        ))
    }

    async fn save_test_execution_settings(state: &Arc<AppState>, execution: ExecutionSettings) {
        let settings = Settings {
            execution: Some(execution),
            ..Default::default()
        };
        save_settings(state.global_store(), &settings)
            .await
            .expect("save settings");
    }

    #[cfg(unix)]
    fn write_sandbox_exec_shim(dir: &Path, container_name: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let path = dir.join("sandbox-cli-vcs-hooks-test.sh");
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\ncmd=\"$1\"\nshift\ncase \"$cmd\" in\n  exec)\n    workdir=\"\"\n    while [ \"$#\" -gt 0 ]; do\n      case \"$1\" in\n        --interactive|--tty)\n          shift\n          ;;\n        --workdir)\n          workdir=\"$2\"\n          shift 2\n          ;;\n        --env)\n          export \"$2\"\n          shift 2\n          ;;\n        *)\n          break\n          ;;\n      esac\n    done\n    actual_container=\"$1\"\n    shift\n    if [ \"$actual_container\" != \"{container}\" ]; then\n      echo \"unexpected container: $actual_container\" >&2\n      exit 1\n    fi\n    cd \"$workdir\"\n    exec \"$@\"\n    ;;\n  *)\n    echo \"unexpected sandbox cli invocation: $cmd $*\" >&2\n    exit 1\n    ;;\nesac\n",
                container = container_name,
            ),
        )
        .expect("write sandbox exec shim");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod sandbox exec shim");
        path
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn sandbox_hooks_live_under_external_vcs_hooks_root_and_cleanup_restores_config() {
        let _serial = crate::test_support::sandbox_cli_env_test_lock()
            .lock()
            .await;
        let temp = tempfile::tempdir().expect("tempdir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("create repo");
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
        let task_id = TaskId::new();
        let worktree_id = WorktreeId::new();
        let managed_root =
            ctx_fs::worktrees::managed_worktree_path(temp.path(), workspace.id, worktree_id);
        let branch_name = format!("ctx/{}/{}", task_id.0, worktree_id.0);
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
        let worktree = Worktree {
            id: worktree_id,
            workspace_id: workspace.id,
            root_path: managed_root.to_string_lossy().to_string(),
            base_commit_sha: base_commit.clone(),
            git_branch: Some(branch_name.clone()),
            vcs_kind: Some(VcsKind::Git),
            base_revision: Some(base_commit.clone()),
            vcs_ref: Some(branch_name),
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
        };
        let worktree = store
            .insert_worktree(worktree)
            .await
            .expect("insert worktree");
        let execution = ExecutionSettings {
            mode: ExecutionMode::Sandbox,
            container: ContainerExecutionSettings {
                runtime: ContainerRuntimeKind::NativeContainer,
                ..Default::default()
            },
        };
        save_test_execution_settings(&state, execution.clone()).await;
        store
            .upsert_sandbox_binding(SandboxBinding {
                worktree_id,
                workspace_id: workspace.id,
                sandbox_instance_id: sandbox_instance_id_for_workspace(workspace.id),
                substrate: SandboxSubstrate::NativeContainer,
                guest_identity: SandboxGuestIdentity::linux_container_ubuntu(),
                profile: SandboxProfile::Standard,
                live_workspace_root: crate::harness_runtime::CTX_CONTAINER_WORKSPACE_ROOT
                    .to_string(),
                live_worktree_root: managed_root.to_string_lossy().to_string(),
                execution_settings_json: Some(
                    serde_json::to_string(&execution).expect("serialize execution snapshot"),
                ),
                container_name: Some(workspace_container_name(workspace.id)),
                host_materialization_root: None,
                created_at: Utc::now(),
            })
            .await
            .expect("insert sandbox binding");

        let original_hooks = temp.path().join("original-hooks");
        set_git_config(
            &managed_root,
            CORE_HOOKS_PATH_KEY,
            &original_hooks.to_string_lossy(),
        )
        .await
        .expect("set original hooks path");

        let sandbox_cli =
            write_sandbox_exec_shim(temp.path(), &workspace_container_name(workspace.id));
        let _sandbox_cli_guard = EnvVarGuard::set(
            "CTX_HARNESS_SANDBOX_CLI_PATH",
            &sandbox_cli.to_string_lossy(),
        );

        ensure_task_commit_hook_sandbox(&state, &workspace, &worktree, &execution, task_id)
            .await
            .expect("install sandbox task hook");

        let hooks_dir = worktree_hooks_dir(temp.path(), workspace.id, worktree.id);
        let hooks_path = hooks_dir.to_string_lossy().to_string();
        assert!(
            tokio::fs::metadata(hooks_dir.join("commit-msg"))
                .await
                .is_ok(),
            "expected external commit-msg hook at {}",
            hooks_dir.display()
        );
        assert!(
            tokio::fs::metadata(managed_root.join(".ctx-hooks"))
                .await
                .is_err(),
            "sandbox worktree should not contain .ctx-hooks"
        );
        assert_eq!(
            get_git_config(&managed_root, CORE_HOOKS_PATH_KEY)
                .await
                .expect("load hooksPath"),
            Some(hooks_path.clone())
        );
        assert_eq!(
            get_git_config(&managed_root, CTX_TASK_ID_KEY)
                .await
                .expect("load task id"),
            Some(task_id.0.to_string())
        );
        assert_eq!(
            get_git_config(&managed_root, CTX_PREV_HOOKS_PATH_KEY)
                .await
                .expect("load prev hooks path"),
            Some(original_hooks.to_string_lossy().to_string())
        );

        cleanup_worktree_hooks(&state, &workspace, &worktree)
            .await
            .expect("cleanup sandbox task hook");

        assert!(
            tokio::fs::metadata(&hooks_dir).await.is_err(),
            "expected hook dir cleanup at {}",
            hooks_dir.display()
        );
        assert_eq!(
            get_git_config(&managed_root, CORE_HOOKS_PATH_KEY)
                .await
                .expect("load restored hooksPath"),
            Some(original_hooks.to_string_lossy().to_string())
        );
        assert_eq!(
            get_git_config(&managed_root, CTX_TASK_ID_KEY)
                .await
                .expect("load cleared task id"),
            None
        );
        assert_eq!(
            get_git_config(&managed_root, CTX_PREV_HOOKS_PATH_KEY)
                .await
                .expect("load cleared prev hooks path"),
            None
        );
    }
}
