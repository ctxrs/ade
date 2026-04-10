use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ctx_core::ids::{WorkspaceId, WorktreeId};
use ctx_core::models::Workspace;
use ctx_fs::worktrees::standaloneize_worktree_git_dir;
use ctx_sandbox_container_runtime::SandboxCommandMode;
pub use ctx_sandbox_contract::container_worktree_root;
use ctx_sandbox_contract::sandbox_workspace_root;
use ctx_storage_admission::StorageAdmissionOperation;

mod copy;
mod sandbox;
mod storage;

pub use storage::set_test_preflight_storage_samples_override;

fn sandbox_container_id(workspace_id: WorkspaceId) -> String {
    format!("ctx-harness-{}", workspace_id.0)
}

pub async fn remove_live_worktree_root(
    data_root: &Path,
    mode: &SandboxCommandMode,
    workspace_id: WorkspaceId,
    live_worktree_root: &Path,
) -> Result<()> {
    let container_id = sandbox_container_id(workspace_id);
    sandbox::remove_live_worktree_root(data_root, mode, &container_id, live_worktree_root).await
}

pub async fn ensure_worktree_from_host_copy(
    data_root: &Path,
    mode: &SandboxCommandMode,
    workspace_id: WorkspaceId,
    worktree_id: WorktreeId,
    host_source_root: &Path,
    base_commit_sha: &str,
    branch_name: &str,
) -> Result<PathBuf> {
    let container_id = sandbox_container_id(workspace_id);
    let dest_root = container_worktree_root(worktree_id);
    tracing::info!(
        workspace_id = %workspace_id.0,
        worktree_id = %worktree_id.0,
        container_id = %container_id,
        dest_root = %dest_root.display(),
        "provisioning disk-isolated worktree from host copy"
    );

    standaloneize_worktree_git_dir(host_source_root)
        .await
        .with_context(|| {
            format!(
                "stabilizing sandbox worktree git metadata at {}",
                host_source_root.display()
            )
        })?;

    let estimated_copy_bytes = copy::estimate_self_contained_copy_size_bytes(host_source_root)
        .await
        .with_context(|| {
            format!(
                "estimating self-contained sandbox copy size from {}",
                host_source_root.display()
            )
        })?;
    let workspace_root = sandbox_workspace_root();
    storage::preflight_disk_isolated_copy(
        data_root,
        mode,
        &container_id,
        estimated_copy_bytes,
        &workspace_root,
        StorageAdmissionOperation::DiskIsolatedWorktreeMaterialization,
    )
    .await
    .context("preflighting disk-isolated worktree materialization")?;

    let (copy_root, _staging_guard) =
        copy::prepare_self_contained_copy_root(data_root, host_source_root)
            .await
            .with_context(|| {
                format!(
                    "preparing self-contained sandbox copy root from {}",
                    host_source_root.display()
                )
            })?;

    sandbox::ensure_directory(data_root, mode, &container_id, &dest_root)
        .await
        .context("creating disk-isolated worktree root")?;
    copy::stream_dir_to_container(data_root, mode, &container_id, &copy_root, &dest_root)
        .await
        .context("streaming host copy into disk-isolated worktree")?;
    tracing::debug!(
        workspace_id = %workspace_id.0,
        worktree_id = %worktree_id.0,
        "disk-isolated host copy completed"
    );
    sandbox::best_effort_make_user_writable(data_root, mode, &container_id, &dest_root).await?;

    sandbox::checkout_branch_at_base(
        data_root,
        mode,
        &container_id,
        &dest_root,
        branch_name,
        base_commit_sha,
    )
    .await
    .context("resetting disk-isolated worktree branch")?;
    tracing::debug!(
        workspace_id = %workspace_id.0,
        worktree_id = %worktree_id.0,
        base_commit_sha = %base_commit_sha,
        branch_name = %branch_name,
        "disk-isolated checkout completed"
    );

    sandbox::verify_container_git_repo(data_root, mode, &container_id, &dest_root).await?;
    tracing::info!(
        workspace_id = %workspace_id.0,
        worktree_id = %worktree_id.0,
        "disk-isolated worktree repo verification succeeded"
    );

    Ok(dest_root)
}

