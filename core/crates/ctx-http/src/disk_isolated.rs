use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result};
use tempfile::TempDir;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::storage_guard::{self, StorageAdmissionOperation, StorageAdmissionSample};
use ctx_core::ids::{WorkspaceId, WorktreeId};
use ctx_core::models::Workspace;

const DISK_ISOLATED_COPY_OVERHEAD_BYTES: u64 = 64 * 1024 * 1024;

fn host_tar_stream_command(src_root: &Path) -> Result<Option<Command>> {
    let entries = std::fs::read_dir(src_root)
        .with_context(|| format!("reading {}", src_root.display()))?
        .collect::<std::io::Result<Vec<_>>>()
        .with_context(|| format!("reading {}", src_root.display()))?;

    if entries.is_empty() {
        return Ok(None);
    }

    #[cfg(target_os = "macos")]
    let mut tar_cmd = {
        let mut cmd = Command::new("bsdtar");
        cmd.arg("--format=pax").arg("--no-mac-metadata");
        cmd
    };
    #[cfg(not(target_os = "macos"))]
    let mut tar_cmd = Command::new("tar");

    tar_cmd
        .arg("-C")
        .arg(src_root)
        .arg("-cf")
        .arg("-")
        // Shared-VM guest exec still truncates explicit multi-entry archives like
        // `-- .git README.md`; archiving `.` is the stable shape now that `/ctx/ws`
        // is normalized to the execution user before import.
        .arg(".");
    tar_cmd.stdout(Stdio::piped());
    Ok(Some(tar_cmd))
}

/// Container path for a disk-isolated worktree root.
pub fn container_worktree_root(worktree_id: WorktreeId) -> PathBuf {
    PathBuf::from("/ctx/ws/worktrees").join(worktree_id.0.to_string())
}

fn disk_isolated_copy_budget_bytes(source_bytes: u64) -> u64 {
    source_bytes.saturating_add(DISK_ISOLATED_COPY_OVERHEAD_BYTES)
}

fn collect_tree_size_bytes(root: &Path) -> Result<u64> {
    fn recurse(path: &Path) -> Result<u64> {
        let meta = std::fs::symlink_metadata(path)
            .with_context(|| format!("reading {}", path.display()))?;
        if meta.is_file() {
            return Ok(meta.len());
        }
        if !meta.is_dir() {
            return Ok(0);
        }
        let mut total = 0u64;
        for entry in
            std::fs::read_dir(path).with_context(|| format!("reading {}", path.display()))?
        {
            let entry = entry.with_context(|| format!("reading {}", path.display()))?;
            total = total.saturating_add(recurse(&entry.path())?);
        }
        Ok(total)
    }

    recurse(root)
}

async fn estimate_tree_size_bytes(root: &Path) -> Result<u64> {
    let root = root.to_path_buf();
    tokio::task::spawn_blocking(move || collect_tree_size_bytes(&root))
        .await
        .context("joining tree-size estimate task")?
}

async fn estimate_self_contained_copy_size_bytes(source_root: &Path) -> Result<u64> {
    let source_bytes = estimate_tree_size_bytes(source_root).await?;
    let dotgit = source_root.join(".git");
    let dotgit_meta = match tokio::fs::symlink_metadata(&dotgit).await {
        Ok(meta) => meta,
        Err(_) => return Ok(source_bytes),
    };
    if dotgit_meta.is_dir() {
        return Ok(source_bytes);
    }

    // Conservative upper bound: staging replaces the lightweight `.git` pointer with a full
    // standalone `.git` directory assembled from the common git dir plus worktree-specific git
    // metadata. Summing both trees can slightly overestimate when files overlap, but it avoids
    // admitting copies that still fail during host-side staging.
    let git_dir = resolve_git_dir(source_root).await?;
    let common_git_dir = resolve_common_git_dir(&git_dir).await?;
    let mut expanded_bytes = source_bytes.saturating_sub(dotgit_meta.len());
    expanded_bytes =
        expanded_bytes.saturating_add(estimate_tree_size_bytes(&common_git_dir).await?);
    if git_dir != common_git_dir {
        expanded_bytes = expanded_bytes.saturating_add(estimate_tree_size_bytes(&git_dir).await?);
    }
    Ok(expanded_bytes)
}

fn reserve_file_active(data_root: &Path) -> bool {
    data_root.join(".storage-guard.reserve").exists()
}

