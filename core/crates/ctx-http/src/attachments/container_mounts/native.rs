use super::*;
use std::process::Stdio;

use tokio::process::Command;

pub(super) async fn container_path_exists(
    state: &AppState,
    container_id: &str,
    path: &Path,
) -> Result<bool> {
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

pub(super) async fn container_rm_rf(
    state: &AppState,
    container_id: &str,
    path: &Path,
) -> Result<()> {
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

pub(super) async fn container_mkdir_p(
    state: &AppState,
    container_id: &str,
    path: &Path,
) -> Result<()> {
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

pub(super) async fn ensure_attachment_imported_to_container(
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

pub(super) async fn container_ensure_mount(
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
