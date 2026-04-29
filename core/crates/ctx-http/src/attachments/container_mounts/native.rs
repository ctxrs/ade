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

async fn container_reject_source_symlinks(
    state: &AppState,
    container_id: &str,
    source: &Path,
) -> Result<()> {
    let script = r#"
set -eu
if [ -L "$1" ]; then
  printf 'read-only attachment copy refuses symlink: %s\n' "$1" >&2
  exit 2
fi
if [ -d "$1" ]; then
  link="$(find "$1" -type l -print -quit)"
  if [ -n "$link" ]; then
    printf 'read-only attachment copy refuses symlink: %s\n' "$link" >&2
    exit 2
  fi
fi
"#;
    let mut cmd = sandbox_container_command(&state.core.data_root)?;
    cmd.arg("exec")
        .arg("--interactive")
        .arg(container_id)
        .arg("sh")
        .arg("-lc")
        .arg(script)
        .arg("--")
        .arg(source);
    let out = cmd
        .output()
        .await
        .context("sandbox exec reject read-only attachment symlinks")?;
    if out.status.success() {
        Ok(())
    } else {
        anyhow::bail!(
            "container read-only source validation failed (status {}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
}

pub(super) async fn container_prepare_for_removal(
    state: &AppState,
    container_id: &str,
    path: &Path,
) -> Result<()> {
    let mut cmd = sandbox_container_command(&state.core.data_root)?;
    cmd.arg("exec")
        .arg("--interactive")
        .arg(container_id)
        .arg("sh")
        .arg("-lc")
        .arg("if [ -L \"$1\" ]; then exit 0; fi; if [ -e \"$1\" ]; then chmod -R u+w -- \"$1\"; fi")
        .arg("--")
        .arg(path);
    let out = cmd
        .output()
        .await
        .context("sandbox exec prepare attachment removal")?;
    if out.status.success() {
        Ok(())
    } else {
        anyhow::bail!(
            "container attachment removal prep failed (status {}): {}",
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
    mode: AttachmentMode,
) -> Result<()> {
    if let Some(parent) = target.parent() {
        container_mkdir_p(state, container_id, parent).await?;
    }
    // Remove any existing mount path (file/dir/symlink).
    let _ = container_prepare_for_removal(state, container_id, target).await;
    let _ = container_rm_rf(state, container_id, target).await;

    if mode == AttachmentMode::Ro {
        container_reject_source_symlinks(state, container_id, source).await?;

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
        if !out.status.success() {
            anyhow::bail!(
                "container read-only mount copy failed (status {}): {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }

        let mut chmod = sandbox_container_command(&state.core.data_root)?;
        chmod
            .arg("exec")
            .arg("--interactive")
            .arg(container_id)
            .arg("chmod")
            .arg("-R")
            .arg("a-w")
            .arg("--")
            .arg(target);
        let out = chmod.output().await.context("sandbox exec chmod -R a-w")?;
        if out.status.success() {
            return Ok(());
        }
        anyhow::bail!(
            "container read-only mount chmod failed (status {}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }

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
