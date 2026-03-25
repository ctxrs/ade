use super::*;
use crate::settings::ContainerRuntimeKind;
use crate::worktree_data_plane::{
    apply_data_plane_to_execution_settings, resolve_worktree_data_plane,
};

#[derive(Debug, Clone)]
enum AttachmentRuntime {
    NativeContainer {
        container_id: String,
    },
    SharedVmContainer {
        workspace_id: WorkspaceId,
        worktree_id: WorktreeId,
        worktree_root: PathBuf,
    },
}

async fn ensure_workspace_container_for_attachments(
    state: &AppState,
    workspace: &Workspace,
    worktree: &Worktree,
) -> Result<ContainerRuntimeKind> {
    let effective = execution_effective::effective_execution_settings(state, workspace.id).await?;
    let data_plane = resolve_worktree_data_plane(state, worktree).await?;
    let effective = apply_data_plane_to_execution_settings(&effective, &data_plane)?;
    state
        .execution
        .harness
        .ensure_workspace_container_for_worktree(
            workspace,
            worktree,
            &effective,
            &state.core.daemon_url,
        )
        .await?;
    Ok(effective.container.runtime)
}

async fn attachment_runtime_for_worktree(
    state: &AppState,
    workspace: &Workspace,
    worktree_id: WorktreeId,
    worktree_root: &Path,
) -> Result<AttachmentRuntime> {
    let worktree = state
        .global_store()
        .get_worktree(worktree_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("worktree not found for attachment mount"))?;
    let runtime = ensure_workspace_container_for_attachments(state, workspace, &worktree).await?;
    Ok(match runtime {
        ContainerRuntimeKind::NativeContainer => AttachmentRuntime::NativeContainer {
            container_id: workspace_container_name(workspace.id),
        },
        ContainerRuntimeKind::SharedVmContainer => AttachmentRuntime::SharedVmContainer {
            workspace_id: workspace.id,
            worktree_id,
            worktree_root: worktree_root.to_path_buf(),
        },
    })
}

fn container_attachment_materialized_root(attachment: &WorkspaceAttachment) -> PathBuf {
    PathBuf::from(CTX_CONTAINER_WORKSPACE_ROOT)
        .join(CONTAINER_ATTACHMENTS_SUBDIR)
        .join(attachment.id.0.to_string())
        .join(revision_key(attachment))
}

fn container_attachment_root(attachment: &WorkspaceAttachment) -> PathBuf {
    PathBuf::from(CTX_CONTAINER_WORKSPACE_ROOT)
        .join(CONTAINER_ATTACHMENTS_SUBDIR)
        .join(attachment.id.0.to_string())
}

async fn container_path_exists(state: &AppState, container_id: &str, path: &Path) -> Result<bool> {
    let mut cmd = sandbox_container_command(&state.core.data_root)?;
    cmd.arg("exec")
        .arg("--interactive")
        .arg(container_id)
        .arg("test")
        .arg("-e")
        .arg("--")
        .arg(path);
    let out = cmd.output().await.context("sandbox exec test -e")?;
    Ok(out.status.success())
}

fn command_failure_detail(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !stderr.is_empty() {
        stderr
    } else if !stdout.is_empty() {
        stdout
    } else {
        "unknown sandbox command failure".to_string()
    }
}

