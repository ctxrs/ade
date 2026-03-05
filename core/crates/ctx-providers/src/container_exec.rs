use std::collections::HashMap;
use std::path::{Path, PathBuf};

use sha2::Digest;
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
        apply_podman_env(&mut cmd, root);
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

fn apply_podman_env(cmd: &mut Command, data_root: &str) {
    for (key, value) in podman_env_for_data_root(data_root) {
        cmd.env(key, value);
    }
}

fn podman_env_for_data_root(data_root: &str) -> HashMap<String, String> {
    let data_root = data_root.trim();
    if data_root.is_empty() {
        return HashMap::new();
    }
    let root = PathBuf::from(data_root);
    let xdg_root = root.join("podman").join("xdg");
    let xdg_config = xdg_root.join("config");
    let xdg_data = xdg_root.join("data");
    let xdg_run = podman_runtime_root(&root).join("run");
    let podman_home = podman_home_root(&root);
    let podman_tmp_root = podman_temp_root(&root);
    // Best-effort: directory creation failures will surface as podman connection failures.
    let _ = std::fs::create_dir_all(&xdg_config);
    let _ = std::fs::create_dir_all(&xdg_data);
    let _ = std::fs::create_dir_all(&xdg_run);
    let _ = std::fs::create_dir_all(&podman_home);
    let _ = std::fs::create_dir_all(&podman_tmp_root);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&xdg_run, std::fs::Permissions::from_mode(0o700));
        let _ = std::fs::set_permissions(&podman_home, std::fs::Permissions::from_mode(0o700));
    }

    let tmp = podman_tmp_root.to_string_lossy().to_string();
    HashMap::from([
        (
            "XDG_CONFIG_HOME".to_string(),
            xdg_config.to_string_lossy().to_string(),
        ),
        (
            "XDG_DATA_HOME".to_string(),
            xdg_data.to_string_lossy().to_string(),
        ),
        (
            "XDG_RUNTIME_DIR".to_string(),
            xdg_run.to_string_lossy().to_string(),
        ),
        (
            "HOME".to_string(),
            podman_home.to_string_lossy().to_string(),
        ),
        ("TMPDIR".to_string(), tmp.clone()),
        ("TMP".to_string(), tmp.clone()),
        ("TEMP".to_string(), tmp),
    ])
}

fn podman_runtime_root(data_root: &Path) -> PathBuf {
    let hash = podman_data_root_hash(data_root);
    #[cfg(unix)]
    {
        PathBuf::from("/tmp").join("ctxp").join(hash)
    }
    #[cfg(not(unix))]
    {
        std::env::temp_dir().join("ctxp").join(hash)
    }
}

fn podman_home_root(data_root: &Path) -> PathBuf {
    podman_runtime_root(data_root).join("home")
}

fn podman_temp_root(data_root: &Path) -> PathBuf {
    podman_runtime_root(data_root).join("tmp")
}

fn podman_data_root_hash(data_root: &Path) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(data_root.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    digest[..6]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn podman_env_for_data_root_uses_short_runtime_root_and_shared_state_paths() {
        let temp = tempfile::tempdir().expect("tempdir");
        let env = podman_env_for_data_root(&temp.path().to_string_lossy());

        let runtime_dir = env.get("XDG_RUNTIME_DIR").cloned().expect("runtime dir");
        let home = env.get("HOME").cloned().expect("home");
        let tmpdir = env.get("TMPDIR").cloned().expect("tmpdir");

        assert_eq!(
            PathBuf::from(runtime_dir),
            podman_runtime_root(temp.path()).join("run")
        );
        assert_eq!(PathBuf::from(home), podman_home_root(temp.path()));
        assert_eq!(PathBuf::from(tmpdir), podman_temp_root(temp.path()));
        assert_eq!(
            env.get("XDG_CONFIG_HOME").map(PathBuf::from),
            Some(temp.path().join("podman").join("xdg").join("config"))
        );
        assert_eq!(
            env.get("XDG_DATA_HOME").map(PathBuf::from),
            Some(temp.path().join("podman").join("xdg").join("data"))
        );
    }
}
