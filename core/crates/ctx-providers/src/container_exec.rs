use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tokio::process::Command;

#[derive(Debug, Clone)]
pub struct ContainerExecSpec {
    pub container_id: String,
    pub user: Option<String>,
    pub podman_path: Option<String>,
}

pub fn container_exec_spec(env: &HashMap<String, String>) -> Option<ContainerExecSpec> {
    let container_id = env.get("CTX_HARNESS_CONTAINER_ID")?.to_string();
    let user = env.get("CTX_HARNESS_CONTAINER_USER").cloned();
    let podman_path = env.get("CTX_PODMAN_PATH").cloned();
    Some(ContainerExecSpec {
        container_id,
        user,
        podman_path,
    })
}

pub fn build_container_exec_command(
    spec: &ContainerExecSpec,
    workdir: &Path,
    env: &HashMap<String, String>,
    command: &str,
    args: &[String],
) -> Command {
    let mut cmd = Command::new(spec.podman_path.as_deref().unwrap_or("podman"));
    // Keep Podman state deterministic and tied to the daemon data root when available.
    // Without this, `podman exec` may try to use a different connection/machine and fail.
    if let Some(root) = env
        .get("CTX_DATA_ROOT_HOST")
        .or_else(|| env.get("CTX_DATA_ROOT"))
    {
        apply_podman_xdg_env(&mut cmd, root);
    }
    cmd.arg("exec").arg("--interactive");
    if let Some(user) = spec.user.as_deref() {
        cmd.arg("--user").arg(user);
    }
    cmd.arg("--workdir").arg(workdir);
    for (k, v) in env {
        if k.starts_with("CTX_HARNESS_CONTAINER_") || k == "CTX_PODMAN_PATH" {
            continue;
        }
        cmd.arg("--env").arg(format!("{k}={v}"));
    }
    cmd.arg(&spec.container_id);
    cmd.arg(command);
    cmd.args(args);
    cmd
}

fn apply_podman_xdg_env(cmd: &mut Command, data_root: &str) {
    let data_root = data_root.trim();
    if data_root.is_empty() {
        return;
    }
    let root = PathBuf::from(data_root);
    let xdg_root = root.join("podman").join("xdg");
    let xdg_config = xdg_root.join("config");
    let xdg_data = xdg_root.join("data");
    let xdg_run = xdg_root.join("run");
    // Best-effort: directory creation failures will surface as podman connection failures.
    let _ = std::fs::create_dir_all(&xdg_config);
    let _ = std::fs::create_dir_all(&xdg_data);
    let _ = std::fs::create_dir_all(&xdg_run);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&xdg_run, std::fs::Permissions::from_mode(0o700));
    }
    cmd.env("XDG_CONFIG_HOME", xdg_config);
    cmd.env("XDG_DATA_HOME", xdg_data);
    cmd.env("XDG_RUNTIME_DIR", xdg_run);
}
