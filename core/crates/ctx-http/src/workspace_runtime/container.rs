use super::*;

pub(crate) const CONTAINER_TERMINAL_USER: &str = "ctx-user";
pub(crate) const CONTAINER_TERMINAL_HOME: &str = "/home/example-user";

const CONTAINER_HOSTNAME_SUFFIX: &str = "-container";
const MAX_CONTAINER_HOSTNAME_LEN: usize = 63;
const CONTAINER_TERMINAL_SUDO_MISSING_SENTINEL: &str = "__CTX_CONTAINER_TERMINAL_SUDO_MISSING__";

pub(super) fn container_data_root(data_root: &Path, workspace_id: WorkspaceId) -> PathBuf {
    data_root
        .join("containers")
        .join("workspaces")
        .join(workspace_id.0.to_string())
        .join("data")
}

pub(super) struct MountPlan {
    pub(super) mounts: Vec<String>,
    pub(super) external_mounts: HashSet<String>,
}

fn volume_mount(name: &str, dst: &str, read_only: bool) -> String {
    let mode = if read_only { "ro" } else { "rw" };
    format!("type=volume,src={name},dst={dst},{mode}")
}

pub(super) fn build_mounts(
    data_root: &Path,
    workspace: &Workspace,
    _worktree: Option<&Worktree>,
    settings: &ContainerExecutionSettings,
) -> MountPlan {
    let mut mounts = Vec::new();
    let mut external_mounts = HashSet::new();
    let workspace_root = PathBuf::from(&workspace.root_path);

    let worktrees_root = worktrees_root(data_root).join(workspace.id.0.to_string());
    if matches!(settings.mount_mode, ContainerMountMode::DiskIsolated) {
        let vol_name = format!("ctx-ws-{}", workspace.id.0);
        mounts.push(volume_mount(&vol_name, CTX_CONTAINER_WORKSPACE_ROOT, false));
    } else {
        ensure_dir(&workspace_root);
        mounts.push(bind_mount(&workspace_root, &workspace_root, false));
        ensure_dir(&worktrees_root);
        mounts.push(bind_mount(&worktrees_root, &worktrees_root, false));
    }

    let container_data = container_data_root(data_root, workspace.id);
    ensure_dir(&container_data);
    mounts.push(bind_mount(&container_data, &container_data, false));

    let agent_servers = data_root.join("providers").join("agent-servers");
    ensure_dir(&agent_servers);
    mounts.push(bind_mount(&agent_servers, &agent_servers, true));

    let runtimes = data_root.join("runtimes");
    ensure_dir(&runtimes);
    mounts.push(bind_mount(&runtimes, &runtimes, true));

    let vcs_hooks = crate::vcs_hooks::vcs_hooks_root(data_root);
    ensure_dir(&vcs_hooks);
    mounts.push(bind_mount(&vcs_hooks, &vcs_hooks, false));
    external_mounts.insert(vcs_hooks.to_string_lossy().to_string());

    if let Ok(raw) = std::env::var("CTX_BUNDLE_DIR") {
        let bundle_dir = PathBuf::from(raw.trim());
        if bundle_dir.exists() {
            if should_mount_bundle_dir_in_container(&bundle_dir) {
                mounts.push(bind_mount(&bundle_dir, &bundle_dir, true));
            } else {
                tracing::info!(
                    "skipping CTX_BUNDLE_DIR container mount (path not shareable by runtime): {}",
                    bundle_dir.display()
                );
            }
        }
    }

    MountPlan {
        mounts,
        external_mounts,
    }
}

pub(super) fn should_mount_bundle_dir_in_container(bundle_dir: &Path) -> bool {
    if cfg!(target_os = "linux") {
        return true;
    }
    if cfg!(target_os = "macos") || cfg!(target_os = "windows") {
        tracing::debug!(
            "sandbox-machine runtime cannot bind-mount CTX_BUNDLE_DIR host paths into guest containers: {}",
            bundle_dir.display()
        );
        return false;
    }
    true
}

pub(super) fn bind_mount(src: &Path, dst: &Path, read_only: bool) -> String {
    let mode = if read_only { "ro" } else { "rw" };
    format!(
        "type=bind,src={},dst={},{}",
        src.to_string_lossy(),
        dst.to_string_lossy(),
        mode
    )
}

fn ensure_dir(path: &Path) {
    let _ = std::fs::create_dir_all(path);
}

pub(super) fn rewrite_daemon_url_for_container(daemon_url: &str, host: &str) -> String {
    if let Ok(mut url) = Url::parse(daemon_url) {
        let _ = url.set_host(Some(host));
        return url.to_string();
    }
    daemon_url.to_string()
}

pub(crate) const AVF_GUEST_HOST_GATEWAY: &str = "192.168.64.1";

pub(super) fn rewrite_daemon_url_for_avf_guest(daemon_url: &str) -> String {
    rewrite_daemon_url_for_container(daemon_url, AVF_GUEST_HOST_GATEWAY)
}

