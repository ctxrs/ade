use super::*;
use std::borrow::Cow;

use ctx_core::ids::WorktreeId;

const AVF_EGRESS_PROXY_GUEST_PATH: &str = "/usr/local/bin/ctx-egress-proxy";
const AVF_EGRESS_PROXY_CONFIG_GUEST_PATH: &str = "/var/lib/ctx/egress-proxy.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct AppliedContainerNetworkPolicy {
    pub(super) egress_guard: bool,
}

pub(super) async fn apply_avf_linux_network_policy(
    data_root: &Path,
    workspace_id: WorkspaceId,
    worktree_id: WorktreeId,
    guest_worktree_root: &Path,
    settings: &ContainerExecutionSettings,
    daemon_host: &str,
    daemon_port: u16,
) -> Result<AppliedContainerNetworkPolicy> {
    if avf_linux_vm::workspace_vm_state(data_root, workspace_id)?.simulated {
        tracing::info!(
            workspace_id = %workspace_id.0,
            worktree_id = %worktree_id.0,
            "AVF workspace VM is simulated; skipping guest network-policy enforcement"
        );
        return Ok(AppliedContainerNetworkPolicy {
            egress_guard: false,
        });
    }
    if matches!(settings.network_mode, ContainerNetworkMode::All) {
        return transition_avf_to_unrestricted_network(
            data_root,
            workspace_id,
            worktree_id,
            guest_worktree_root,
        )
        .await;
    }

    transition_avf_to_restricted_network(
        data_root,
        workspace_id,
        worktree_id,
        guest_worktree_root,
        settings,
        daemon_host,
        daemon_port,
    )
    .await
}

pub(super) async fn apply_container_network_policy(
    data_root: &Path,
    workspace_id: WorkspaceId,
    name: &str,
    settings: &ContainerExecutionSettings,
    daemon_host: &str,
    daemon_port: u16,
) -> Result<AppliedContainerNetworkPolicy> {
    if matches!(settings.network_mode, ContainerNetworkMode::All) {
        return transition_to_unrestricted_network(data_root, name).await;
    }

    transition_to_restricted_network(
        data_root,
        workspace_id,
        name,
        settings,
        daemon_host,
        daemon_port,
    )
    .await
}

fn llm_only_proxy_allowlist_entries() -> Vec<String> {
    let mut entries: Vec<String> = network_allowlist::LLM_ALLOWLIST
        .iter()
        .filter_map(|entry| network_allowlist::normalize_allowlist_entry(entry))
        .collect();
    entries.sort();
    entries.dedup();
    entries
}

fn transparent_proxy_pid_file() -> Cow<'static, str> {
    std::env::var("CTX_EGRESS_PROXY_PID_FILE")
        .map(Cow::Owned)
        .unwrap_or_else(|_| Cow::Borrowed("/tmp/ctx-egress-proxy.pid"))
}

pub(super) fn transparent_proxy_policy(
    settings: &ContainerExecutionSettings,
) -> (ContainerNetworkMode, Vec<String>) {
    match settings.network_mode {
        // Use explicit allowlist mode for llm_only so policy is fully driven by daemon-side
        // config and does not depend on baked allowlist constants inside container images.
        ContainerNetworkMode::LlmOnly => (
            ContainerNetworkMode::Allowlist,
            llm_only_proxy_allowlist_entries(),
        ),
        ContainerNetworkMode::Allowlist => {
            (ContainerNetworkMode::Allowlist, settings.allowlist.clone())
        }
        ContainerNetworkMode::All => (ContainerNetworkMode::All, Vec::new()),
    }
}

async fn transition_to_unrestricted_network(
    data_root: &Path,
    name: &str,
) -> Result<AppliedContainerNetworkPolicy> {
    let stop_err = stop_transparent_proxy(data_root, name).await.err();
    let clear_err = clear_egress_guard(data_root, name).await.err();
    finalize_unrestricted_transition(stop_err, clear_err)
}

fn finalize_unrestricted_transition(
    stop_err: Option<anyhow::Error>,
    clear_err: Option<anyhow::Error>,
) -> Result<AppliedContainerNetworkPolicy> {
    let mut failures = Vec::new();
    if let Some(err) = stop_err {
        failures.push(format!("stop transparent proxy: {err:#}"));
    }
    if let Some(err) = clear_err {
        failures.push(format!("clear egress guard: {err:#}"));
    }
    if !failures.is_empty() {
        anyhow::bail!(
            "failed to tear down restricted container network policy: {}",
            failures.join("; ")
        );
    }
    Ok(AppliedContainerNetworkPolicy {
        egress_guard: false,
    })
}

