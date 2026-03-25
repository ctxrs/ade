use super::*;

fn explicit_sandbox_cli_binary_path() -> Option<PathBuf> {
    let raw = std::env::var(CTX_HARNESS_SANDBOX_CLI_PATH_ENV).ok()?;
    let path = PathBuf::from(raw.trim());
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

fn sandbox_cli_available(data_root: &Path) -> bool {
    if cfg!(test) {
        if let Ok(value) = std::env::var("CTX_TEST_SANDBOX_CLI_AVAILABLE") {
            let value = value.trim().to_ascii_lowercase();
            return matches!(value.as_str(), "1" | "true" | "yes" | "y");
        }
    }
    if explicit_sandbox_cli_binary_path().is_some() {
        return true;
    }
    #[cfg(target_os = "macos")]
    if super::avf_linux_runtime_available() && super::avf_linux_vm::helper_path().is_ok() {
        return true;
    }
    sandbox_cli_binary_path(data_root).is_some()
}

pub(super) fn sandbox_cli_binary_path(data_root: &Path) -> Option<PathBuf> {
    if let Some(path) = explicit_sandbox_cli_binary_path() {
        return Some(path);
    }
    let _ = data_root;
    which::which("nerdctl").ok()
}

#[derive(Debug, Clone)]
pub(crate) struct SandboxCliInvocation {
    pub(crate) bin: PathBuf,
    pub(crate) env: HashMap<String, String>,
}

pub(crate) fn sandbox_cli_invocation(data_root: &Path) -> Result<SandboxCliInvocation> {
    let bin = sandbox_cli_binary_path(data_root)
        .ok_or_else(|| anyhow::anyhow!("sandbox container CLI unavailable"))?;
    let env = sandbox_cli_env_for_data_root(data_root)?;
    Ok(SandboxCliInvocation { bin, env })
}

pub(crate) fn sandbox_cli_env_for_data_root(data_root: &Path) -> Result<HashMap<String, String>> {
    let xdg_root = data_root.join("sandbox").join("xdg");
    let xdg_config = xdg_root.join("config");
    let xdg_data = xdg_root.join("data");
    let xdg_run = data_root.join("sandbox").join("run");
    let sandbox_home = data_root.join("sandbox").join("home");
    let sandbox_tmp_root = data_root.join("sandbox").join("tmp");
    std::fs::create_dir_all(&xdg_config)
        .with_context(|| format!("create dir {}", xdg_config.display()))?;
    std::fs::create_dir_all(&xdg_data)
        .with_context(|| format!("create dir {}", xdg_data.display()))?;
    std::fs::create_dir_all(&xdg_run)
        .with_context(|| format!("create dir {}", xdg_run.display()))?;
    std::fs::create_dir_all(&sandbox_home)
        .with_context(|| format!("create dir {}", sandbox_home.display()))?;
    std::fs::create_dir_all(&sandbox_tmp_root)
        .with_context(|| format!("create dir {}", sandbox_tmp_root.display()))?;
    let _ = std::fs::set_permissions(&xdg_run, std::fs::Permissions::from_mode(0o700));
    let _ = std::fs::set_permissions(&sandbox_home, std::fs::Permissions::from_mode(0o700));

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
        sandbox_home.to_string_lossy().to_string(),
    );
    let tmp = sandbox_tmp_root.to_string_lossy().to_string();
    env.insert("TMPDIR".to_string(), tmp.clone());
    env.insert("TMP".to_string(), tmp.clone());
    env.insert("TEMP".to_string(), tmp);
    Ok(env)
}

pub(crate) fn sandbox_container_command(data_root: &Path) -> Result<Command> {
    if let Some(bin) = explicit_sandbox_cli_binary_path() {
        let env = sandbox_cli_env_for_data_root(data_root)?;
        let mut cmd = Command::new(bin);
        for (key, value) in env {
            cmd.env(key, value);
        }
        return Ok(cmd);
    }
    #[cfg(target_os = "macos")]
    if super::avf_linux_runtime_available() && super::avf_linux_vm::helper_path().is_ok() {
        let helper = super::avf_linux_vm::helper_path()?;
        let env = sandbox_cli_env_for_data_root(data_root)?;
        let mut cmd = Command::new(helper);
        cmd.arg("shared-vm-exec")
            .arg("--data-root")
            .arg(data_root)
            .arg("--cwd")
            .arg("/")
            .arg("--command")
            .arg("nerdctl")
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
    let inv = sandbox_cli_invocation(data_root)?;
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
    sandbox_cli_available(data_root)
}

pub async fn sandbox_engine_ready(data_root: &Path) -> Result<bool> {
    let mut cmd = sandbox_container_command(data_root)?;
    cmd.arg("info");
    match command_output_with_timeout(cmd, SANDBOX_INFO_TIMEOUT).await {
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
    let mut cmd = sandbox_container_command(data_root)?;
    cmd.arg("container").arg("exists").arg(name);
    let output = command_output_with_timeout(cmd, SANDBOX_OP_TIMEOUT).await?;
    Ok(output.status.success())
}

pub(super) async fn container_running(data_root: &Path, name: &str) -> Result<Option<bool>> {
    let mut cmd = sandbox_container_command(data_root)?;
    cmd.arg("container")
        .arg("inspect")
        .arg("--format")
        .arg("{{.State.Running}}")
        .arg(name);
    let output = command_output_with_timeout(cmd, SANDBOX_OP_TIMEOUT).await?;
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
    let mut inspect = sandbox_container_command(data_root)?;
    inspect.arg("volume").arg("inspect").arg(&name);
    let out = command_output_with_timeout(inspect, SANDBOX_OP_TIMEOUT).await?;
    if out.status.success() {
        return Ok(name);
    }
    let mut create = sandbox_container_command(data_root)?;
    create.arg("volume").arg("create").arg(&name);
    let out = command_output_with_timeout(create, SANDBOX_OP_TIMEOUT).await?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let combined = format!("{stderr}\n{stdout}").trim().to_string();
        if combined.is_empty() {
            anyhow::bail!(
                "container volume create failed for {name} (status: {})",
                out.status
            );
        }
        anyhow::bail!("container volume create failed for {name}: {combined}");
    }
    Ok(name)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

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
        crate::test_support::sandbox_cli_env_test_lock()
    }

    #[tokio::test]
    async fn sandbox_cli_available_uses_test_override() {
        let _serial = env_var_test_lock().lock().await;
        let _guard = EnvVarGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "true");
        assert!(sandbox_cli_available(tempdir().unwrap().path()));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn sandbox_engine_ready_uses_explicit_cli_override() {
        let _serial = env_var_test_lock().lock().await;
        let temp = tempdir().expect("tempdir");
        let cli_path = temp.path().join("sandbox-cli.sh");
        std::fs::write(
            &cli_path,
            "#!/bin/sh\nif [ \"$1\" = \"info\" ]; then\n  printf '{}\\n'\n  exit 0\nfi\necho \"unexpected invocation: $*\" >&2\nexit 1\n",
        )
        .expect("write sandbox cli shim");
        std::fs::set_permissions(&cli_path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod sandbox cli shim");
        let _guard = EnvVarGuard::set(
            CTX_HARNESS_SANDBOX_CLI_PATH_ENV,
            &cli_path.to_string_lossy(),
        );

        assert!(
            sandbox_engine_ready(temp.path())
                .await
                .expect("sandbox engine ready check"),
            "sandbox engine should honor the explicit CLI override",
        );
    }
}
