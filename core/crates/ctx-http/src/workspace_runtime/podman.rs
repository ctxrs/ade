use super::*;

fn podman_available(data_root: &Path) -> bool {
    if cfg!(test) {
        if let Ok(value) = std::env::var("CTX_TEST_PODMAN_AVAILABLE") {
            let value = value.trim().to_ascii_lowercase();
            return matches!(value.as_str(), "1" | "true" | "yes" | "y");
        }
    }
    #[cfg(target_os = "macos")]
    if super::avf_linux_runtime_available() && super::avf_linux_vm::helper_path().is_ok() {
        return true;
    }
    podman_binary_path(data_root).is_some()
}

pub(super) fn podman_binary_path(data_root: &Path) -> Option<PathBuf> {
    if let Ok(raw) = std::env::var(PODMAN_PATH_ENV) {
        let path = PathBuf::from(raw.trim());
        if path.exists() {
            return Some(path);
        }
    }
    if let Some(bundled) = bundled_assets::bundled_podman_runtime() {
        return Some(bundled.bin);
    }
    if let Some(source) = managed_podman_runtime_source() {
        let managed_path = managed_podman_runtime_bin_path(data_root, &source);
        if managed_path.exists() {
            return Some(managed_path);
        }
    }
    None
}

#[derive(Debug, Clone)]
pub(crate) struct PodmanInvocation {
    pub(crate) bin: PathBuf,
    pub(crate) env: HashMap<String, String>,
}

pub(crate) fn podman_invocation(data_root: &Path) -> Result<PodmanInvocation> {
    let bin = podman_binary_path(data_root)
        .ok_or_else(|| anyhow::anyhow!("podman binary unavailable"))?;
    let env = podman_env_for_data_root(data_root)?;
    Ok(PodmanInvocation { bin, env })
}

pub(crate) fn podman_env_for_data_root(data_root: &Path) -> Result<HashMap<String, String>> {
    let xdg_root = data_root.join("podman").join("xdg");
    let xdg_config = xdg_root.join("config");
    let xdg_data = xdg_root.join("data");
    let xdg_run = podman_runtime_root(data_root).join("run");
    let podman_home = podman_home_root(data_root);
    let podman_tmp_root = podman_temp_root(data_root);
    std::fs::create_dir_all(&xdg_config)
        .with_context(|| format!("create dir {}", xdg_config.display()))?;
    std::fs::create_dir_all(&xdg_data)
        .with_context(|| format!("create dir {}", xdg_data.display()))?;
    std::fs::create_dir_all(&xdg_run)
        .with_context(|| format!("create dir {}", xdg_run.display()))?;
    std::fs::create_dir_all(&podman_home)
        .with_context(|| format!("create dir {}", podman_home.display()))?;
    std::fs::create_dir_all(&podman_tmp_root)
        .with_context(|| format!("create dir {}", podman_tmp_root.display()))?;
    let _ = std::fs::set_permissions(&xdg_run, std::fs::Permissions::from_mode(0o700));
    let _ = std::fs::set_permissions(&podman_home, std::fs::Permissions::from_mode(0o700));

    let mut env = HashMap::new();
    env.insert(
        "XDG_CONFIG_HOME".to_string(),
        xdg_config.to_string_lossy().to_string(),
    );
    env.insert(
        "XDG_DATA_HOME".to_string(),
        xdg_data.to_string_lossy().to_string(),
    );
    env.insert(
        "XDG_RUNTIME_DIR".to_string(),
        xdg_run.to_string_lossy().to_string(),
    );
    env.insert(
        "HOME".to_string(),
        podman_home.to_string_lossy().to_string(),
    );
    let tmp = podman_tmp_root.to_string_lossy().to_string();
    env.insert("TMPDIR".to_string(), tmp.clone());
    env.insert("TMP".to_string(), tmp.clone());
    env.insert("TEMP".to_string(), tmp);
    Ok(env)
}

