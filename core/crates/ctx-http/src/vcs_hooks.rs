use std::collections::HashMap;
use std::path::Path;

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use ctx_core::ids::TaskId;
#[cfg(test)]
use ctx_core::ids::WorktreeId;
use ctx_core::models::{Workspace, Worktree};
use ctx_harness_runtime::sandbox_container_command;
use ctx_workspace_container::workspace_container_name;
pub use ctx_workspace_services::vcs_hooks::{
    cleanup_workspace_hooks, get_git_config, set_git_config, worktree_hooks_dir,
    CORE_HOOKS_PATH_KEY, CTX_PREV_HOOKS_PATH_KEY, CTX_TASK_ID_KEY,
};
use ctx_workspace_services::vcs_hooks::{
    SandboxContainerRuntime, VcsHooksHost, WorktreeExecutionLocation, WorktreeHookExecution,
};
use ctx_worktree_data_plane::apply_data_plane_to_execution_settings;
use tokio::process::Command;

use crate::daemon::AppState;
use crate::execution_effective;
use crate::settings::{ContainerRuntimeKind, ExecutionMode};
use crate::worktree_data_plane::resolve_worktree_data_plane;

pub async fn ensure_task_commit_hook(
    state: &AppState,
    workspace: &Workspace,
    worktree: &Worktree,
    task_id: TaskId,
) -> Result<()> {
    ctx_workspace_services::vcs_hooks::ensure_task_commit_hook(state, workspace, worktree, task_id)
        .await
}

pub async fn cleanup_worktree_hooks(
    state: &AppState,
    workspace: &Workspace,
    worktree: &Worktree,
) -> Result<()> {
    ctx_workspace_services::vcs_hooks::cleanup_worktree_hooks(state, workspace, worktree).await
}

#[async_trait]
impl VcsHooksHost for AppState {
    fn data_root(&self) -> &Path {
        &self.core.data_root
    }

    async fn worktree_execution(
        &self,
        workspace: &Workspace,
        worktree: &Worktree,
    ) -> Result<WorktreeHookExecution> {
        let data_plane = resolve_worktree_data_plane(self, worktree).await?;
        let settings = execution_effective::effective_execution_settings(self, workspace.id).await?;
        let settings = apply_data_plane_to_execution_settings(&settings, &data_plane)?;
        if matches!(settings.mode, ExecutionMode::Host) {
            return Ok(WorktreeHookExecution {
                location: WorktreeExecutionLocation::Host,
                live_worktree_root: None,
                container_runtime: None,
            });
        }
        Ok(WorktreeHookExecution {
            location: WorktreeExecutionLocation::Sandbox,
            live_worktree_root: Some(data_plane.live_worktree_root.to_string_lossy().to_string()),
            container_runtime: Some(match settings.container.runtime {
                ContainerRuntimeKind::NativeContainer => SandboxContainerRuntime::NativeContainer,
                ContainerRuntimeKind::SharedVmContainer => {
                    SandboxContainerRuntime::SharedVmContainer
                }
            }),
        })
    }

    async fn ensure_workspace_container(
        &self,
        workspace: &Workspace,
        worktree: &Worktree,
    ) -> Result<()> {
        let data_plane = resolve_worktree_data_plane(self, worktree).await?;
        let settings = execution_effective::effective_execution_settings(self, workspace.id).await?;
        let settings = apply_data_plane_to_execution_settings(&settings, &data_plane)?;
        self.execution
            .harness
            .ensure_workspace_container_for_worktree(
                workspace,
                worktree,
                &settings,
                &self.core.daemon_url,
            )
            .await
    }

