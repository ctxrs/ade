use super::*;
use std::process::Stdio;

use tokio::process::Command;

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
    ctx_avf_linux_runtime::run_guest_exec_capture(
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

pub(super) async fn avf_run_success(
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

pub(super) async fn avf_rm_rf(
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

    let mut guest_cmd = ctx_avf_linux_runtime::build_guest_exec_command(
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
    let mut guest_cmd = ctx_avf_linux_runtime::build_guest_exec_command(
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

pub(super) async fn avf_copy_source_to_mount(
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