async fn transition_avf_to_unrestricted_network(
    data_root: &Path,
    workspace_id: WorkspaceId,
    worktree_id: WorktreeId,
    guest_worktree_root: &Path,
) -> Result<AppliedContainerNetworkPolicy> {
    let stop_err =
        stop_avf_transparent_proxy(data_root, workspace_id, worktree_id, guest_worktree_root)
            .await
            .err();
    let clear_err =
        clear_avf_egress_guard(data_root, workspace_id, worktree_id, guest_worktree_root)
            .await
            .err();
    finalize_unrestricted_transition(stop_err, clear_err)
}

async fn transition_to_restricted_network(
    data_root: &Path,
    workspace_id: WorkspaceId,
    name: &str,
    settings: &ContainerExecutionSettings,
    daemon_host: &str,
    daemon_port: u16,
) -> Result<AppliedContainerNetworkPolicy> {
    // Prefer the in-image proxy binary (required for out-of-the-box behavior on macOS/Windows).
    // If the image doesn't have it, we also allow an explicit override via CTX_EGRESS_PROXY_PATH,
    // but restricted modes must not silently fall back to full network access.
    let proxy_bin = match ensure_egress_proxy_available(data_root, name).await {
        Ok(()) => EGRESS_PROXY_CONTAINER_PATH.to_string(),
        Err(img_err) => {
            // Optional escape hatch: allow a Linux proxy binary to be provided via the host
            // (it is bind-mounted into the container under ~/.ctx/runtimes/...).
            if std::env::var("CTX_EGRESS_PROXY_PATH").ok().is_some() {
                let host_bin = ensure_egress_proxy_binary(data_root).await?;
                host_bin.to_string_lossy().to_string()
            } else {
                return Err(img_err).context(
                    "restricted container networking requires ctx-egress-proxy in the container image",
                );
            }
        }
    };
    let (proxy_mode, proxy_allowlist) = transparent_proxy_policy(settings);
    let proxy_config = TransparentProxyConfig {
        listen: format!("127.0.0.1:{TRANSPARENT_PROXY_PORT}"),
        mode: proxy_mode,
        allowlist: proxy_allowlist,
        max_peek_bytes: 16 * 1024,
    };
    let config_path =
        write_transparent_proxy_config(&container_data_root(data_root, workspace_id), proxy_config)
            .await?;
    start_transparent_proxy(data_root, name, &PathBuf::from(proxy_bin), &config_path).await?;
    let egress_guard = configure_transparent_egress_guard(
        data_root,
        name,
        TRANSPARENT_PROXY_PORT,
        daemon_host,
        daemon_port,
    )
    .await?;
    Ok(AppliedContainerNetworkPolicy { egress_guard })
}

async fn transition_avf_to_restricted_network(
    data_root: &Path,
    workspace_id: WorkspaceId,
    worktree_id: WorktreeId,
    guest_worktree_root: &Path,
    settings: &ContainerExecutionSettings,
    daemon_host: &str,
    daemon_port: u16,
) -> Result<AppliedContainerNetworkPolicy> {
    ensure_avf_egress_proxy_available(data_root, workspace_id, worktree_id, guest_worktree_root)
        .await
        .context(
            "restricted AVF Linux networking requires iptables and ctx-egress-proxy in the guest runtime",
        )?;
    let (proxy_mode, proxy_allowlist) = transparent_proxy_policy(settings);
    let proxy_config = TransparentProxyConfig {
        listen: format!("127.0.0.1:{TRANSPARENT_PROXY_PORT}"),
        mode: proxy_mode,
        allowlist: proxy_allowlist,
        max_peek_bytes: 16 * 1024,
    };
    write_avf_transparent_proxy_config(
        data_root,
        workspace_id,
        worktree_id,
        guest_worktree_root,
        &proxy_config,
    )
    .await?;
    start_avf_transparent_proxy(data_root, workspace_id, worktree_id, guest_worktree_root).await?;
    let egress_guard = configure_avf_transparent_egress_guard(
        data_root,
        workspace_id,
        worktree_id,
        guest_worktree_root,
        daemon_host,
        daemon_port,
        TRANSPARENT_PROXY_PORT,
    )
    .await?;
    Ok(AppliedContainerNetworkPolicy { egress_guard })
}