    async fn sandbox_git_config_get(
        &self,
        workspace: &Workspace,
        worktree: &Worktree,
        execution: &WorktreeHookExecution,
        key: &str,
    ) -> Result<Option<String>> {
        let mut cmd = sandbox_command(
            self,
            workspace,
            worktree,
            execution,
            "git",
            &[
                "config".to_string(),
                "--worktree".to_string(),
                "--get".to_string(),
                key.to_string(),
            ],
        )?;
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
        &self,
        workspace: &Workspace,
        worktree: &Worktree,
        execution: &WorktreeHookExecution,
        key: &str,
        value: &str,
    ) -> Result<()> {
        let mut cmd = sandbox_command(
            self,
            workspace,
            worktree,
            execution,
            "git",
            &[
                "config".to_string(),
                "--worktree".to_string(),
                key.to_string(),
                value.to_string(),
            ],
        )?;
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
        &self,
        workspace: &Workspace,
        worktree: &Worktree,
        execution: &WorktreeHookExecution,
        key: &str,
    ) -> Result<()> {
        let mut cmd = sandbox_command(
            self,
            workspace,
            worktree,
            execution,
            "git",
            &[
                "config".to_string(),
                "--worktree".to_string(),
                "--unset-all".to_string(),
                key.to_string(),
            ],
        )?;
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
}

fn sandbox_command(
    state: &AppState,
    workspace: &Workspace,
    worktree: &Worktree,
    execution: &WorktreeHookExecution,
    command: &str,
    args: &[String],
) -> Result<Command> {
    let live_worktree_root = execution
        .live_worktree_root
        .as_ref()
        .context("sandbox hook execution missing live worktree root")?;
    let runtime = execution
        .container_runtime
        .context("sandbox hook execution missing runtime kind")?;
    match runtime {
        SandboxContainerRuntime::NativeContainer => {
            let mut cmd = sandbox_container_command(&state.core.data_root)?;
            cmd.arg("exec")
                .arg("--interactive")
                .arg("--workdir")
                .arg(live_worktree_root)
                .arg(workspace_container_name(workspace.id))
                .arg(command);
            cmd.args(args);
            Ok(cmd)
        }
        SandboxContainerRuntime::SharedVmContainer => ctx_avf_linux_runtime::build_guest_exec_command(
            &state.core.data_root,
            workspace.id,
            worktree.id,
            Path::new(live_worktree_root),
            command,
            args,
            &HashMap::new(),
            None,
            false,
        ),
    }
}

#[cfg(test)]
async fn ensure_task_commit_hook_sandbox(
    state: &AppState,
    workspace: &Workspace,
    worktree: &Worktree,
    _execution: &crate::settings::ExecutionSettings,
    task_id: TaskId,
) -> Result<()> {
    ensure_task_commit_hook(state, workspace, worktree, task_id).await
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
        SandboxSubstrate, VcsKind,
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
        assert!(status.success(), "git {args:?} failed");
    }

    fn git_output(args: &[&str], cwd: &Path) -> String {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("run git");
        assert!(output.status.success(), "git {args:?} failed");
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn init_git_workspace(root: &Path) -> String {
        git(&["init"], root);
        git(&["symbolic-ref", "HEAD", "refs/heads/main"], root);
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
                "#!/bin/sh\ncmd=\"$1\"\nshift\ncase \"$cmd\" in\n  exec)\n    workdir=\"\"\n    while [ \"$#\" -gt 0 ]; do\n      case \"$1\" in\n        --interactive|--tty)\n          shift\n          ;;\n        --workdir)\n          workdir=\"$2\"\n          shift 2\n          ;;\n        --env)\n          export \"$2\"\n          shift 2\n          ;;\n        *)\n          break\n          ;;\n      esac\n    done\n    actual_container=\"$1\"\n    shift\n    if [ \"$actual_container\" != \"{container_name}\" ]; then\n      echo \"unexpected container: $actual_container\" >&2\n      exit 1\n    fi\n    cd \"$workdir\"\n    exec \"$@\"\n    ;;\n  *)\n    echo \"unexpected sandbox cli invocation: $cmd $*\" >&2\n    exit 1\n    ;;\nesac\n",
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
                live_workspace_root: ctx_sandbox_contract::CTX_CONTAINER_WORKSPACE_ROOT.to_string(),
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
