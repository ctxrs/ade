use std::collections::HashMap;
use std::path::Path;

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