async fn ensure_egress_proxy_binary(data_root: &Path) -> Result<PathBuf> {
    let runtime_root = proxy_runtime_root(data_root);
    fs::create_dir_all(&runtime_root).await?;
    let dest = proxy_runtime_path(data_root);
    let src = match std::env::var("CTX_EGRESS_PROXY_PATH") {
        Ok(path) => PathBuf::from(path),
        Err(_) => {
            anyhow::bail!(
                "missing CTX_EGRESS_PROXY_PATH; host-injected egress proxy requires an explicit Linux binary path"
            )
        }
    };
    if !src.exists() {
        anyhow::bail!("missing {EGRESS_PROXY_BINARY} binary at {}", src.display());
    }
    if src != dest {
        fs::copy(&src, &dest).await?;
    }
    let mut perms = fs::metadata(&dest).await?.permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&dest, perms).await?;
    Ok(dest)
}

async fn ensure_egress_proxy_available(data_root: &Path, container_name: &str) -> Result<()> {
    // Validate required tooling inside the container for restricted network modes.
    //
    // This is a hard requirement: without these, we cannot enforce allowlist/llm-only safely.
    let script = format!(
        "set -e; command -v iptables >/dev/null 2>&1; test -x '{EGRESS_PROXY_CONTAINER_PATH}'"
    );
    let mut cmd = podman_command(data_root)?;
    cmd.arg("exec")
        .arg("--user")
        .arg("0")
        .arg(container_name)
        .arg("sh")
        .arg("-c")
        .arg(script);
    let output = command_output_with_timeout(cmd, PODMAN_OP_TIMEOUT).await?;
    if output.status.success() {
        return Ok(());
    }
    anyhow::bail!(
        "container missing required egress tooling (iptables and/or {EGRESS_PROXY_CONTAINER_PATH}); status {}",
        output.status
    );
}

async fn write_transparent_proxy_config(
    root: &Path,
    config: TransparentProxyConfig,
) -> Result<PathBuf> {
    fs::create_dir_all(root).await?;
    let path = root.join(EGRESS_PROXY_CONFIG_NAME);
    let raw = serde_json::to_string_pretty(&config)?;
    let mut file = fs::File::create(&path).await?;
    file.write_all(raw.as_bytes()).await?;
    Ok(path)
}

async fn start_transparent_proxy(
    data_root: &Path,
    name: &str,
    bin_path: &Path,
    config_path: &Path,
) -> Result<()> {
    let bin = bin_path.to_string_lossy();
    let config = config_path.to_string_lossy();
    let pid_file = transparent_proxy_pid_file();
    let script = format!(
        r#"
set -e
pid_file="{pid_file}"
if [ -f "$pid_file" ]; then
  old_pid="$(cat "$pid_file" 2>/dev/null || true)"
  if [ -n "$old_pid" ]; then
    kill "$old_pid" || true
  fi
  rm -f "$pid_file"
fi
if command -v nohup >/dev/null 2>&1; then
  nohup '{bin}' --config '{config}' >/tmp/ctx-egress-proxy.log 2>&1 &
elif command -v setsid >/dev/null 2>&1; then
  setsid '{bin}' --config '{config}' >/tmp/ctx-egress-proxy.log 2>&1 &
else
  '{bin}' --config '{config}' >/tmp/ctx-egress-proxy.log 2>&1 &
fi
echo $! > "$pid_file"
exit 0
"#
    );
    let mut cmd = podman_command(data_root)?;
    cmd.arg("exec")
        .arg("--user")
        .arg("0")
        .arg(name)
        .arg("sh")
        .arg("-c")
        .arg(script);
    let output = command_output_with_timeout(cmd, PODMAN_OP_TIMEOUT).await?;
    if output.status.success() {
        Ok(())
    } else {
        anyhow::bail!(
            "failed to start transparent proxy (status: {})",
            output.status
        );
    }
}