fn host_storage_sample(
    path: &Path,
    label: &str,
    reserve_file_eligible: bool,
) -> Result<StorageAdmissionSample> {
    let free_bytes = fs2::available_space(path)
        .with_context(|| format!("checking free space for {}", path.display()))?;
    let total_bytes = fs2::total_space(path)
        .with_context(|| format!("checking total space for {}", path.display()))?;
    Ok(StorageAdmissionSample {
        label: label.to_string(),
        path: path.to_string_lossy().to_string(),
        mount_point: path.to_string_lossy().to_string(),
        free_bytes,
        total_bytes,
        reserve_file_eligible,
    })
}

fn parse_df_pk_line(line: &str, label: &str, path: &Path) -> Result<StorageAdmissionSample> {
    let parts = line.split_whitespace().collect::<Vec<_>>();
    if parts.len() < 6 {
        anyhow::bail!(
            "unexpected df output for {}: {}",
            path.display(),
            line.trim()
        );
    }
    let total_bytes = parts[1]
        .parse::<u64>()
        .with_context(|| format!("parsing total blocks from df output: {line}"))?
        .saturating_mul(1024);
    let free_bytes = parts[3]
        .parse::<u64>()
        .with_context(|| format!("parsing free blocks from df output: {line}"))?
        .saturating_mul(1024);
    let mount_point = parts[5..].join(" ");
    Ok(StorageAdmissionSample {
        label: label.to_string(),
        path: path.to_string_lossy().to_string(),
        mount_point,
        free_bytes,
        total_bytes,
        reserve_file_eligible: false,
    })
}

