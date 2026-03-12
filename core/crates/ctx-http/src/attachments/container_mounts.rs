use super::*;

async fn ensure_workspace_container_for_attachments(
    state: &AppState,
    workspace: &Workspace,
) -> Result<String> {
    let effective = execution_effective::effective_execution_settings(state, workspace.id).await?;
    state
        .execution
        .harness
        .ensure_workspace_container(workspace, &effective, &state.core.daemon_url)
        .await?;
    Ok(workspace_container_name(workspace.id))
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
    let mut cmd = podman_command(&state.core.data_root)?;
    cmd.arg("exec")
        .arg("--interactive")
        .arg(container_id)
        .arg("test")
        .arg("-e")
        .arg("--")
        .arg(path);
    let out = cmd.output().await.context("podman exec test -e")?;
    Ok(out.status.success())
}

async fn container_rm_rf(state: &AppState, container_id: &str, path: &Path) -> Result<()> {
    let mut cmd = podman_command(&state.core.data_root)?;
    cmd.arg("exec")
        .arg("--interactive")
        .arg(container_id)
        .arg("rm")
        .arg("-rf")
        .arg("--")
        .arg(path);
    let out = cmd.output().await.context("podman exec rm -rf")?;
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

async fn container_mkdir_p(state: &AppState, container_id: &str, path: &Path) -> Result<()> {
    let mut cmd = podman_command(&state.core.data_root)?;
    cmd.arg("exec")
        .arg("--interactive")
        .arg(container_id)
        .arg("mkdir")
        .arg("-p")
        .arg("--")
        .arg(path);
    let out = cmd.output().await.context("podman exec mkdir -p")?;
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
    // user (avoids `podman cp` ownership quirks).
    let mut tar_cmd = Command::new("tar");
    tar_cmd.arg("-C").arg(src).arg("-cf").arg("-").arg(".");
    tar_cmd.stdout(Stdio::piped());
    let mut tar_child = tar_cmd.spawn().context("spawning tar")?;
    let mut tar_out = tar_child.stdout.take().context("taking tar stdout")?;

    let mut pod_cmd = podman_command(&state.core.data_root)?;
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
    let mut pod_child = pod_cmd.spawn().context("spawning podman exec tar")?;
    let mut pod_in = pod_child.stdin.take().context("taking podman exec stdin")?;

    tokio::io::copy(&mut tar_out, &mut pod_in)
        .await
        .context("streaming tar to podman exec")?;
    drop(pod_in);

    let tar_status = tar_child.wait().await.context("waiting on tar")?;
    if !tar_status.success() {
        anyhow::bail!("tar failed with status {tar_status}");
    }
    let out = pod_child
        .wait_with_output()
        .await
        .context("waiting on podman exec tar")?;
    if !out.status.success() {
        anyhow::bail!(
            "podman exec tar failed (status {}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

async fn ensure_attachment_imported_to_container(
    state: &AppState,
    workspace: &Workspace,
    attachment: &WorkspaceAttachment,
    src_dir: &Path,
    refresh: bool,
) -> Result<PathBuf> {
    let container_id = ensure_workspace_container_for_attachments(state, workspace).await?;
    let dest = container_attachment_materialized_root(attachment);
    let exists = if refresh {
        false
    } else {
        container_path_exists(state, &container_id, &dest)
            .await
            .unwrap_or(false)
    };
    if exists {
        return Ok(dest);
    }
    // Reset and re-import.
    let _ = container_rm_rf(state, &container_id, &dest).await;
    container_mkdir_p(state, &container_id, &dest).await?;
    import_dir_to_container(state, &container_id, src_dir, &dest).await?;
    Ok(dest)
}

async fn container_ensure_mount(
    state: &AppState,
    workspace: &Workspace,
    target: &Path,
    source: &Path,
) -> Result<()> {
    let container_id = ensure_workspace_container_for_attachments(state, workspace).await?;
    if let Some(parent) = target.parent() {
        container_mkdir_p(state, &container_id, parent).await?;
    }
    // Remove any existing mount path (file/dir/symlink).
    let _ = container_rm_rf(state, &container_id, target).await;

    // Prefer symlink; if unavailable, fall back to a recursive copy.
    let mut ln = podman_command(&state.core.data_root)?;
    ln.arg("exec")
        .arg("--interactive")
        .arg(&container_id)
        .arg("ln")
        .arg("-s")
        .arg("--")
        .arg(source)
        .arg(target);
    let out = ln.output().await.context("podman exec ln -s")?;
    if out.status.success() {
        return Ok(());
    }

    let mut cp = podman_command(&state.core.data_root)?;
    cp.arg("exec")
        .arg("--interactive")
        .arg(&container_id)
        .arg("cp")
        .arg("-a")
        .arg("--")
        .arg(source)
        .arg(target);
    let out = cp.output().await.context("podman exec cp -a")?;
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
    worktree_root: &Path,
) -> Result<()> {
    let container_id = ensure_workspace_container_for_attachments(state, workspace).await?;
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
    let mut cmd = podman_command(&state.core.data_root)?;
    cmd.arg("exec")
        .arg("--interactive")
        .arg("--workdir")
        .arg(worktree_root)
        .arg(&container_id)
        .arg("sh")
        .arg("-lc")
        .arg(script);
    let out = cmd.output().await.context("podman exec git exclude")?;
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

async fn container_remove_mount_path(
    state: &AppState,
    workspace_id: WorkspaceId,
    target: &Path,
) -> Result<()> {
    let container_id = workspace_container_name(workspace_id);
    // Best-effort: if the container doesn't exist, skip.
    let mut exists = podman_command(&state.core.data_root)?;
    exists.arg("container").arg("exists").arg(&container_id);
    let out = exists.output().await.context("podman container exists")?;
    if !out.status.success() {
        return Ok(());
    }
    let _ = container_rm_rf(state, &container_id, target).await;
    Ok(())
}

async fn container_remove_attachment_data_best_effort(
    state: &AppState,
    workspace_id: WorkspaceId,
    attachment: &WorkspaceAttachment,
) -> Result<()> {
    let container_id = workspace_container_name(workspace_id);
    let mut exists = podman_command(&state.core.data_root)?;
    exists.arg("container").arg("exists").arg(&container_id);
    let out = exists.output().await.context("podman container exists")?;
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

    let container_mode = is_container_path(worktree_root);
    if container_mode {
        let should_refresh = refresh || attachment.update_policy != AttachmentUpdatePolicy::Manual;
        let imported = ensure_attachment_imported_to_container(
            state,
            workspace,
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
        container_ensure_mount(state, workspace, &mount_abs, &source_path).await?;
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
        if is_container_path(&path) {
            let _ = container_remove_mount_path(state, attachment.workspace_id, &path).await;
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