async fn stop_transparent_proxy(data_root: &Path, name: &str) -> Result<()> {
    let pid_file = transparent_proxy_pid_file();
    let script = r#"
set -e
pid_file="{pid_file}"
if [ -f "$pid_file" ]; then
  old_pid="$(cat "$pid_file")"
  if [ -n "$old_pid" ]; then
    if kill -0 "$old_pid" 2>/dev/null; then
      if ! kill "$old_pid" 2>/dev/null; then
        if kill -0 "$old_pid" 2>/dev/null; then
          echo "failed to stop transparent proxy pid $old_pid" >&2
          exit 45
        fi
      fi
    fi
  fi
  rm -f "$pid_file"
fi
"#;
    let script = script.replace("{pid_file}", pid_file.as_ref());
    let mut cmd = podman_command(data_root)?;
    cmd.arg("exec")
        .arg("--user")
        .arg("0")
        .arg(name)
        .arg("sh")
        .arg("-c")
        .arg(script);
    let output = command_output_with_timeout(cmd, PODMAN_OP_TIMEOUT).await?;
    if output.status.success() {
        Ok(())
    } else {
        let combined = command_output_message(&output);
        if combined.is_empty() {
            anyhow::bail!(
                "failed to stop transparent proxy (status: {})",
                output.status
            );
        }
        anyhow::bail!("failed to stop transparent proxy: {combined}");
    }
}

async fn configure_transparent_egress_guard(
    data_root: &Path,
    name: &str,
    proxy_port: u16,
    daemon_host: &str,
    daemon_port: u16,
) -> Result<bool> {
    let script = format!(
        r#"
set -e
if ! command -v iptables >/dev/null 2>&1; then
  exit 43
fi
daemon_ip="$(getent hosts {daemon_host} | awk '{{print $1}}' | head -n1)"
if [ -z "$daemon_ip" ]; then
  exit 44
fi
iptables -t nat -F OUTPUT || true
iptables -F OUTPUT || true
iptables -P OUTPUT DROP
iptables -A OUTPUT -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT
iptables -A OUTPUT -d 127.0.0.1/8 -j ACCEPT
iptables -A OUTPUT -o lo -j ACCEPT
iptables -A OUTPUT -p udp --dport 53 -j ACCEPT
iptables -A OUTPUT -p tcp --dport 53 -j ACCEPT
iptables -A OUTPUT -d "$daemon_ip" -p tcp --dport {daemon_port} -j ACCEPT
iptables -A OUTPUT -m owner --uid-owner 0 -j ACCEPT
iptables -t nat -A OUTPUT -m owner --uid-owner 0 -j RETURN
iptables -t nat -A OUTPUT -p tcp --dport 80 -j REDIRECT --to-ports {proxy_port}
iptables -t nat -A OUTPUT -p tcp --dport 443 -j REDIRECT --to-ports {proxy_port}
exit 0
"#
    );
    let mut cmd = podman_command(data_root)?;
    cmd.arg("exec")
        .arg("--user")
        .arg("0")
        .arg(name)
        .arg("sh")
        .arg("-c")
        .arg(script);
    let output = command_output_with_timeout(cmd, PODMAN_OP_TIMEOUT).await?;
    if output.status.success() {
        return Ok(true);
    }
    if let Some(code) = output.status.code() {
        if code == 43 {
            anyhow::bail!("iptables missing in harness container");
        }
        if code == 44 {
            anyhow::bail!("daemon host not resolvable inside harness container");
        }
    }
    anyhow::bail!(
        "failed to configure egress guard (status: {})",
        output.status
    );
}

async fn clear_egress_guard(data_root: &Path, name: &str) -> Result<()> {
    let script = r#"
set -e
if ! command -v iptables >/dev/null 2>&1; then
  exit 0
fi
iptables -t nat -F OUTPUT
iptables -F OUTPUT
iptables -P OUTPUT ACCEPT
"#;
    let mut cmd = podman_command(data_root)?;
    cmd.arg("exec")
        .arg("--user")
        .arg("0")
        .arg(name)
        .arg("sh")
        .arg("-c")
        .arg(script);
    let output = command_output_with_timeout(cmd, PODMAN_OP_TIMEOUT).await?;
    if output.status.success() {
        Ok(())
    } else {
        let combined = command_output_message(&output);
        if combined.is_empty() {
            anyhow::bail!("failed to clear egress guard (status: {})", output.status);
        }
        anyhow::bail!("failed to clear egress guard: {combined}");
    }
}

