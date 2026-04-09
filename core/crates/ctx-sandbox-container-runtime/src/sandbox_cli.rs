use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result};
use ctx_core::ids::WorkspaceId;
use ctx_linux_sandbox_runtime::preferred_native_sandbox_cli_path;
use tokio::process::Command;

use crate::{
    CTX_HARNESS_SANDBOX_CLI_PATH_ENV, SANDBOX_INFO_TIMEOUT, SANDBOX_OP_TIMEOUT,
    SandboxCommandMode,
};

pub const SHARED_VM_SANDBOX_CLI_GUEST_BIN: &str = "/usr/local/bin/nerdctl";

fn find_binary_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

fn explicit_sandbox_cli_binary_path() -> Option<PathBuf> {
    let raw = std::env::var(CTX_HARNESS_SANDBOX_CLI_PATH_ENV).ok()?;
    let path = PathBuf::from(raw.trim());
    if path.exists() { Some(path) } else { None }
}

fn sandbox_cli_available(_data_root: &Path) -> bool {
    if explicit_sandbox_cli_binary_path().is_some() {
        return true;
    }
    if cfg!(test) {
        if let Ok(value) = std::env::var("CTX_TEST_SANDBOX_CLI_AVAILABLE") {
            let value = value.trim().to_ascii_lowercase();
            return matches!(value.as_str(), "1" | "true" | "yes" | "y");
        }
    }
    preferred_native_sandbox_cli_path().is_some() || find_binary_in_path("nerdctl").is_some()
}

pub fn native_container_runtime_available(data_root: &Path) -> bool {
    sandbox_cli_available(data_root)
}

pub fn sandbox_cli_binary_path(_data_root: &Path) -> Option<PathBuf> {
    if let Some(path) = explicit_sandbox_cli_binary_path() {
        return Some(path);
    }
    if let Some(path) = preferred_native_sandbox_cli_path() {
        return Some(path);
    }
    find_binary_in_path("nerdctl")
}

#[derive(Debug, Clone)]
pub struct SandboxCliInvocation {
    pub bin: PathBuf,
    pub env: HashMap<String, String>,
}

pub fn sandbox_cli_invocation(data_root: &Path) -> Result<SandboxCliInvocation> {
    let bin = sandbox_cli_binary_path(data_root)
        .ok_or_else(|| anyhow::anyhow!("sandbox container CLI unavailable"))?;
    let env = sandbox_cli_env_for_data_root(data_root)?;
    Ok(SandboxCliInvocation { bin, env })
}

pub fn sandbox_cli_env_for_data_root(data_root: &Path) -> Result<HashMap<String, String>> {
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
        "XDG_CACHE_HOME".to_string(),
        xdg_root.join("cache").to_string_lossy().to_string(),
    );
    env.insert(
        "XDG_RUNTIME_DIR".to_string(),
        xdg_run.to_string_lossy().to_string(),
    );
    env.insert(
        "HOME".to_string(),
        sandbox_home.to_string_lossy().to_string(),
    );
    env.insert(
        "CONTAINERD_ADDRESS".to_string(),
        "/run/containerd/containerd.sock".to_string(),
    );
    env.insert("CONTAINERD_NAMESPACE".to_string(), "default".to_string());
    let tmp = sandbox_tmp_root.to_string_lossy().to_string();
    env.insert("TMPDIR".to_string(), tmp.clone());
    env.insert("TMP".to_string(), tmp.clone());
    env.insert("TEMP".to_string(), tmp);
    Ok(env)
}

pub fn sandbox_container_command(data_root: &Path, mode: &SandboxCommandMode) -> Result<Command> {
    if let Some(bin) = explicit_sandbox_cli_binary_path() {
        let env = sandbox_cli_env_for_data_root(data_root)?;
        let mut cmd = Command::new(bin);
        for (key, value) in env {
            cmd.env(key, value);
        }
        return Ok(cmd);
    }

    match mode {
        SandboxCommandMode::NativeContainer => {
            let inv = sandbox_cli_invocation(data_root)?;
            let mut cmd = Command::new(inv.bin);
            for (key, value) in inv.env {
                cmd.env(key, value);
            }
            Ok(cmd)
        }
        SandboxCommandMode::SharedVm { helper_path } => {
            let env = sandbox_cli_env_for_data_root(data_root)?;
            let mut cmd = Command::new(helper_path);
            cmd.arg("shared-vm-exec")
                .arg("--data-root")
                .arg(data_root)
                .arg("--cwd")
                .arg("/")
                .arg("--command")
                .arg(SHARED_VM_SANDBOX_CLI_GUEST_BIN)
                .arg("--user")
                .arg("root");
            let mut env_pairs = env.into_iter().collect::<Vec<_>>();
            env_pairs.sort_by(|(left, _), (right, _)| left.cmp(right));
            for (key, value) in env_pairs {
                cmd.arg("--env").arg(format!("{key}={value}"));
            }
            cmd.arg("--");
            Ok(cmd)
        }
    }
}