pub(crate) fn workspace_container_hostname(workspace: &Workspace) -> String {
    let max_base_len = MAX_CONTAINER_HOSTNAME_LEN - CONTAINER_HOSTNAME_SUFFIX.len();
    let mut slug = String::with_capacity(workspace.name.len().min(max_base_len));
    let mut last_was_dash = false;
    for ch in workspace.name.trim().chars() {
        let normalized = if ch.is_ascii_alphanumeric() {
            Some(ch.to_ascii_lowercase())
        } else {
            None
        };
        match normalized {
            Some(value) => {
                slug.push(value);
                last_was_dash = false;
            }
            None if !slug.is_empty() && !last_was_dash => {
                slug.push('-');
                last_was_dash = true;
            }
            None => {}
        }
        if slug.len() >= max_base_len {
            break;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        slug.push_str("workspace");
    }
    if slug.len() > max_base_len {
        slug.truncate(max_base_len);
        while slug.ends_with('-') {
            slug.pop();
        }
        if slug.is_empty() {
            slug.push_str("workspace");
            slug.truncate(max_base_len);
        }
    }
    format!("{slug}{CONTAINER_HOSTNAME_SUFFIX}")
}

pub(super) fn daemon_port_from_url(daemon_url: &str) -> Option<u16> {
    Url::parse(daemon_url).ok()?.port_or_known_default()
}

pub(super) fn proxy_runtime_root(data_root: &Path) -> PathBuf {
    data_root.join("runtimes").join(EGRESS_PROXY_RUNTIME_ID)
}

pub(super) fn proxy_runtime_path(data_root: &Path) -> PathBuf {
    proxy_runtime_root(data_root).join(EGRESS_PROXY_BINARY)
}

pub(super) fn sandbox_machine_required() -> bool {
    cfg!(target_os = "macos") || cfg!(target_os = "windows")
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct SandboxInspectContainer {
    #[serde(default)]
    mounts: Vec<SandboxInspectMount>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct SandboxInspectMount {
    #[serde(rename = "Type")]
    mount_type: Option<String>,
    name: Option<String>,
    destination: Option<String>,
}

pub(super) async fn verify_disk_isolated_container_mounts(
    data_root: &Path,
    workspace: &Workspace,
    container_name: &str,
) -> Result<()> {
    let mut cmd = sandbox_container_command(data_root)?;
    cmd.arg("inspect").arg(container_name);
    let out = command_output_with_timeout(cmd, SANDBOX_OP_TIMEOUT).await?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let combined = format!("{stderr}\n{stdout}").trim().to_string();
        if combined.is_empty() {
            anyhow::bail!(
                "container inspect failed for {container_name} (status: {})",
                out.status
            );
        }
        anyhow::bail!("container inspect failed for {container_name}: {combined}");
    }

    let inspected: Vec<SandboxInspectContainer> = serde_json::from_slice(&out.stdout)
        .context("failed to parse container inspect output as JSON")?;
    let container = inspected
        .into_iter()
        .next()
        .context("container inspect returned empty output")?;

    let expected_vol = format!("ctx-ws-{}", workspace.id.0);
    let has_ws_volume = container.mounts.iter().any(|m| {
        m.mount_type.as_deref() == Some("volume")
            && m.destination.as_deref() == Some(CTX_CONTAINER_WORKSPACE_ROOT)
            && m.name.as_deref() == Some(expected_vol.as_str())
    });
    if !has_ws_volume {
        anyhow::bail!(
            "disk-isolated container {container_name} is missing expected volume mount: volume {expected_vol} -> {}",
            CTX_CONTAINER_WORKSPACE_ROOT
        );
    }

    let host_workspace_root = workspace.root_path.trim();
    let host_worktrees_root = worktrees_root(data_root)
        .join(workspace.id.0.to_string())
        .to_string_lossy()
        .to_string();
    let has_host_bind = container.mounts.iter().any(|m| {
        m.mount_type.as_deref() == Some("bind")
            && (m.destination.as_deref() == Some(host_workspace_root)
                || m.destination.as_deref() == Some(host_worktrees_root.as_str()))
    });
    if has_host_bind {
        anyhow::bail!(
            "disk-isolated container {container_name} unexpectedly bind-mounted host workspace/worktrees"
        );
    }

    Ok(())
}

#[cfg(unix)]
pub(super) fn current_container_uid_gid() -> Option<(u32, u32)> {
    let uid = unsafe { libc::geteuid() };
    let gid = unsafe { libc::getegid() };
    Some((uid, gid))
}

#[cfg(not(unix))]
pub(super) fn current_container_uid_gid() -> Option<(u32, u32)> {
    None
}

#[cfg(unix)]
pub(super) fn container_user() -> Option<String> {
    current_container_uid_gid().map(|(uid, gid)| format!("{uid}:{gid}"))
}

#[cfg(not(unix))]
pub(super) fn container_user() -> Option<String> {
    None
}

pub(super) fn container_terminal_identity_missing_sudo(err: &anyhow::Error) -> bool {
    err.to_string()
        .contains(CONTAINER_TERMINAL_SUDO_MISSING_SENTINEL)
}

pub(super) async fn sync_container_terminal_identity(
    data_root: &Path,
    container_name: &str,
) -> Result<()> {
    let mut cmd = sandbox_container_command(data_root)?;
    cmd.arg("exec")
        .arg("--user")
        .arg("0")
        .arg("--env")
        .arg(format!(
            "CTX_CONTAINER_TERMINAL_USER={CONTAINER_TERMINAL_USER}"
        ))
        .arg("--env")
        .arg(format!(
            "CTX_CONTAINER_TERMINAL_HOME={CONTAINER_TERMINAL_HOME}"
        ));
    if let Some((uid, gid)) = current_container_uid_gid() {
        cmd.arg("--env")
            .arg(format!("CTX_CONTAINER_TERMINAL_UID={uid}"))
            .arg("--env")
            .arg(format!("CTX_CONTAINER_TERMINAL_GID={gid}"));
    }
    cmd.arg(container_name)
        .arg("/bin/sh")
        .arg("-lc")
        .arg(
            "set -eu\n\
user=\"$CTX_CONTAINER_TERMINAL_USER\"\n\
home=\"$CTX_CONTAINER_TERMINAL_HOME\"\n\
shell=\"/bin/bash\"\n\
uid=\"${CTX_CONTAINER_TERMINAL_UID:-}\"\n\
gid=\"${CTX_CONTAINER_TERMINAL_GID:-}\"\n\
if [ ! -x /usr/bin/sudo ]; then\n\
  echo \"__CTX_CONTAINER_TERMINAL_SUDO_MISSING__\" >&2\n\
  exit 91\n\
fi\n\
mkdir -p \"$home\"\n\
if [ -n \"$gid\" ]; then\n\
  group_tmp=\"$(mktemp)\"\n\
  awk -F: -v group=\"$user\" -v gid=\"$gid\" '\n\
BEGIN { updated = 0 }\n\
$1 == group { print group \":x:\" gid \":\"; updated = 1; next }\n\
{ print }\n\
END { if (!updated) print group \":x:\" gid \":\" }\n\
' /etc/group > \"$group_tmp\"\n\
  cat \"$group_tmp\" > /etc/group\n\
  rm -f \"$group_tmp\"\n\
fi\n\
if [ -n \"$uid\" ] && [ -n \"$gid\" ]; then\n\
  passwd_tmp=\"$(mktemp)\"\n\
  awk -F: -v user=\"$user\" -v uid=\"$uid\" -v gid=\"$gid\" -v home=\"$home\" -v shell=\"$shell\" '\n\
BEGIN { updated = 0 }\n\
$1 == user { print user \":x:\" uid \":\" gid \"::\" home \":\" shell; updated = 1; next }\n\
{ print }\n\
END { if (!updated) print user \":x:\" uid \":\" gid \"::\" home \":\" shell }\n\
' /etc/passwd > \"$passwd_tmp\"\n\
  cat \"$passwd_tmp\" > /etc/passwd\n\
  rm -f \"$passwd_tmp\"\n\
  chown \"$uid:$gid\" \"$home\"\n\
fi\n\
install -d -m 0755 /etc/sudoers.d\n\
printf '%s ALL=(ALL) NOPASSWD:ALL\\n' \"$user\" > \"/etc/sudoers.d/$user\"\n\
chmod 0440 \"/etc/sudoers.d/$user\"\n",
        );
    let out = command_output_with_timeout(cmd, SANDBOX_OP_TIMEOUT).await?;
    if out.status.success() {
        return Ok(());
    }
    let combined = command_output_message(&out);
    if combined.is_empty() {
        anyhow::bail!(
            "container terminal identity sync failed for {container_name} (status: {})",
            out.status
        );
    }
    anyhow::bail!("container terminal identity sync failed for {container_name}: {combined}");
}

pub(super) fn should_use_keep_id_userns() -> bool {
    cfg!(target_os = "linux")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn workspace_named(name: &str) -> Workspace {
        Workspace {
            id: WorkspaceId::new(),
            name: name.to_string(),
            root_path: "/tmp/workspace".to_string(),
            created_at: Utc::now(),
            vcs_kind: None,
        }
    }

    #[test]
    fn workspace_container_hostname_uses_workspace_name_slug() {
        let workspace = workspace_named("ctx-monorepo");
        assert_eq!(
            workspace_container_hostname(&workspace),
            "ctx-monorepo-container"
        );
    }

    #[test]
    fn workspace_container_hostname_sanitizes_and_bounds_length() {
        let workspace = workspace_named("  Ctx Monorepo !!! Alpha Beta Gamma Delta Epsilon Zeta ");
        let hostname = workspace_container_hostname(&workspace);
        assert!(hostname.ends_with("-container"));
        assert!(hostname.len() <= MAX_CONTAINER_HOSTNAME_LEN);
        assert_eq!(
            hostname,
            "ctx-monorepo-alpha-beta-gamma-delta-epsilon-zeta-container"
        );
    }

    #[test]
    fn workspace_container_hostname_falls_back_when_name_is_unusable() {
        let workspace = workspace_named("___");
        assert_eq!(
            workspace_container_hostname(&workspace),
            "workspace-container"
        );
    }
}