async fn run_avf_root_shell_capture(
    data_root: &Path,
    workspace_id: WorkspaceId,
    worktree_id: WorktreeId,
    guest_worktree_root: &Path,
    script: &str,
) -> Result<std::process::Output> {
    run_avf_linux_guest_exec_capture(
        data_root,
        workspace_id,
        worktree_id,
        guest_worktree_root,
        "sh",
        &[String::from("-lc"), script.to_string()],
        &HashMap::new(),
        Some("root"),
        false,
    )
    .await
}

async fn ensure_avf_egress_proxy_available(
    data_root: &Path,
    workspace_id: WorkspaceId,
    worktree_id: WorktreeId,
    guest_worktree_root: &Path,
) -> Result<()> {
    let output = run_avf_root_shell_capture(
        data_root,
        workspace_id,
        worktree_id,
        guest_worktree_root,
        &format!(
            "set -e; command -v iptables >/dev/null 2>&1; test -x '{AVF_EGRESS_PROXY_GUEST_PATH}'"
        ),
    )
    .await?;
    if output.status.success() {
        return Ok(());
    }
    anyhow::bail!(
        "AVF guest is missing required egress tooling (iptables and/or {AVF_EGRESS_PROXY_GUEST_PATH}); status {}",
        output.status
    );
}

async fn write_avf_transparent_proxy_config(
    data_root: &Path,
    workspace_id: WorkspaceId,
    worktree_id: WorktreeId,
    guest_worktree_root: &Path,
    config: &TransparentProxyConfig,
) -> Result<()> {
    let raw = serde_json::to_string_pretty(config)?;
    let script = format!(
        "set -e\nmkdir -p /var/lib/ctx\ncat > '{path}' <<'CTX_PROXY_EOF'\n{raw}\nCTX_PROXY_EOF\nchmod 0644 '{path}'\n",
        path = AVF_EGRESS_PROXY_CONFIG_GUEST_PATH,
        raw = raw,
    );
    let output = run_avf_root_shell_capture(
        data_root,
        workspace_id,
        worktree_id,
        guest_worktree_root,
        &script,
    )
    .await?;
    if output.status.success() {
        return Ok(());
    }
    anyhow::bail!(
        "failed to write AVF guest egress proxy config (status: {})",
        output.status
    );
}

async fn start_avf_transparent_proxy(
    data_root: &Path,
    workspace_id: WorkspaceId,
    worktree_id: WorktreeId,
    guest_worktree_root: &Path,
) -> Result<()> {
    let pid_file = transparent_proxy_pid_file();
    let script = format!(
        r#"
set -e
pid_file="{pid_file}"
if [ -f "$pid_file" ]; then
  old_pid="$(cat "$pid_file" 2>/dev/null || true)"
  if [ -n "$old_pid" ]; then
    kill "$old_pid" || true
  fi
  rm -f "$pid_file"
fi
if command -v nohup >/dev/null 2>&1; then
  nohup '{bin}' --config '{config}' >/tmp/ctx-egress-proxy.log 2>&1 &
elif command -v setsid >/dev/null 2>&1; then
  setsid '{bin}' --config '{config}' >/tmp/ctx-egress-proxy.log 2>&1 &
else
  '{bin}' --config '{config}' >/tmp/ctx-egress-proxy.log 2>&1 &
fi
echo $! > "$pid_file"
exit 0
"#,
        pid_file = pid_file,
        bin = AVF_EGRESS_PROXY_GUEST_PATH,
        config = AVF_EGRESS_PROXY_CONFIG_GUEST_PATH,
    );
    let output = run_avf_root_shell_capture(
        data_root,
        workspace_id,
        worktree_id,
        guest_worktree_root,
        &script,
    )
    .await?;
    if output.status.success() {
        return Ok(());
    }
    anyhow::bail!(
        "failed to start AVF guest transparent proxy (status: {})",
        output.status
    );
}