pub async fn command_output_with_timeout(
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

pub fn command_output_message(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    format!("{stderr}\n{stdout}").trim().to_string()
}

pub async fn sandbox_engine_ready(data_root: &Path, mode: &SandboxCommandMode) -> Result<bool> {
    let mut cmd = sandbox_container_command(data_root, mode)?;
    cmd.arg("info");
    match command_output_with_timeout(cmd, SANDBOX_INFO_TIMEOUT).await {
        Ok(out) => Ok(out.status.success()),
        Err(_) => Ok(false),
    }
}

pub async fn container_exists(
    data_root: &Path,
    mode: &SandboxCommandMode,
    name: &str,
) -> Result<bool> {
    let mut cmd = sandbox_container_command(data_root, mode)?;
    cmd.arg("container").arg("inspect").arg(name);
    let output = command_output_with_timeout(cmd, SANDBOX_OP_TIMEOUT).await?;
    Ok(output.status.success())
}

pub async fn container_running(
    data_root: &Path,
    mode: &SandboxCommandMode,
    name: &str,
) -> Result<Option<bool>> {
    let mut cmd = sandbox_container_command(data_root, mode)?;
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

pub async fn ensure_workspace_volume(
    data_root: &Path,
    mode: &SandboxCommandMode,
    workspace_id: WorkspaceId,
) -> Result<String> {
    let name = format!("ctx-ws-{}", workspace_id.0);
    let mut inspect = sandbox_container_command(data_root, mode)?;
    inspect.arg("volume").arg("inspect").arg(&name);
    let out = command_output_with_timeout(inspect, SANDBOX_OP_TIMEOUT).await?;
    if out.status.success() {
        return Ok(name);
    }
    let mut create = sandbox_container_command(data_root, mode)?;
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
        crate::sandbox_cli_env_test_lock()
    }

    #[cfg(unix)]
    fn write_shared_vm_state_helper(root: &Path, simulated: bool) -> (PathBuf, EnvVarGuard) {
        let helper_path = root.join("avf-linux-helper.sh");
        let simulated_value = if simulated { "true" } else { "false" };
        std::fs::write(
            &helper_path,
            format!(
                "#!/bin/sh\ncmd=\"$1\"\nshift\ncase \"$cmd\" in\n  workspace-vm-state)\n    data_root=\"$1\"\n    vm_root=\"$data_root/managed/vms/avf-linux/test/shared\"\n    logs_root=\"$vm_root/logs\"\n    state_path=\"$vm_root/shared-vm-state.json\"\n    log_path=\"$logs_root/shared-vm.log\"\n    mkdir -p \"$logs_root\"\n    printf '{{\"protocol_version\":1,\"protocol_schema\":\"ctx.avf_linux_helper.v1\",\"state\":\"running\",\"vm_root\":\"%s\",\"logs_root\":\"%s\",\"state_path\":\"%s\",\"log_path\":\"%s\",\"transition_status\":\"ready\",\"last_start_outcome\":\"already_running\",\"simulated\":{simulated_value},\"notes\":[\"sandbox cli test helper\"]}}\\n' \"$vm_root\" \"$logs_root\" \"$state_path\" \"$log_path\"\n    ;;\n  *)\n    echo \"unexpected helper invocation: $cmd $*\" >&2\n    exit 1\n    ;;\nesac\n"
            ),
        )
        .expect("write AVF Linux helper shim");
        std::fs::set_permissions(&helper_path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod AVF Linux helper shim");
        let guard = EnvVarGuard::set(
            super::super::avf_linux_vm::AVF_LINUX_HELPER_PATH_ENV,
            &helper_path.to_string_lossy(),
        );
        (helper_path, guard)
    }

    #[tokio::test]
    async fn sandbox_cli_available_uses_test_override() {
        let _serial = env_var_test_lock().lock().await;
        let _guard = EnvVarGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "true");
        assert!(sandbox_cli_available(tempdir().unwrap().path()));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn explicit_cli_override_beats_negative_test_override() {
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
        let _override = EnvVarGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "0");
        let _guard =
            EnvVarGuard::set(CTX_HARNESS_SANDBOX_CLI_PATH_ENV, &cli_path.to_string_lossy());

        assert!(sandbox_cli_available(temp.path()));
        assert!(
            sandbox_engine_ready(temp.path(), &SandboxCommandMode::NativeContainer)
                .await
                .expect("sandbox engine ready check"),
            "sandbox engine should honor the explicit CLI override even when the negative test override is set",
        );
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
        let _guard =
            EnvVarGuard::set(CTX_HARNESS_SANDBOX_CLI_PATH_ENV, &cli_path.to_string_lossy());

        assert!(
            sandbox_engine_ready(temp.path(), &SandboxCommandMode::NativeContainer)
                .await
                .expect("sandbox engine ready check"),
            "sandbox engine should honor the explicit CLI override",
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn container_exists_uses_runtime_neutral_inspect_probe() {
        let _serial = env_var_test_lock().lock().await;
        let temp = tempdir().expect("tempdir");
        let cli_path = temp.path().join("sandbox-cli.sh");
        let log_path = temp.path().join("sandbox-cli.log");
        std::fs::write(
            &cli_path,
            format!(
                "#!/bin/sh\nLOG=\"{log}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"container\" ] && [ \"$2\" = \"inspect\" ] && [ \"$3\" = \"ctx-harness-test\" ]; then\n  printf '[{{}}]\\n'\n  exit 0\nfi\necho \"unexpected invocation: $*\" >&2\nexit 1\n",
                log = log_path.display(),
            ),
        )
        .expect("write sandbox cli shim");
        std::fs::set_permissions(&cli_path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod sandbox cli shim");
        let _guard =
            EnvVarGuard::set(CTX_HARNESS_SANDBOX_CLI_PATH_ENV, &cli_path.to_string_lossy());

        assert!(
            container_exists(
                temp.path(),
                &SandboxCommandMode::NativeContainer,
                "ctx-harness-test",
            )
            .await
            .expect("container exists probe"),
            "inspect-based probe should treat the container as existing",
        );

        let log = std::fs::read_to_string(&log_path).expect("read sandbox cli log");
        assert!(
            log.contains("container inspect ctx-harness-test"),
            "expected inspect probe in log:\n{log}"
        );
        assert!(
            !log.contains("container exists"),
            "inspect-based probe should not call unsupported container exists:\n{log}"
        );
    }
}