pub(crate) fn podman_command(data_root: &Path) -> Result<Command> {
    #[cfg(target_os = "macos")]
    if super::avf_linux_runtime_available() && super::avf_linux_vm::helper_path().is_ok() {
        let helper = super::avf_linux_vm::helper_path()?;
        let env = podman_env_for_data_root(data_root)?;
        let mut cmd = Command::new(helper);
        cmd.arg("shared-vm-exec")
            .arg("--data-root")
            .arg(data_root)
            .arg("--cwd")
            .arg("/")
            .arg("--command")
            .arg("podman")
            .arg("--user")
            .arg("root");
        let mut env_pairs = env.into_iter().collect::<Vec<_>>();
        env_pairs.sort_by(|(left, _), (right, _)| left.cmp(right));
        for (key, value) in env_pairs {
            cmd.arg("--env").arg(format!("{key}={value}"));
        }
        cmd.arg("--");
        return Ok(cmd);
    }
    let inv = podman_invocation(data_root)?;
    let mut cmd = Command::new(inv.bin);
    for (key, value) in inv.env {
        cmd.env(key, value);
    }
    Ok(cmd)
}

pub(crate) async fn command_output_with_timeout(
    mut cmd: Command,
    timeout: Duration,
) -> Result<std::process::Output> {
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.kill_on_drop(true);
    let child = cmd.spawn().context("spawning command")?;
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(res) => Ok(res?),
        Err(_) => anyhow::bail!("command timed out after {}s", timeout.as_secs()),
    }
}

pub fn container_runtime_available(data_root: &Path) -> bool {
    podman_available(data_root) || managed_podman_runtime_source().is_some()
}

pub async fn podman_engine_ready(data_root: &Path) -> Result<bool> {
    let mut cmd = podman_command(data_root)?;
    cmd.arg("info");
    match command_output_with_timeout(cmd, PODMAN_INFO_TIMEOUT).await {
        Ok(out) => Ok(out.status.success()),
        Err(_) => Ok(false),
    }
}

pub(super) fn command_output_message(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    format!("{stderr}\n{stdout}").trim().to_string()
}

pub(super) async fn container_exists(data_root: &Path, name: &str) -> Result<bool> {
    let mut cmd = podman_command(data_root)?;
    cmd.arg("container").arg("exists").arg(name);
    let output = command_output_with_timeout(cmd, PODMAN_OP_TIMEOUT).await?;
    Ok(output.status.success())
}

pub(super) async fn container_running(data_root: &Path, name: &str) -> Result<Option<bool>> {
    let mut cmd = podman_command(data_root)?;
    cmd.arg("container")
        .arg("inspect")
        .arg("--format")
        .arg("{{.State.Running}}")
        .arg(name);
    let output = command_output_with_timeout(cmd, PODMAN_OP_TIMEOUT).await?;
    if !output.status.success() {
        return Ok(None);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(Some(stdout.trim() == "true"))
}

pub(super) async fn ensure_workspace_volume(
    data_root: &Path,
    workspace_id: WorkspaceId,
) -> Result<String> {
    let name = format!("ctx-ws-{}", workspace_id.0);
    let mut inspect = podman_command(data_root)?;
    inspect.arg("volume").arg("inspect").arg(&name);
    let out = command_output_with_timeout(inspect, PODMAN_OP_TIMEOUT).await?;
    if out.status.success() {
        return Ok(name);
    }
    let mut create = podman_command(data_root)?;
    create.arg("volume").arg("create").arg(&name);
    let out = command_output_with_timeout(create, PODMAN_OP_TIMEOUT).await?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let combined = format!("{stderr}\n{stdout}").trim().to_string();
        if combined.is_empty() {
            anyhow::bail!(
                "podman volume create failed for {name} (status: {})",
                out.status
            );
        }
        anyhow::bail!("podman volume create failed for {name}: {combined}");
    }
    Ok(name)
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    struct EnvVarGuard {
        key: &'static str,
        prev: Option<String>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let prev = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, prev }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(prev) = self.prev.take() {
                std::env::set_var(self.key, prev);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    fn env_var_test_lock() -> &'static tokio::sync::Mutex<()> {
        crate::test_support::podman_env_test_lock()
    }

    #[tokio::test]
    async fn podman_available_uses_test_override() {
        let _serial = env_var_test_lock().lock().await;
        let _guard = EnvVarGuard::set("CTX_TEST_PODMAN_AVAILABLE", "true");
        assert!(podman_available(tempdir().unwrap().path()));
    }
}