async fn sandbox_storage_sample(
    data_root: &Path,
    container_id: &str,
    path: &Path,
    label: &str,
) -> Result<StorageAdmissionSample> {
    const SANDBOX_EXEC_TIMEOUT: Duration = Duration::from_secs(60);
    let mut cmd = crate::harness_runtime::sandbox_container_command(data_root)?;
    cmd.arg("exec")
        .arg("--interactive")
        .arg(container_id)
        .arg("sh")
        .arg("-lc")
        .arg(r#"df -Pk "$1" | tail -n 1"#)
        .arg("sh")
        .arg(path);
    let out = crate::harness_runtime::command_output_with_timeout(cmd, SANDBOX_EXEC_TIMEOUT)
        .await
        .with_context(|| format!("querying sandbox free space for {}", path.display()))?;
    if !out.status.success() {
        anyhow::bail!(
            "failed to query sandbox free space for {} (status {}): {}",
            path.display(),
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    parse_df_pk_line(String::from_utf8_lossy(&out.stdout).trim(), label, path)
}

async fn preflight_disk_isolated_copy(
    data_root: &Path,
    container_id: &str,
    estimated_copy_bytes: u64,
    destination_probe_root: &Path,
    operation: StorageAdmissionOperation,
) -> Result<()> {
    let required_bytes = storage_guard::storage_admission_required_bytes(
        disk_isolated_copy_budget_bytes(estimated_copy_bytes),
    );
    let host_sample = host_storage_sample(data_root, "CTX data root", true)?;
    let sandbox_sample = sandbox_storage_sample(
        data_root,
        container_id,
        destination_probe_root,
        "sandbox workspace volume",
    )
    .await?;
    storage_guard::check_storage_admission(
        operation,
        required_bytes,
        reserve_file_active(data_root),
        &[host_sample.clone(), sandbox_sample.clone()],
    )
    .map_err(|err| {
        tracing::warn!(
            operation = ?operation,
            estimated_copy_bytes,
            required_bytes,
            host_path = %host_sample.path,
            host_free_bytes = host_sample.free_bytes,
            sandbox_path = %sandbox_sample.path,
            sandbox_mount_point = %sandbox_sample.mount_point,
            sandbox_free_bytes = sandbox_sample.free_bytes,
            "storage admission denied disk-isolated materialization",
        );
        err.into()
    })
}

pub async fn remove_live_worktree_root(
    data_root: &Path,
    workspace_id: WorkspaceId,
    live_worktree_root: &Path,
) -> Result<()> {
    const SANDBOX_EXEC_TIMEOUT: Duration = Duration::from_secs(60);
    let container_id = format!("ctx-harness-{}", workspace_id.0);
    let mut cmd = crate::harness_runtime::sandbox_container_command(data_root)?;
    cmd.arg("exec")
        .arg("--interactive")
        .arg(&container_id)
        .arg("rm")
        .arg("-rf")
        .arg("--")
        .arg(live_worktree_root);
    let out = crate::harness_runtime::command_output_with_timeout(cmd, SANDBOX_EXEC_TIMEOUT)
        .await
        .context("sandbox exec rm -rf disk-isolated worktree")?;
    if !out.status.success() {
        anyhow::bail!(
            "failed to remove disk-isolated worktree root {} (status {}): {}",
            live_worktree_root.display(),
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

async fn verify_container_git_repo(
    data_root: &Path,
    container_id: &str,
    worktree_root: &Path,
) -> Result<()> {
    const SANDBOX_EXEC_TIMEOUT: Duration = Duration::from_secs(60);
    let mut cmd = crate::harness_runtime::sandbox_container_command(data_root)?;
    cmd.arg("exec")
        .arg("--interactive")
        .arg("--workdir")
        .arg(worktree_root)
        .arg(container_id)
        .arg("sh")
        .arg("-lc")
        .arg("git rev-parse --is-inside-work-tree && git rev-parse HEAD >/dev/null");
    let out = crate::harness_runtime::command_output_with_timeout(cmd, SANDBOX_EXEC_TIMEOUT)
        .await
        .context("sandbox exec git repo verification")?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let detail = if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            "unknown error".to_string()
        };
        anyhow::bail!(
            "disk-isolated worktree verification failed (status {}): {}",
            out.status,
            detail
        );
    }
    Ok(())
}

async fn ensure_empty_container_root(
    data_root: &Path,
    container_id: &str,
    dest_root: &Path,
) -> Result<()> {
    const SANDBOX_EXEC_TIMEOUT: Duration = Duration::from_secs(60);

    let mut normalize = crate::harness_runtime::sandbox_container_command(data_root)?;
    normalize
        .arg("exec")
        .arg("--interactive")
        .arg("--user")
        .arg("root")
        .arg(container_id)
        .arg("sh")
        .arg("-lc")
        .arg(r#"mkdir -p -- "$1" && chmod 0777 "$1""#)
        .arg("sh")
        .arg(dest_root);
    let out = crate::harness_runtime::command_output_with_timeout(normalize, SANDBOX_EXEC_TIMEOUT)
        .await
        .context("sandbox exec normalize disk-isolated root")?;
    if !out.status.success() {
        anyhow::bail!(
            "failed to normalize disk-isolated root {} (status {}): {}",
            dest_root.display(),
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }

    let mut clear = crate::harness_runtime::sandbox_container_command(data_root)?;
    clear
        .arg("exec")
        .arg("--interactive")
        .arg("--user")
        .arg("root")
        .arg(container_id)
        .arg("sh")
        .arg("-lc")
        .arg(r#"find "$1" -mindepth 1 -maxdepth 1 -exec rm -rf -- {} +"#)
        .arg("sh")
        .arg(dest_root);
    let out = crate::harness_runtime::command_output_with_timeout(clear, SANDBOX_EXEC_TIMEOUT)
        .await
        .context("sandbox exec clear disk-isolated root")?;
    if !out.status.success() {
        anyhow::bail!(
            "failed to clear disk-isolated root {} (status {}): {}",
            dest_root.display(),
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }

    let uid = resolve_container_exec_id(data_root, container_id, "-u")
        .await
        .context("resolving sandbox exec uid for disk-isolated root")?;
    let gid = resolve_container_exec_id(data_root, container_id, "-g")
        .await
        .context("resolving sandbox exec gid for disk-isolated root")?;
    let mut chown = crate::harness_runtime::sandbox_container_command(data_root)?;
    chown
        .arg("exec")
        .arg("--interactive")
        .arg("--user")
        .arg("root")
        .arg(container_id)
        .arg("chown")
        .arg(format!("{uid}:{gid}"))
        .arg(dest_root);
    let out = crate::harness_runtime::command_output_with_timeout(chown, SANDBOX_EXEC_TIMEOUT)
        .await
        .context("sandbox exec chown disk-isolated root")?;
    if !out.status.success() {
        anyhow::bail!(
            "failed to set disk-isolated root owner {} to {}:{} (status {}): {}",
            dest_root.display(),
            uid,
            gid,
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }

    Ok(())
}

async fn resolve_container_exec_id(
    data_root: &Path,
    container_id: &str,
    id_flag: &str,
) -> Result<u32> {
    const SANDBOX_EXEC_TIMEOUT: Duration = Duration::from_secs(60);
    let mut cmd = crate::harness_runtime::sandbox_container_command(data_root)?;
    cmd.arg("exec")
        .arg("--interactive")
        .arg(container_id)
        .arg("id")
        .arg(id_flag);
    let out = crate::harness_runtime::command_output_with_timeout(cmd, SANDBOX_EXEC_TIMEOUT)
        .await
        .with_context(|| format!("sandbox exec id {id_flag}"))?;
    if !out.status.success() {
        anyhow::bail!(
            "failed to resolve sandbox exec id {} (status {}): {}",
            id_flag,
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<u32>()
        .with_context(|| format!("parsing sandbox exec id {} output", id_flag))
}

async fn stream_dir_to_container(
    data_root: &Path,
    container_id: &str,
    src_root: &Path,
    dest_root: &Path,
) -> Result<()> {
    let mut sandbox_cmd = crate::harness_runtime::sandbox_container_command(data_root)?;
    sandbox_cmd
        .arg("exec")
        .arg("--interactive")
        .arg("--workdir")
        .arg(dest_root)
        .arg(container_id)
        .arg("tar")
        .arg("-xf")
        .arg("-");
    sandbox_cmd.stdin(Stdio::piped());
    let mut sandbox_child = sandbox_cmd
        .spawn()
        .context("spawning sandbox exec tar for disk-isolated copy")?;
    let mut sandbox_in = sandbox_child
        .stdin
        .take()
        .context("taking sandbox exec stdin for disk-isolated copy")?;

    let Some(mut tar_cmd) = host_tar_stream_command(src_root)? else {
        return Ok(());
    };
    let mut tar_child = tar_cmd
        .spawn()
        .context("spawning tar for disk-isolated copy")?;
    let mut tar_out = tar_child
        .stdout
        .take()
        .context("taking tar stdout for disk-isolated copy")?;

    tokio::io::copy(&mut tar_out, &mut sandbox_in)
        .await
        .context("streaming disk-isolated tar archive into container")?;
    sandbox_in
        .shutdown()
        .await
        .context("closing sandbox exec stdin for disk-isolated copy")?;
    drop(sandbox_in);

    let tar_status = tar_child.wait().await.context("waiting on tar")?;
    if !tar_status.success() {
        anyhow::bail!("tar failed with status {tar_status}");
    }
    let out = sandbox_child
        .wait_with_output()
        .await
        .context("waiting on sandbox exec tar for disk-isolated copy")?;
    if !out.status.success() {
        anyhow::bail!(
            "sandbox exec tar failed (status {}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

async fn resolve_git_dir(worktree_root: &Path) -> Result<PathBuf> {
    let dotgit = worktree_root.join(".git");
    let meta = tokio::fs::symlink_metadata(&dotgit)
        .await
        .with_context(|| format!("reading {}", dotgit.display()))?;
    if meta.is_dir() {
        return Ok(dotgit);
    }
    let txt = tokio::fs::read_to_string(&dotgit)
        .await
        .with_context(|| format!("reading {}", dotgit.display()))?;
    let line = txt
        .lines()
        .find(|value| value.trim_start().starts_with("gitdir:"))
        .ok_or_else(|| anyhow::anyhow!("invalid .git file: missing gitdir"))?;
    let raw = line.trim_start().trim_start_matches("gitdir:").trim();
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(worktree_root.join(path))
    }
}

async fn resolve_common_git_dir(git_dir: &Path) -> Result<PathBuf> {
    let commondir = git_dir.join("commondir");
    let meta = match tokio::fs::symlink_metadata(&commondir).await {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(git_dir.to_path_buf());
        }
        Err(err) => return Err(err).with_context(|| format!("reading {}", commondir.display())),
    };
    if !meta.is_file() {
        return Ok(git_dir.to_path_buf());
    }
    let raw = tokio::fs::read_to_string(&commondir)
        .await
        .with_context(|| format!("reading {}", commondir.display()))?;
    let path = PathBuf::from(raw.trim());
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(git_dir.join(path))
    }
}

#[cfg(unix)]
fn symlink_path(target: &Path, dest: &Path, _is_dir: bool) -> Result<()> {
    std::os::unix::fs::symlink(target, dest)?;
    Ok(())
}

#[cfg(windows)]
fn symlink_path(target: &Path, dest: &Path, is_dir: bool) -> Result<()> {
    if is_dir {
        std::os::windows::fs::symlink_dir(target, dest)?;
    } else {
        std::os::windows::fs::symlink_file(target, dest)?;
    }
    Ok(())
}

fn copy_dir_recursive(source: &Path, target: &Path) -> Result<()> {
    std::fs::create_dir_all(target)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let entry_path = entry.path();
        let dest = target.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&entry_path, &dest)?;
        } else if file_type.is_symlink() {
            if dest.exists() {
                let _ = std::fs::remove_file(&dest);
                let _ = std::fs::remove_dir_all(&dest);
            }
            let link_target = std::fs::read_link(&entry_path)?;
            let is_dir = std::fs::metadata(&entry_path)
                .map(|meta| meta.is_dir())
                .unwrap_or(false);
            symlink_path(&link_target, &dest, is_dir)?;
        } else if file_type.is_file() {
            std::fs::copy(&entry_path, &dest)?;
        }
    }
    Ok(())
}

async fn prepare_self_contained_copy_root(
    data_root: &Path,
    source_root: &Path,
) -> Result<(PathBuf, Option<TempDir>)> {
    let dotgit = source_root.join(".git");
    let dotgit_meta = match tokio::fs::symlink_metadata(&dotgit).await {
        Ok(meta) => meta,
        Err(_) => return Ok((source_root.to_path_buf(), None)),
    };
    if dotgit_meta.is_dir() {
        return Ok((source_root.to_path_buf(), None));
    }

    let git_dir = resolve_git_dir(source_root).await?;
    let common_git_dir = resolve_common_git_dir(&git_dir).await?;
    let staging_parent = data_root.join("disk-isolated").join("staging");
    tokio::fs::create_dir_all(&staging_parent)
        .await
        .with_context(|| format!("creating {}", staging_parent.display()))?;
    let staging = TempDir::new_in(&staging_parent)
        .with_context(|| format!("creating temp dir in {}", staging_parent.display()))?;
    let staging_root = staging.path().join("worktree");
    let source = source_root.to_path_buf();
    let git_dir_copy = git_dir.clone();
    let common_git_dir_copy = common_git_dir.clone();
    let staging_copy = staging_root.clone();
    tokio::task::spawn_blocking(move || -> Result<()> {
        copy_dir_recursive(&source, &staging_copy)?;
        let staged_dotgit = staging_copy.join(".git");
        if staged_dotgit.exists() {
            if staged_dotgit.is_dir() {
                std::fs::remove_dir_all(&staged_dotgit)?;
            } else {
                std::fs::remove_file(&staged_dotgit)?;
            }
        }
        copy_dir_recursive(&common_git_dir_copy, &staged_dotgit)?;
        if git_dir_copy != common_git_dir_copy {
            copy_dir_recursive(&git_dir_copy, &staged_dotgit)?;
        }
        let commondir = staged_dotgit.join("commondir");
        if commondir.exists() {
            std::fs::remove_file(&commondir)?;
        }
        let gitdir = staged_dotgit.join("gitdir");
        if gitdir.exists() {
            std::fs::remove_file(&gitdir)?;
        }
        Ok(())
    })
    .await??;

    Ok((staging_root, Some(staging)))
}

pub async fn ensure_worktree_from_host_copy(
    data_root: &Path,
    workspace_id: WorkspaceId,
    worktree_id: WorktreeId,
    host_workspace_root: &Path,
    base_commit_sha: &str,
    branch_name: &str,
) -> Result<PathBuf> {
    const SANDBOX_EXEC_TIMEOUT: Duration = Duration::from_secs(60);
    let container_id = format!("ctx-harness-{}", workspace_id.0);
    let dest_root = container_worktree_root(worktree_id);
    tracing::info!(
        workspace_id = %workspace_id.0,
        worktree_id = %worktree_id.0,
        container_id = %container_id,
        dest_root = %dest_root.display(),
        "provisioning disk-isolated worktree from host copy"
    );
    let estimated_copy_bytes = estimate_self_contained_copy_size_bytes(host_workspace_root)
        .await
        .with_context(|| {
            format!(
                "estimating self-contained sandbox copy size from {}",
                host_workspace_root.display()
            )
        })?;
    preflight_disk_isolated_copy(
        data_root,
        &container_id,
        estimated_copy_bytes,
        dest_root.parent().unwrap_or(dest_root.as_path()),
        StorageAdmissionOperation::DiskIsolatedWorktreeMaterialization,
    )
    .await
    .context("preflighting disk-isolated worktree materialization")?;
    let (copy_root, _staging_guard) =
        prepare_self_contained_copy_root(data_root, host_workspace_root)
            .await
            .with_context(|| {
                format!(
                    "preparing self-contained sandbox copy root from {}",
                    host_workspace_root.display()
                )
            })?;
    // 1) Create destination directory.
    {
        let mut cmd = crate::harness_runtime::sandbox_container_command(data_root)?;
        cmd.arg("exec")
            .arg("--interactive")
            .arg(&container_id)
            .arg("mkdir")
            .arg("-p")
            .arg("--")
            .arg(&dest_root);
        let out = crate::harness_runtime::command_output_with_timeout(cmd, SANDBOX_EXEC_TIMEOUT)
            .await
            .context("sandbox exec mkdir")?;
        if !out.status.success() {
            anyhow::bail!(
                "failed to create disk-isolated worktree dir (status {}): {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
    }

    // 2) Stream host workspace contents into the container worktree root.
    //
    // This is intentionally a one-time copy for v1. The disk-isolated worktree becomes the
    // canonical filesystem for the workbench + agents.
    {
        stream_dir_to_container(data_root, &container_id, &copy_root, &dest_root)
            .await
            .context("streaming host copy into disk-isolated worktree")?;
        tracing::debug!(
            workspace_id = %workspace_id.0,
            worktree_id = %worktree_id.0,
            "disk-isolated host copy completed"
        );

        // Best-effort: ensure files are writable for the execution user.
        let mut chmod = crate::harness_runtime::sandbox_container_command(data_root)?;
        chmod
            .arg("exec")
            .arg("--interactive")
            .arg("--workdir")
            .arg(&dest_root)
            .arg(&container_id)
            .arg("sh")
            .arg("-lc")
            .arg("chmod -R u+rwX . >/dev/null 2>&1 || true");
        let _ =
            crate::harness_runtime::command_output_with_timeout(chmod, SANDBOX_EXEC_TIMEOUT).await;
    }

    // 3) Create/reset the worktree branch at the base revision.
    {
        let mut cmd = crate::harness_runtime::sandbox_container_command(data_root)?;
        cmd.arg("exec")
            .arg("--interactive")
            .arg("--workdir")
            .arg(&dest_root)
            .arg(&container_id)
            .arg("git")
            .arg("checkout")
            .arg("-B")
            .arg(branch_name)
            .arg(base_commit_sha);
        let out = crate::harness_runtime::command_output_with_timeout(cmd, SANDBOX_EXEC_TIMEOUT)
            .await
            .context("sandbox exec git checkout")?;
        if !out.status.success() {
            anyhow::bail!(
                "git checkout failed (status {}): {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        tracing::debug!(
            workspace_id = %workspace_id.0,
            worktree_id = %worktree_id.0,
            base_commit_sha = %base_commit_sha,
            branch_name = %branch_name,
            "disk-isolated checkout completed"
        );
    }

    // 4) Verify repository integrity after copy + checkout. We fail fast here so callers never
    // register a worktree/session that cannot answer git/diff requests later.
    verify_container_git_repo(data_root, &container_id, &dest_root).await?;
    tracing::info!(
        workspace_id = %workspace_id.0,
        worktree_id = %worktree_id.0,
        "disk-isolated worktree repo verification succeeded"
    );

    Ok(dest_root)
}

pub async fn ensure_workspace_root_from_host_copy(
    data_root: &Path,
    workspace: &Workspace,
) -> Result<PathBuf> {
    const SANDBOX_EXEC_TIMEOUT: Duration = Duration::from_secs(60);
    let container_id = format!("ctx-harness-{}", workspace.id.0);
    let dest_root = PathBuf::from(crate::harness_runtime::CTX_CONTAINER_WORKSPACE_ROOT);
    if verify_container_git_repo(data_root, &container_id, &dest_root)
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
    let estimated_copy_bytes = estimate_self_contained_copy_size_bytes(host_workspace_root)
        .await
        .with_context(|| {
            format!(
                "estimating self-contained sandbox workspace copy size from {}",
                host_workspace_root.display()
            )
        })?;
    preflight_disk_isolated_copy(
        data_root,
        &container_id,
        estimated_copy_bytes,
        &dest_root,
        StorageAdmissionOperation::DiskIsolatedWorkspaceMaterialization,
    )
    .await
    .context("preflighting disk-isolated workspace materialization")?;
    let (copy_root, _staging_guard) =
        prepare_self_contained_copy_root(data_root, host_workspace_root)
            .await
            .with_context(|| {
                format!(
                    "preparing self-contained sandbox workspace copy root from {}",
                    host_workspace_root.display()
                )
            })?;
    ensure_empty_container_root(data_root, &container_id, &dest_root)
        .await
        .context("preparing disk-isolated workspace root")?;

    stream_dir_to_container(data_root, &container_id, &copy_root, &dest_root)
        .await
        .context("streaming host copy into disk-isolated workspace root")?;

    let mut chmod = crate::harness_runtime::sandbox_container_command(data_root)?;
    chmod
        .arg("exec")
        .arg("--interactive")
        .arg("--workdir")
        .arg(&dest_root)
        .arg(&container_id)
        .arg("sh")
        .arg("-lc")
        .arg("chmod -R u+rwX . >/dev/null 2>&1 || true");
    let _ = crate::harness_runtime::command_output_with_timeout(chmod, SANDBOX_EXEC_TIMEOUT).await;

    verify_container_git_repo(data_root, &container_id, &dest_root)
        .await
        .context("verifying seeded disk-isolated workspace root")?;
    Ok(dest_root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command as StdCommand;

    fn git(args: &[&str], cwd: &Path) {
        let status = StdCommand::new("git")
            .args(args)
            .current_dir(cwd)
            .status()
            .expect("run git command");
        assert!(status.success(), "git command failed: {args:?}");
    }

    #[tokio::test]
    async fn prepare_self_contained_copy_root_makes_git_worktree_clone_standalone() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("create repo root");
        git(&["init", "-b", "main"], &repo_root);
        git(&["config", "user.name", "Test User"], &repo_root);
        git(&["config", "user.email", "test@example.com"], &repo_root);
        std::fs::write(repo_root.join("README.md"), "hello\n").expect("write readme");
        git(&["add", "README.md"], &repo_root);
        git(&["commit", "-m", "initial"], &repo_root);

        let worktree_root = temp.path().join("worktree");
        git(
            &[
                "worktree",
                "add",
                "-b",
                "ctx/test-worktree",
                worktree_root.to_str().expect("worktree path"),
            ],
            &repo_root,
        );

        let (copy_root, _guard) = prepare_self_contained_copy_root(temp.path(), &worktree_root)
            .await
            .expect("prepare self-contained root");
        assert_ne!(copy_root, worktree_root);
        assert!(copy_root.join(".git").is_dir());
        assert!(!copy_root.join(".git").join("commondir").exists());
        assert!(!copy_root.join(".git").join("gitdir").exists());

        let output = StdCommand::new("git")
            .arg("rev-parse")
            .arg("--is-inside-work-tree")
            .current_dir(&copy_root)
            .output()
            .expect("run git rev-parse");
        assert!(
            output.status.success(),
            "git rev-parse failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "true");
    }

    #[tokio::test]
    async fn estimate_self_contained_copy_size_accounts_for_expanded_git_metadata() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("create repo root");
        git(&["init", "-b", "main"], &repo_root);
        git(&["config", "user.name", "Test User"], &repo_root);
        git(&["config", "user.email", "test@example.com"], &repo_root);
        std::fs::write(repo_root.join("README.md"), "hello\n").expect("write readme");
        git(&["add", "README.md"], &repo_root);
        git(&["commit", "-m", "initial"], &repo_root);

        let worktree_root = temp.path().join("worktree");
        git(
            &[
                "worktree",
                "add",
                "-b",
                "ctx/test-size-estimate",
                worktree_root.to_str().expect("worktree path"),
            ],
            &repo_root,
        );

        let source_bytes = collect_tree_size_bytes(&worktree_root).expect("measure worktree");
        let estimated_bytes = estimate_self_contained_copy_size_bytes(&worktree_root)
            .await
            .expect("estimate self-contained worktree");
        let (copy_root, _guard) = prepare_self_contained_copy_root(temp.path(), &worktree_root)
            .await
            .expect("prepare self-contained root");
        let staged_bytes = collect_tree_size_bytes(&copy_root).expect("measure staged copy");

        assert!(estimated_bytes > source_bytes);
        assert!(estimated_bytes >= staged_bytes);
    }

    #[tokio::test]
    async fn ensure_workspace_root_from_host_copy_fails_when_host_workspace_is_missing() {
        let _env_lock = crate::test_support::sandbox_cli_env_test_lock()
            .lock()
            .await;
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

        let old_cli = std::env::var("CTX_HARNESS_SANDBOX_CLI_PATH").ok();
        std::env::set_var("CTX_HARNESS_SANDBOX_CLI_PATH", &cli_path);

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

        let err = ensure_workspace_root_from_host_copy(temp.path(), &workspace)
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

        match old_cli {
            Some(value) => std::env::set_var("CTX_HARNESS_SANDBOX_CLI_PATH", value),
            None => std::env::remove_var("CTX_HARNESS_SANDBOX_CLI_PATH"),
        }
    }

    #[tokio::test]
    async fn stream_dir_to_container_uses_tar_exec_instead_of_container_cp() {
        let _env_lock = crate::test_support::sandbox_cli_env_test_lock()
            .lock()
            .await;
        let temp = tempfile::tempdir().expect("tempdir");
        let log_path = temp.path().join("sandbox-cli.log");
        let cli_path = temp.path().join("fake-sandbox-cli.sh");
        fs::write(
            &cli_path,
            format!(
                "#!/bin/sh\nset -eu\nprintf '%s\\n' \"$*\" >> '{log_path}'\ncmd=\"$1\"\nshift\nif [ \"$cmd\" != \"exec\" ]; then\n  echo \"unexpected sandbox cli command: $cmd\" >&2\n  exit 1\nfi\nwhile [ \"$#\" -gt 0 ]; do\n  case \"$1\" in\n    --interactive)\n      shift\n      ;;\n    --workdir)\n      workdir=\"$2\"\n      shift 2\n      ;;\n    *)\n      break\n      ;;\n  esac\ndone\ncontainer_id=\"$1\"\nshift\ncommand=\"$1\"\nshift\nif [ \"$command\" != \"tar\" ]; then\n  echo \"unexpected exec command: $command\" >&2\n  exit 1\nfi\ncat >/dev/null\nexit 0\n",
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

        let src = temp.path().join("src");
        fs::create_dir_all(&src).expect("create src dir");
        fs::write(src.join("file.txt"), "hello\n").expect("write src file");
        fs::create_dir_all(src.join(".git")).expect("create git dir");
        fs::write(src.join(".git").join("HEAD"), "ref: refs/heads/main\n").expect("write git head");

        let old_cli = std::env::var("CTX_HARNESS_SANDBOX_CLI_PATH").ok();
        std::env::set_var("CTX_HARNESS_SANDBOX_CLI_PATH", &cli_path);

        stream_dir_to_container(temp.path(), "ctx-harness-test", &src, Path::new("/ctx/ws"))
            .await
            .expect("stream dir to container");

        let log = fs::read_to_string(&log_path).expect("read sandbox cli log");
        assert!(log.contains("exec --interactive --workdir /ctx/ws ctx-harness-test tar -xf -"));
        assert!(!log.contains(" cp "));

        let tar_cmd = host_tar_stream_command(&src)
            .expect("build host tar stream command")
            .expect("non-empty archive command");
        let program = tar_cmd.as_std().get_program().to_string_lossy().to_string();
        let args = tar_cmd
            .as_std()
            .get_args()
            .map(|value| value.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        #[cfg(target_os = "macos")]
        {
            assert_eq!(program, "bsdtar");
            assert!(
                args.starts_with(&["--format=pax".to_string(), "--no-mac-metadata".to_string(),])
            );
        }
        #[cfg(not(target_os = "macos"))]
        {
            assert_eq!(program, "tar");
        }
        assert!(args.contains(&".".to_string()));
        assert!(!args.contains(&".git".to_string()));
        assert!(!args.contains(&"file.txt".to_string()));

        match old_cli {
            Some(value) => std::env::set_var("CTX_HARNESS_SANDBOX_CLI_PATH", value),
            None => std::env::remove_var("CTX_HARNESS_SANDBOX_CLI_PATH"),
        }
    }

    #[test]
    fn parse_df_pk_line_extracts_capacity_sample() {
        let sample = parse_df_pk_line(
            "overlay 10485760 1024 7340032 1% /ctx/ws",
            "sandbox workspace volume",
            Path::new("/ctx/ws/worktrees"),
        )
        .expect("parse df output");
        assert_eq!(sample.label, "sandbox workspace volume");
        assert_eq!(sample.mount_point, "/ctx/ws");
        assert_eq!(sample.total_bytes, 10485760_u64 * 1024);
        assert_eq!(sample.free_bytes, 7340032_u64 * 1024);
    }

    #[test]
    fn disk_isolated_copy_budget_uses_source_size_plus_overhead() {
        assert_eq!(
            disk_isolated_copy_budget_bytes(512),
            512 + DISK_ISOLATED_COPY_OVERHEAD_BYTES
        );
    }

    #[test]
    fn disk_isolated_storage_admission_denial_mentions_task_worktree() {
        let required_bytes = storage_guard::storage_admission_required_bytes(
            disk_isolated_copy_budget_bytes(512 * 1024 * 1024),
        );
        let err = storage_guard::check_storage_admission(
            StorageAdmissionOperation::DiskIsolatedWorktreeMaterialization,
            required_bytes,
            false,
            &[
                StorageAdmissionSample {
                    label: "CTX data root".to_string(),
                    path: "/ctx-data".to_string(),
                    mount_point: "/".to_string(),
                    free_bytes: 5 * 1024 * 1024 * 1024,
                    total_bytes: 20 * 1024 * 1024 * 1024,
                    reserve_file_eligible: true,
                },
                StorageAdmissionSample {
                    label: "sandbox workspace volume".to_string(),
                    path: "/ctx/ws/worktrees".to_string(),
                    mount_point: "/ctx/ws".to_string(),
                    free_bytes: 256 * 1024 * 1024,
                    total_bytes: 20 * 1024 * 1024 * 1024,
                    reserve_file_eligible: false,
                },
            ],
        )
        .expect_err("low sandbox capacity should deny admission");
        assert!(err.to_string().contains("isolated task worktree"));
        assert!(err.to_string().contains("sandbox workspace volume"));
    }
}