async fn container_rm_rf(state: &AppState, container_id: &str, path: &Path) -> Result<()> {
    let mut cmd = sandbox_container_command(&state.core.data_root)?;
    cmd.arg("exec")
        .arg("--interactive")
        .arg(container_id)
        .arg("rm")
        .arg("-rf")
        .arg("--")
        .arg(path);
    let out = cmd.output().await.context("sandbox exec rm -rf")?;
    if out.status.success() {
        Ok(())
    } else {
        anyhow::bail!(
            "container rm -rf failed (status {}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
}

fn avf_guest_rel_path(worktree_root: &Path, target: &Path) -> Result<String> {
    let relative = target.strip_prefix(worktree_root).with_context(|| {
        format!(
            "path {} must stay under AVF worktree root {}",
            target.display(),
            worktree_root.display()
        )
    })?;
    if relative.as_os_str().is_empty() {
        Ok(".".to_string())
    } else {
        Ok(relative.to_string_lossy().to_string())
    }
}

async fn avf_run_capture(
    state: &AppState,
    workspace_id: WorkspaceId,
    worktree_id: WorktreeId,
    worktree_root: &Path,
    command: &str,
    args: &[String],
) -> Result<std::process::Output> {
    crate::workspace_runtime::run_avf_linux_guest_exec_capture(
        &state.core.data_root,
        workspace_id,
        worktree_id,
        worktree_root,
        command,
        args,
        &std::collections::HashMap::new(),
        None,
        false,
    )
    .await
    .with_context(|| format!("AVF guest exec `{command}`"))
}

async fn avf_run_success(
    state: &AppState,
    workspace_id: WorkspaceId,
    worktree_id: WorktreeId,
    worktree_root: &Path,
    command: &str,
    args: &[String],
) -> Result<()> {
    let out = avf_run_capture(
        state,
        workspace_id,
        worktree_id,
        worktree_root,
        command,
        args,
    )
    .await?;
    if out.status.success() {
        Ok(())
    } else {
        anyhow::bail!(
            "AVF guest exec `{command}` failed (status {}): {}",
            out.status,
            command_failure_detail(&out)
        );
    }
}

async fn avf_rm_rf(
    state: &AppState,
    workspace_id: WorkspaceId,
    worktree_id: WorktreeId,
    worktree_root: &Path,
    path: &Path,
) -> Result<()> {
    let rel = avf_guest_rel_path(worktree_root, path)?;
    avf_run_success(
        state,
        workspace_id,
        worktree_id,
        worktree_root,
        "rm",
        &["-rf".to_string(), "--".to_string(), rel],
    )
    .await
}

async fn avf_mkdir_p(
    state: &AppState,
    workspace_id: WorkspaceId,
    worktree_id: WorktreeId,
    worktree_root: &Path,
    path: &Path,
) -> Result<()> {
    let rel = avf_guest_rel_path(worktree_root, path)?;
    avf_run_success(
        state,
        workspace_id,
        worktree_id,
        worktree_root,
        "mkdir",
        &["-p".to_string(), "--".to_string(), rel],
    )
    .await
}

async fn import_dir_to_avf_worktree(
    state: &AppState,
    workspace_id: WorkspaceId,
    worktree_id: WorktreeId,
    worktree_root: &Path,
    src: &Path,
    dest: &Path,
) -> Result<()> {
    let dest_rel = avf_guest_rel_path(worktree_root, dest)?;
    let mut tar_cmd = Command::new("tar");
    tar_cmd.arg("-C").arg(src).arg("-cf").arg("-").arg(".");
    tar_cmd.stdout(Stdio::piped());
    let mut tar_child = tar_cmd
        .spawn()
        .context("spawning tar for AVF attachment import")?;
    let mut tar_out = tar_child.stdout.take().context("taking tar stdout")?;

    let mut guest_cmd = crate::workspace_runtime::build_avf_linux_guest_exec_command(
        &state.core.data_root,
        workspace_id,
        worktree_id,
        worktree_root,
        "tar",
        &[
            "-C".to_string(),
            dest_rel,
            "-xf".to_string(),
            "-".to_string(),
        ],
        &std::collections::HashMap::new(),
        None,
        false,
    )?;
    guest_cmd.stdin(Stdio::piped());
    let mut guest_child = guest_cmd
        .spawn()
        .context("spawning AVF guest tar extract")?;
    let mut guest_in = guest_child
        .stdin
        .take()
        .context("taking AVF guest tar stdin")?;

    tokio::io::copy(&mut tar_out, &mut guest_in)
        .await
        .context("streaming tar to AVF guest exec")?;
    drop(guest_in);

    let tar_status = tar_child.wait().await.context("waiting on host tar")?;
    if !tar_status.success() {
        anyhow::bail!("tar failed with status {tar_status}");
    }
    let out = guest_child
        .wait_with_output()
        .await
        .context("waiting on AVF guest tar extract")?;
    if !out.status.success() {
        anyhow::bail!(
            "AVF guest tar extract failed (status {}): {}",
            out.status,
            command_failure_detail(&out)
        );
    }
    Ok(())
}

async fn import_file_to_avf_worktree(
    state: &AppState,
    workspace_id: WorkspaceId,
    worktree_id: WorktreeId,
    worktree_root: &Path,
    src: &Path,
    dest: &Path,
) -> Result<()> {
    let dest_rel = avf_guest_rel_path(worktree_root, dest)?;
    let mut guest_cmd = crate::workspace_runtime::build_avf_linux_guest_exec_command(
        &state.core.data_root,
        workspace_id,
        worktree_id,
        worktree_root,
        "sh",
        &[
            "-lc".to_string(),
            "set -eu; cat > \"$1\"".to_string(),
            "--".to_string(),
            dest_rel,
        ],
        &std::collections::HashMap::new(),
        None,
        false,
    )?;
    guest_cmd.stdin(Stdio::piped());
    let mut guest_child = guest_cmd.spawn().context("spawning AVF guest file copy")?;
    let mut guest_in = guest_child
        .stdin
        .take()
        .context("taking AVF guest file stdin")?;
    let mut source = tokio::fs::File::open(src)
        .await
        .with_context(|| format!("opening source file {}", src.display()))?;
    tokio::io::copy(&mut source, &mut guest_in)
        .await
        .with_context(|| format!("streaming file {} into AVF guest", src.display()))?;
    drop(guest_in);
    let out = guest_child
        .wait_with_output()
        .await
        .context("waiting on AVF guest file copy")?;
    if !out.status.success() {
        anyhow::bail!(
            "AVF guest file copy failed (status {}): {}",
            out.status,
            command_failure_detail(&out)
        );
    }
    Ok(())
}

async fn avf_copy_source_to_mount(
    state: &AppState,
    workspace_id: WorkspaceId,
    worktree_id: WorktreeId,
    worktree_root: &Path,
    source: &Path,
    target: &Path,
) -> Result<()> {
    if let Some(parent) = target.parent() {
        avf_mkdir_p(state, workspace_id, worktree_id, worktree_root, parent).await?;
    }
    let _ = avf_rm_rf(state, workspace_id, worktree_id, worktree_root, target).await;
    let metadata = tokio::fs::metadata(source)
        .await
        .with_context(|| format!("stat attachment source {}", source.display()))?;
    if metadata.is_dir() {
        avf_mkdir_p(state, workspace_id, worktree_id, worktree_root, target).await?;
        import_dir_to_avf_worktree(
            state,
            workspace_id,
            worktree_id,
            worktree_root,
            source,
            target,
        )
        .await
    } else {
        import_file_to_avf_worktree(
            state,
            workspace_id,
            worktree_id,
            worktree_root,
            source,
            target,
        )
        .await
    }
}

async fn container_mkdir_p(state: &AppState, container_id: &str, path: &Path) -> Result<()> {
    let mut cmd = sandbox_container_command(&state.core.data_root)?;
    cmd.arg("exec")
        .arg("--interactive")
        .arg(container_id)
        .arg("mkdir")
        .arg("-p")
        .arg("--")
        .arg(path);
    let out = cmd.output().await.context("sandbox exec mkdir -p")?;
    if out.status.success() {
        Ok(())
    } else {
        anyhow::bail!(
            "container mkdir -p failed (status {}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
}

async fn import_dir_to_container(
    state: &AppState,
    container_id: &str,
    src: &Path,
    dest: &Path,
) -> Result<()> {
    // Stream a tar archive into the container so extracted files are writable by the execution
    // user (avoids `container cp` ownership quirks).
    let mut tar_cmd = Command::new("tar");
    tar_cmd.arg("-C").arg(src).arg("-cf").arg("-").arg(".");
    tar_cmd.stdout(Stdio::piped());
    let mut tar_child = tar_cmd.spawn().context("spawning tar")?;
    let mut tar_out = tar_child.stdout.take().context("taking tar stdout")?;

    let mut pod_cmd = sandbox_container_command(&state.core.data_root)?;
    pod_cmd
        .arg("exec")
        .arg("--interactive")
        .arg("--workdir")
        .arg(dest)
        .arg(container_id)
        .arg("tar")
        .arg("-xf")
        .arg("-");
    pod_cmd.stdin(Stdio::piped());
    let mut pod_child = pod_cmd.spawn().context("spawning sandbox exec tar")?;
    let mut pod_in = pod_child
        .stdin
        .take()
        .context("taking sandbox exec stdin")?;

    tokio::io::copy(&mut tar_out, &mut pod_in)
        .await
        .context("streaming tar to sandbox exec")?;
    drop(pod_in);

    let tar_status = tar_child.wait().await.context("waiting on tar")?;
    if !tar_status.success() {
        anyhow::bail!("tar failed with status {tar_status}");
    }
    let out = pod_child
        .wait_with_output()
        .await
        .context("waiting on sandbox exec tar")?;
    if !out.status.success() {
        anyhow::bail!(
            "sandbox exec tar failed (status {}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

async fn ensure_attachment_imported_to_container(
    state: &AppState,
    container_id: &str,
    attachment: &WorkspaceAttachment,
    src_dir: &Path,
    refresh: bool,
) -> Result<PathBuf> {
    let dest = container_attachment_materialized_root(attachment);
    let exists = if refresh {
        false
    } else {
        container_path_exists(state, container_id, &dest)
            .await
            .unwrap_or(false)
    };
    if exists {
        return Ok(dest);
    }
    // Reset and re-import.
    let _ = container_rm_rf(state, container_id, &dest).await;
    container_mkdir_p(state, container_id, &dest).await?;
    import_dir_to_container(state, container_id, src_dir, &dest).await?;
    Ok(dest)
}

async fn container_ensure_mount(
    state: &AppState,
    container_id: &str,
    target: &Path,
    source: &Path,
) -> Result<()> {
    if let Some(parent) = target.parent() {
        container_mkdir_p(state, container_id, parent).await?;
    }
    // Remove any existing mount path (file/dir/symlink).
    let _ = container_rm_rf(state, container_id, target).await;

    // Prefer symlink; if unavailable, fall back to a recursive copy.
    let mut ln = sandbox_container_command(&state.core.data_root)?;
    ln.arg("exec")
        .arg("--interactive")
        .arg(container_id)
        .arg("ln")
        .arg("-s")
        .arg("--")
        .arg(source)
        .arg(target);
    let out = ln.output().await.context("sandbox exec ln -s")?;
    if out.status.success() {
        return Ok(());
    }

    let mut cp = sandbox_container_command(&state.core.data_root)?;
    cp.arg("exec")
        .arg("--interactive")
        .arg(container_id)
        .arg("cp")
        .arg("-a")
        .arg("--")
        .arg(source)
        .arg(target);
    let out = cp.output().await.context("sandbox exec cp -a")?;
    if out.status.success() {
        Ok(())
    } else {
        anyhow::bail!(
            "container mount failed (ln+cp) (status {}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
}

pub(super) async fn container_ensure_git_exclude(
    state: &AppState,
    workspace: &Workspace,
    worktree_id: WorktreeId,
    worktree_root: &Path,
) -> Result<()> {
    let runtime =
        attachment_runtime_for_worktree(state, workspace, worktree_id, worktree_root).await?;
    let script = r#"
set -e
gitdir="$(git rev-parse --git-dir)"
mkdir -p "$gitdir/info"
path="$gitdir/info/exclude"
touch "$path"
for line in ".ctx/attachments/refs/" ".ctx/attachments/docs/"; do
  if ! grep -Fxq "$line" "$path"; then
    printf '%s\n' "$line" >> "$path"
  fi
done
"#;
    match runtime {
        AttachmentRuntime::NativeContainer { container_id } => {
            let mut cmd = sandbox_container_command(&state.core.data_root)?;
            cmd.arg("exec")
                .arg("--interactive")
                .arg("--workdir")
                .arg(worktree_root)
                .arg(&container_id)
                .arg("sh")
                .arg("-lc")
                .arg(script);
            let out = cmd.output().await.context("sandbox exec git exclude")?;
            if out.status.success() {
                Ok(())
            } else {
                anyhow::bail!(
                    "container git exclude failed (status {}): {}",
                    out.status,
                    String::from_utf8_lossy(&out.stderr).trim()
                );
            }
        }
        AttachmentRuntime::SharedVmContainer {
            workspace_id,
            worktree_id,
            worktree_root,
        } => {
            avf_run_success(
                state,
                workspace_id,
                worktree_id,
                &worktree_root,
                "sh",
                &["-lc".to_string(), script.to_string()],
            )
            .await
        }
    }
}

async fn container_remove_mount_path(
    state: &AppState,
    workspace_id: WorkspaceId,
    worktree_id: WorktreeId,
    target: &Path,
) -> Result<()> {
    let effective = execution_effective::effective_execution_settings(state, workspace_id).await?;
    let store = state.store_for_workspace(workspace_id).await?;
    let Some(worktree) = store.get_worktree(worktree_id).await? else {
        return Ok(());
    };
    let data_plane = resolve_worktree_data_plane(state, &worktree).await?;
    let effective = apply_data_plane_to_execution_settings(&effective, &data_plane)?;
    match effective.container.runtime {
        ContainerRuntimeKind::NativeContainer => {
            let container_id = workspace_container_name(workspace_id);
            // Best-effort: if the container doesn't exist, skip.
            let mut exists = sandbox_container_command(&state.core.data_root)?;
            exists.arg("container").arg("exists").arg(&container_id);
            let out = exists.output().await.context("container exists")?;
            if !out.status.success() {
                return Ok(());
            }
            let _ = container_rm_rf(state, &container_id, target).await;
            Ok(())
        }
        ContainerRuntimeKind::SharedVmContainer => {
            let worktree_root = PathBuf::from(worktree.root_path);
            let _ = avf_rm_rf(state, workspace_id, worktree_id, &worktree_root, target).await;
            Ok(())
        }
    }
}

async fn container_remove_attachment_data_best_effort(
    state: &AppState,
    workspace_id: WorkspaceId,
    attachment: &WorkspaceAttachment,
) -> Result<()> {
    let effective = execution_effective::effective_execution_settings(state, workspace_id).await?;
    if matches!(
        effective.container.runtime,
        ContainerRuntimeKind::SharedVmContainer
    ) {
        return Ok(());
    }
    let container_id = workspace_container_name(workspace_id);
    let mut exists = sandbox_container_command(&state.core.data_root)?;
    exists.arg("container").arg("exists").arg(&container_id);
    let out = exists.output().await.context("container exists")?;
    if !out.status.success() {
        return Ok(());
    }
    let root = container_attachment_root(attachment);
    let _ = container_rm_rf(state, &container_id, &root).await;
    Ok(())
}

pub(super) async fn ensure_attachment_mount(
    state: &AppState,
    workspace: &Workspace,
    worktree_id: WorktreeId,
    worktree_root: &Path,
    attachment: &WorkspaceAttachment,
    refresh: bool,
    materialize: bool,
) -> Result<WorktreeAttachmentMount> {
    let materialized = if materialize {
        let should_refresh = refresh || attachment.update_policy != AttachmentUpdatePolicy::Manual;
        materialize_attachment(state, workspace, attachment, should_refresh).await?
    } else {
        let path = materialized_path_for_attachment(state, attachment);
        if !path.exists() {
            anyhow::bail!("attachment materialization not found at {}", path.display());
        }
        MaterializationResult {
            path,
            materialized_id: revision_key(attachment),
        }
    };
    let mount_rel = sanitize_mount_relpath(&attachment.mount_relpath)?;
    let mount_abs = worktree_root.join(&mount_rel);
    let worktree = state
        .global_store()
        .get_worktree(worktree_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("worktree not found for attachment mount"))?;
    let data_plane = resolve_worktree_data_plane(state, &worktree).await?;
    let container_mode = matches!(
        data_plane.execution_mode,
        crate::settings::ExecutionMode::Sandbox
    );
    if container_mode {
        let runtime =
            attachment_runtime_for_worktree(state, workspace, worktree_id, worktree_root).await?;
        match runtime {
            AttachmentRuntime::NativeContainer { container_id } => {
                let should_refresh =
                    refresh || attachment.update_policy != AttachmentUpdatePolicy::Manual;
                let imported = ensure_attachment_imported_to_container(
                    state,
                    &container_id,
                    attachment,
                    &materialized.path,
                    should_refresh,
                )
                .await?;
                let source_path = if let Some(subpath) = &attachment.subpath {
                    imported.join(subpath)
                } else {
                    imported
                };
                container_ensure_mount(state, &container_id, &mount_abs, &source_path).await?;
            }
            AttachmentRuntime::SharedVmContainer {
                workspace_id,
                worktree_id,
                worktree_root,
            } => {
                let source_path = if let Some(subpath) = &attachment.subpath {
                    materialized.path.join(subpath)
                } else {
                    materialized.path.clone()
                };
                avf_copy_source_to_mount(
                    state,
                    workspace_id,
                    worktree_id,
                    &worktree_root,
                    &source_path,
                    &mount_abs,
                )
                .await?;
            }
        }
    } else {
        if let Some(parent) = mount_abs.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let source_path = if let Some(subpath) = &attachment.subpath {
            materialized.path.join(subpath)
        } else {
            materialized.path.clone()
        };
        ensure_mount(&mount_abs, &source_path).await?;
    }

    let now = Utc::now();
    let mount = WorktreeAttachmentMount {
        worktree_id,
        attachment_id: attachment.id,
        mount_abs_path: mount_abs.to_string_lossy().to_string(),
        materialized_id: materialized.materialized_id,
        status: WorktreeAttachmentStatus::Ready,
        last_sync_at: Some(now),
        error_message: None,
        created_at: now,
        updated_at: now,
    };
    let store = state
        .store_for_workspace(workspace.id)
        .await
        .context("load workspace store for attachment mount update")?;
    store.upsert_worktree_attachment_mount(&mount).await?;
    Ok(mount)
}

pub(super) async fn cleanup_removed_attachment(
    state: &AppState,
    attachment: &WorkspaceAttachment,
) -> Result<()> {
    let store = state.store_for_workspace(attachment.workspace_id).await?;
    let mounts = store
        .list_worktree_attachment_mounts_for_attachment(attachment.id)
        .await?;
    for mount in mounts {
        let path = PathBuf::from(&mount.mount_abs_path);
        let worktree = state.global_store().get_worktree(mount.worktree_id).await?;
        let container_mode = match worktree {
            Some(worktree) => {
                let data_plane = resolve_worktree_data_plane(state, &worktree).await?;
                matches!(
                    data_plane.execution_mode,
                    crate::settings::ExecutionMode::Sandbox
                )
            }
            None => store
                .get_sandbox_binding(mount.worktree_id)
                .await?
                .is_some(),
        };
        if container_mode {
            let _ = container_remove_mount_path(
                state,
                attachment.workspace_id,
                mount.worktree_id,
                &path,
            )
            .await;
        } else {
            remove_mount_path(&path).await?;
        }
    }
    store
        .delete_worktree_attachment_mounts_for_attachment(attachment.id)
        .await?;
    let root = materialized_root_for_attachment(state, attachment);
    if root.exists() {
        tokio::fs::remove_dir_all(root).await?;
    }
    let _ =
        container_remove_attachment_data_best_effort(state, attachment.workspace_id, attachment)
            .await;
    Ok(())
}