async fn stop_avf_transparent_proxy(
    data_root: &Path,
    workspace_id: WorkspaceId,
    worktree_id: WorktreeId,
    guest_worktree_root: &Path,
) -> Result<()> {
    let pid_file = transparent_proxy_pid_file();
    let script = format!(
        r#"
set -e
pid_file="{pid_file}"
if [ -f "$pid_file" ]; then
  old_pid="$(cat "$pid_file")"
  if [ -n "$old_pid" ]; then
    if kill -0 "$old_pid" 2>/dev/null; then
      if ! kill "$old_pid" 2>/dev/null; then
        if kill -0 "$old_pid" 2>/dev/null; then
          echo "failed to stop transparent proxy pid $old_pid" >&2
          exit 45
        fi
      fi
    fi
  fi
  rm -f "$pid_file"
fi
"#,
        pid_file = pid_file,
    );
    let output = run_avf_root_shell_capture(
        data_root,
        workspace_id,
        worktree_id,
        guest_worktree_root,
        &script,
    )
    .await?;
    if output.status.success() {
        return Ok(());
    }
    let combined = command_output_message(&output);
    if combined.is_empty() {
        anyhow::bail!(
            "failed to stop AVF guest transparent proxy (status: {})",
            output.status
        );
    }
    anyhow::bail!("failed to stop AVF guest transparent proxy: {combined}");
}

async fn configure_avf_transparent_egress_guard(
    data_root: &Path,
    workspace_id: WorkspaceId,
    worktree_id: WorktreeId,
    guest_worktree_root: &Path,
    daemon_host: &str,
    daemon_port: u16,
    proxy_port: u16,
) -> Result<bool> {
    let script = format!(
        r#"
set -e
if ! command -v iptables >/dev/null 2>&1; then
  exit 43
fi
daemon_ip="$(getent hosts {daemon_host} | awk '{{print $1}}' | head -n1)"
if [ -z "$daemon_ip" ]; then
  exit 44
fi
iptables -t nat -F OUTPUT || true
iptables -F OUTPUT || true
iptables -P OUTPUT DROP
iptables -A OUTPUT -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT
iptables -A OUTPUT -d 127.0.0.1/8 -j ACCEPT
iptables -A OUTPUT -o lo -j ACCEPT
iptables -A OUTPUT -p udp --dport 53 -j ACCEPT
iptables -A OUTPUT -p tcp --dport 53 -j ACCEPT
iptables -A OUTPUT -d "$daemon_ip" -p tcp --dport {daemon_port} -j ACCEPT
iptables -A OUTPUT -m owner --uid-owner 0 -j ACCEPT
iptables -t nat -A OUTPUT -m owner --uid-owner 0 -j RETURN
iptables -t nat -A OUTPUT -p tcp --dport 80 -j REDIRECT --to-ports {proxy_port}
iptables -t nat -A OUTPUT -p tcp --dport 443 -j REDIRECT --to-ports {proxy_port}
exit 0
"#,
        daemon_host = daemon_host,
        daemon_port = daemon_port,
        proxy_port = proxy_port,
    );
    let output = run_avf_root_shell_capture(
        data_root,
        workspace_id,
        worktree_id,
        guest_worktree_root,
        &script,
    )
    .await?;
    if output.status.success() {
        return Ok(true);
    }
    if let Some(code) = output.status.code() {
        if code == 43 {
            anyhow::bail!("iptables missing in AVF guest");
        }
        if code == 44 {
            anyhow::bail!("daemon host not resolvable inside AVF guest");
        }
    }
    anyhow::bail!(
        "failed to configure AVF guest egress guard (status: {})",
        output.status
    );
}

async fn clear_avf_egress_guard(
    data_root: &Path,
    workspace_id: WorkspaceId,
    worktree_id: WorktreeId,
    guest_worktree_root: &Path,
) -> Result<()> {
    let script = r#"
set -e
if ! command -v iptables >/dev/null 2>&1; then
  exit 0
fi
iptables -t nat -F OUTPUT
iptables -F OUTPUT
iptables -P OUTPUT ACCEPT
"#;
    let output = run_avf_root_shell_capture(
        data_root,
        workspace_id,
        worktree_id,
        guest_worktree_root,
        script,
    )
    .await?;
    if output.status.success() {
        return Ok(());
    }
    let combined = command_output_message(&output);
    if combined.is_empty() {
        anyhow::bail!(
            "failed to clear AVF guest egress guard (status: {})",
            output.status
        );
    }
    anyhow::bail!("failed to clear AVF guest egress guard: {combined}");
}
