use super::*;

pub(crate) fn helper_path() -> Result<PathBuf> {
    if !cfg!(target_os = "macos") && !cfg!(test) {
        bail!("AVF Linux VM runtime is only supported on macOS");
    }
    let value = std::env::var(AVF_LINUX_HELPER_PATH_ENV)
        .context("CTX_AVF_LINUX_HELPER_PATH is required for the AVF Linux VM backend")?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("CTX_AVF_LINUX_HELPER_PATH is empty");
    }
    let path = PathBuf::from(trimmed);
    if !path.is_file() {
        bail!("AVF Linux helper does not exist at {}", path.display());
    }
    Ok(path)
}

fn invoke_helper_json<T>(args: &[&str]) -> Result<T>
where
    T: DeserializeOwned,
{
    let helper = helper_path()?;
    let output = std::process::Command::new(&helper)
        .args(args)
        .output()
        .with_context(|| format!("spawning AVF Linux helper at {}", helper.display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let combined = format!("{stderr}\n{stdout}").trim().to_string();
        if combined.is_empty() {
            bail!("AVF Linux helper exited with status {}", output.status);
        }
        bail!("AVF Linux helper failed: {combined}");
    }
    serde_json::from_slice(&output.stdout).with_context(|| {
        let joined = args.join(" ");
        format!("parsing AVF Linux helper response for `{joined}`")
    })
}

pub(crate) fn probe_helper() -> Result<AvfLinuxHelperProbe> {
    invoke_helper_json(&["probe"])
}

pub(crate) fn prepare_runtime_layout(data_root: &Path) -> Result<AvfLinuxRuntimeLayout> {
    invoke_helper_json(&["prepare-runtime-layout", &data_root.to_string_lossy()])
}

pub(crate) fn workspace_vm_data_root(data_root: &Path, _workspace_id: WorkspaceId) -> PathBuf {
    data_root.to_path_buf()
}

pub(crate) fn shared_vm_state(data_root: &Path) -> Result<AvfLinuxSharedVmState> {
    invoke_helper_json(&["workspace-vm-state", &data_root.to_string_lossy()])
}

pub(crate) fn workspace_vm_state(
    data_root: &Path,
    workspace_id: WorkspaceId,
) -> Result<AvfLinuxSharedVmState> {
    let vm_data_root = workspace_vm_data_root(data_root, workspace_id);
    shared_vm_state(&vm_data_root)
}

pub(crate) fn prepare_guest_worktree(
    data_root: &Path,
    workspace_id: WorkspaceId,
    worktree_id: WorktreeId,
    host_workspace_root: &Path,
    base_commit_sha: &str,
    branch_name: &str,
) -> Result<AvfLinuxGuestWorktree> {
    let vm_data_root = workspace_vm_data_root(data_root, workspace_id);
    invoke_helper_json(&[
        "prepare-guest-worktree",
        &vm_data_root.to_string_lossy(),
        &workspace_id.0.to_string(),
        &worktree_id.0.to_string(),
        &host_workspace_root.to_string_lossy(),
        base_commit_sha,
        branch_name,
    ])
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_guest_exec_command(
    data_root: &Path,
    workspace_id: WorkspaceId,
    _worktree_id: WorktreeId,
    cwd: &Path,
    command: &str,
    args: &[String],
    env: &HashMap<String, String>,
    user: Option<&str>,
    pty: bool,
) -> Result<tokio::process::Command> {
    let mut child = super::super::sandbox_container_command(data_root)?;
    child.arg("exec").arg("--interactive");
    if pty {
        child.arg("--tty");
    }
    if let Some(user) = user.map(str::trim).filter(|value| !value.is_empty()) {
        child.arg("--user").arg(user);
    }
    child
        .arg("--workdir")
        .arg(cwd)
        .arg(format!("ctx-harness-{}", workspace_id.0));
    let mut env_pairs = env.iter().collect::<Vec<_>>();
    env_pairs.sort_by(|(left, _), (right, _)| left.cmp(right));
    for (key, value) in env_pairs {
        child.arg("--env").arg(format!("{key}={value}"));
    }
    child.arg(command);
    child.args(args);
    Ok(child)
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_guest_exec_capture(
    data_root: &Path,
    workspace_id: WorkspaceId,
    worktree_id: WorktreeId,
    cwd: &Path,
    command: &str,
    args: &[String],
    env: &HashMap<String, String>,
    user: Option<&str>,
    pty: bool,
) -> Result<std::process::Output> {
    let mut child = build_guest_exec_command(
        data_root,
        workspace_id,
        worktree_id,
        cwd,
        command,
        args,
        env,
        user,
        pty,
    )?;
    child.stdin(std::process::Stdio::null());
    child.output().await.with_context(|| {
        format!(
            "running AVF guest exec `{command}` for workspace {} worktree {}",
            workspace_id.0, worktree_id.0
        )
    })
}

pub(crate) fn start_shared_vm(
    data_root: &Path,
    runtime: &AvfLinuxGuestRuntime,
) -> Result<AvfLinuxSharedVmState> {
    invoke_helper_json(&[
        "start-workspace-vm",
        &data_root.to_string_lossy(),
        &runtime.runtime_root.to_string_lossy(),
        &runtime.rootfs_image.to_string_lossy(),
        &runtime.kernel_path.to_string_lossy(),
        &runtime.initrd_path.to_string_lossy(),
        &runtime.version,
    ])
}

pub(crate) fn start_workspace_vm(
    data_root: &Path,
    workspace_id: WorkspaceId,
    runtime: &AvfLinuxGuestRuntime,
) -> Result<AvfLinuxSharedVmState> {
    let vm_data_root = workspace_vm_data_root(data_root, workspace_id);
    start_shared_vm(&vm_data_root, runtime)
}

pub(crate) fn stop_shared_vm(data_root: &Path) -> Result<AvfLinuxSharedVmState> {
    invoke_helper_json(&["stop-workspace-vm", &data_root.to_string_lossy()])
}

#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn stop_workspace_vm(
    data_root: &Path,
    workspace_id: WorkspaceId,
) -> Result<AvfLinuxSharedVmState> {
    let vm_data_root = workspace_vm_data_root(data_root, workspace_id);
    stop_shared_vm(&vm_data_root)
}