pub async fn ensure_workspace_root_from_host_copy(
    data_root: &Path,
    mode: &SandboxCommandMode,
    workspace: &Workspace,
) -> Result<PathBuf> {
    let container_id = sandbox_container_id(workspace.id);
    let dest_root = sandbox_workspace_root();
    if sandbox::verify_container_git_repo(data_root, mode, &container_id, &dest_root)
        .await
        .is_ok()
    {
        return Ok(dest_root);
    }

    #[cfg(windows)]
    {
        let _ = data_root;
        let _ = workspace;
        anyhow::bail!("pre-task sandbox workspace materialization is unsupported on Windows");
    }

    let host_workspace_root = Path::new(&workspace.root_path);
    if !host_workspace_root.exists() {
        anyhow::bail!(
            "host workspace root is unavailable for sandbox materialization: {}",
            host_workspace_root.display()
        );
    }

    standaloneize_worktree_git_dir(host_workspace_root)
        .await
        .with_context(|| {
            format!(
                "stabilizing sandbox workspace git metadata at {}",
                host_workspace_root.display()
            )
        })?;

    let estimated_copy_bytes = copy::estimate_self_contained_copy_size_bytes(host_workspace_root)
        .await
        .with_context(|| {
            format!(
                "estimating self-contained sandbox workspace copy size from {}",
                host_workspace_root.display()
            )
        })?;
    storage::preflight_disk_isolated_copy(
        data_root,
        mode,
        &container_id,
        estimated_copy_bytes,
        &dest_root,
        StorageAdmissionOperation::DiskIsolatedWorkspaceMaterialization,
    )
    .await
    .context("preflighting disk-isolated workspace materialization")?;

    let (copy_root, _staging_guard) =
        copy::prepare_self_contained_copy_root(data_root, host_workspace_root)
            .await
            .with_context(|| {
                format!(
                    "preparing self-contained sandbox workspace copy root from {}",
                    host_workspace_root.display()
                )
            })?;
    sandbox::ensure_empty_container_root(data_root, mode, &container_id, &dest_root)
        .await
        .context("preparing disk-isolated workspace root")?;
    copy::stream_dir_to_container(data_root, mode, &container_id, &copy_root, &dest_root)
        .await
        .context("streaming host copy into disk-isolated workspace root")?;
    sandbox::best_effort_make_user_writable(data_root, mode, &container_id, &dest_root).await?;
    sandbox::verify_container_git_repo(data_root, mode, &container_id, &dest_root)
        .await
        .context("verifying seeded disk-isolated workspace root")?;
    Ok(dest_root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use uuid::Uuid;
    use ctx_sandbox_container_runtime::sandbox_cli_env_test_lock;
    use ctx_sandbox_contract::CTX_CONTAINER_WORKSPACE_ROOT;

    struct EnvGuard {
        key: &'static str,
        prev: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &std::path::Path) -> Self {
            let prev = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, prev }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(value) = self.prev.take() {
                std::env::set_var(self.key, value);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    #[tokio::test]
    async fn ensure_workspace_root_from_host_copy_fails_when_host_workspace_is_missing() {
        let _env_lock = sandbox_cli_env_test_lock().lock().await;
        let temp = tempfile::tempdir().expect("tempdir");
        let log_path = temp.path().join("sandbox-cli.log");
        let cli_path = temp.path().join("fake-sandbox-cli.sh");
        fs::write(
            &cli_path,
            format!(
                "#!/bin/sh\nset -eu\nprintf '%s\\n' \"$*\" >> '{log_path}'\ncmd=\"$1\"\nshift\nif [ \"$cmd\" = \"exec\" ]; then\n  requested_user=\"\"\n  while [ \"$#\" -gt 0 ]; do\n    case \"$1\" in\n      --interactive)\n        shift\n        ;;\n      --user)\n        requested_user=\"$2\"\n        shift 2\n        ;;\n      --workdir)\n        workdir=\"$2\"\n        shift 2\n        ;;\n      *)\n        break\n        ;;\n    esac\n  done\n  container_id=\"$1\"\n  shift\n  command=\"$1\"\n  shift\n  case \"$command\" in\n    sh)\n      if [ \"$1\" = \"-lc\" ] && printf '%s' \"$2\" | grep -q 'git rev-parse'; then\n        exit 1\n      fi\n      exit 0\n      ;;\n    id)\n      if [ \"$1\" = \"-u\" ]; then\n        printf '502\\n'\n        exit 0\n      fi\n      if [ \"$1\" = \"-g\" ]; then\n        printf '20\\n'\n        exit 0\n      fi\n      echo \"unexpected id args: $*\" >&2\n      exit 1\n      ;;\n    chown)\n      if [ \"$requested_user\" != \"root\" ]; then\n        echo \"expected root chown\" >&2\n        exit 1\n      fi\n      exit 0\n      ;;\n    mkdir)\n      exit 0\n      ;;\n    *)\n      echo \"unexpected exec command: $command\" >&2\n      exit 1\n      ;;\n  esac\nfi\nif [ \"$cmd\" = \"cp\" ]; then\n  echo \"unexpected container cp\" >&2\n  exit 1\nfi\necho \"unexpected sandbox cli command: $cmd\" >&2\nexit 1\n",
                log_path = log_path.display(),
            ),
        )
        .expect("write fake sandbox cli");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mut perms = fs::metadata(&cli_path).expect("metadata").permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&cli_path, perms).expect("chmod fake sandbox cli");
        }

        let _cli = EnvGuard::set("CTX_HARNESS_SANDBOX_CLI_PATH", &cli_path);

        let workspace = Workspace {
            id: WorkspaceId(uuid::Uuid::new_v4()),
            name: "missing-root".to_string(),
            root_path: temp
                .path()
                .join("missing-workspace")
                .to_string_lossy()
                .to_string(),
            created_at: chrono::Utc::now(),
            vcs_kind: Some(ctx_core::models::VcsKind::Git),
        };

        let err = ensure_workspace_root_from_host_copy(
            temp.path(),
            &ctx_sandbox_container_runtime::SandboxCommandMode::NativeContainer,
            &workspace,
        )
        .await
        .expect_err("missing host workspace should fail");
        assert!(format!("{err:#}").contains("host workspace root is unavailable"));

        let log = fs::read_to_string(&log_path).unwrap_or_default();
        assert!(!log.contains("chmod 0777"));
        assert!(!log.contains("find \"$1\" -mindepth 1 -maxdepth 1 -exec rm -rf -- {} +"));
        assert!(!log.contains("id -u"));
        assert!(!log.contains("id -g"));
        assert!(!log.contains("exec --interactive --user root"));
        assert!(!log.contains("chown 502:20 /ctx/ws"));
        assert!(!log.contains(" cp "));
    }

    #[tokio::test]
    async fn ensure_worktree_from_host_copy_preflights_against_workspace_volume_root() {
        let _env_lock = sandbox_cli_env_test_lock().lock().await;
        let temp = tempfile::tempdir().expect("tempdir");
        let log_path = temp.path().join("sandbox-cli.log");
        let cli_path = temp.path().join("fake-sandbox-cli.sh");
        let workspace_id = WorkspaceId(Uuid::new_v4());
        let worktree_id = WorktreeId(Uuid::new_v4());
        let container_id = sandbox_container_id(workspace_id);
        fs::write(
            &cli_path,
            format!(
                "#!/bin/sh\nset -eu\nprintf '%s\\n' \"$*\" >> '{log_path}'\ncmd=\"$1\"\nshift\nif [ \"$cmd\" != \"exec\" ]; then\n  echo \"unexpected sandbox cli command: $cmd\" >&2\n  exit 1\nfi\nwhile [ \"$#\" -gt 0 ]; do\n  case \"$1\" in\n    --interactive)\n      shift\n      ;;\n    --workdir)\n      shift 2\n      ;;\n    *)\n      break\n      ;;\n  esac\ndone\nif [ \"$1\" != \"{container_id}\" ]; then\n  echo \"unexpected container: $1\" >&2\n  exit 1\nfi\nshift\ncommand=\"$1\"\nshift\ncase \"$command\" in\n  df)\n    printf 'Filesystem 1024-blocks Used Available Capacity Mounted on\\n'\n    printf 'overlay 10485760 1024 7340032 1%% /ctx/ws\\n'\n    exit 0\n    ;;\n  mkdir)\n    exit 0\n    ;;\n  tar)\n    cat >/dev/null\n    exit 0\n    ;;\n  git)\n    exit 0\n    ;;\n  sh)\n    if [ \"$1\" = \"-lc\" ] && printf '%s' \"$2\" | grep -q 'git rev-parse'; then\n      printf 'true\\n'\n      exit 0\n    fi\n    exit 0\n    ;;\n  *)\n    echo \"unexpected exec command: $command\" >&2\n    exit 1\n    ;;\nesac\n",
                log_path = log_path.display(),
                container_id = container_id,
            ),
        )
        .expect("write fake sandbox cli");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mut perms = fs::metadata(&cli_path).expect("metadata").permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&cli_path, perms).expect("chmod fake sandbox cli");
        }

        let src = temp.path().join("src");
        fs::create_dir_all(src.join(".git")).expect("create git dir");
        fs::write(src.join("README.md"), "hello\n").expect("write readme");
        fs::write(src.join(".git").join("HEAD"), "ref: refs/heads/main\n").expect("write git head");

        let _cli = EnvGuard::set("CTX_HARNESS_SANDBOX_CLI_PATH", &cli_path);
        let expected_container_id = container_id.clone();
        let _storage_override = set_test_preflight_storage_samples_override(std::sync::Arc::new(
            move |data_root,
                  mode,
                  observed_container_id,
                  _estimated_copy_bytes,
                  destination_probe_root,
                  operation,
                  required_bytes| {
                assert!(matches!(
                    mode,
                    ctx_sandbox_container_runtime::SandboxCommandMode::NativeContainer
                ));
                assert_eq!(observed_container_id, expected_container_id);
                assert_eq!(
                    operation,
                    StorageAdmissionOperation::DiskIsolatedWorktreeMaterialization
                );
                assert_eq!(destination_probe_root, Path::new(CTX_CONTAINER_WORKSPACE_ROOT));
                let total_bytes = required_bytes.saturating_add(2 * 1024 * 1024 * 1024);
                Ok((
                    ctx_storage_admission::StorageAdmissionSample {
                        label: "CTX data root".to_string(),
                        path: data_root.to_string_lossy().to_string(),
                        mount_point: "/".to_string(),
                        free_bytes: required_bytes.saturating_add(1024),
                        total_bytes,
                    },
                    ctx_storage_admission::StorageAdmissionSample {
                        label: "sandbox workspace volume".to_string(),
                        path: destination_probe_root.to_string_lossy().to_string(),
                        mount_point: CTX_CONTAINER_WORKSPACE_ROOT.to_string(),
                        free_bytes: required_bytes.saturating_add(1024),
                        total_bytes,
                    },
                ))
            },
        ));

        let dest_root = ensure_worktree_from_host_copy(
            temp.path(),
            &ctx_sandbox_container_runtime::SandboxCommandMode::NativeContainer,
            workspace_id,
            worktree_id,
            &src,
            "deadbeef",
            "ctx/test-preflight-root",
        )
        .await
        .expect("materialize worktree");
        assert_eq!(dest_root, container_worktree_root(worktree_id));

        let log = fs::read_to_string(&log_path).expect("read sandbox cli log");
        assert!(
            !log.contains(&format!(
                "exec --interactive {container_id} df -Pk -- /ctx/ws/worktrees"
            )),
            "preflight should not probe the not-yet-created worktree parent: {log}"
        );
    }
}
