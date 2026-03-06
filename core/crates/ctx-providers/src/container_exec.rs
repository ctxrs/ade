use anyhow::{Context, Result};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use sha2::Digest;
use tokio::process::Command;

use crate::crp::rewrite_bundled_path_for_linux;

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
) -> Result<Command> {
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
        let rewritten = rewrite_container_env_value_for_linux(k, v)
            .with_context(|| format!("rewriting container env {k} for linux execution"))?;
        cmd.arg("--env").arg(format!("{k}={rewritten}"));
    }
    cmd.arg(&spec.container_id);
    cmd.arg(command);
    cmd.args(args);
    Ok(cmd)
}

fn rewrite_container_env_value_for_linux(key: &str, value: &str) -> Result<String> {
    if key == "PATH" {
        return rewrite_container_path_list_for_linux(value);
    }
    if key.ends_with("_PATH") {
        return rewrite_bundled_path_for_linux(value);
    }
    Ok(value.to_string())
}

fn rewrite_container_path_list_for_linux(value: &str) -> Result<String> {
    if value.trim().is_empty() {
        return Ok(value.to_string());
    }
    let rewritten_paths = std::env::split_paths(OsStr::new(value))
        .map(|entry| rewrite_bundled_path_for_linux(entry.to_string_lossy().as_ref()).map(PathBuf::from))
        .collect::<Result<Vec<_>>>()?;
    let joined = std::env::join_paths(rewritten_paths)
        .context("joining rewritten PATH entries for linux container execution")?;
    Ok(joined.to_string_lossy().to_string())
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
    use std::fs;

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

    #[test]
    fn build_container_exec_command_rewrites_bundled_path_envs_for_linux() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let host_provider_dir = tmp.path().join("bundles/providers/droid/macos/aarch64");
        let linux_provider_dir = tmp.path().join("bundles/providers/droid/linux/aarch64");
        fs::create_dir_all(&host_provider_dir).expect("mkdir host provider dir");
        fs::create_dir_all(&linux_provider_dir).expect("mkdir linux provider dir");
        fs::write(host_provider_dir.join("droid"), b"host").expect("write host droid");
        fs::write(linux_provider_dir.join("droid"), b"linux").expect("write linux droid");
        fs::write(linux_provider_dir.join("droid-acp"), b"linux").expect("write linux droid-acp");

        let mut env = HashMap::new();
        env.insert(
            "PATH".to_string(),
            std::env::join_paths([host_provider_dir.as_path(), Path::new("/usr/bin")])
                .expect("join path")
                .to_string_lossy()
                .to_string(),
        );
        env.insert(
            "DROID_PATH".to_string(),
            host_provider_dir.join("droid").to_string_lossy().to_string(),
        );

        let spec = ContainerExecSpec {
            container_id: "ctx-harness-1".to_string(),
            user: Some("1000:1000".to_string()),
            podman_path: None,
        };

        let cmd = build_container_exec_command(
            &spec,
            Path::new("/workspace"),
            &env,
            linux_provider_dir.join("droid-acp").to_string_lossy().as_ref(),
            &[],
        )
        .expect("build command");

        let args = cmd
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        let rewritten_path = args
            .windows(2)
            .find_map(|window| {
                (window[0] == "--env" && window[1].starts_with("PATH="))
                    .then(|| window[1].trim_start_matches("PATH=").to_string())
            })
            .expect("PATH env");
        let path_parts =
            std::env::split_paths(OsStr::new(&rewritten_path)).collect::<Vec<PathBuf>>();
        assert!(
            path_parts.first() == Some(&linux_provider_dir),
            "missing rewritten PATH env in args: {args:?}"
        );
        assert_eq!(path_parts.get(1), Some(&PathBuf::from("/usr/bin")));
        assert!(
            args.windows(2).any(|window| {
                window[0] == "--env"
                    && window[1]
                        == format!("DROID_PATH={}", linux_provider_dir.join("droid").display())
            }),
            "missing rewritten DROID_PATH env in args: {args:?}"
        );
    }

    #[test]
    fn build_container_exec_command_errors_when_linux_bundled_path_for_env_is_missing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let host_provider_dir = tmp.path().join("bundles/providers/droid/macos/aarch64");
        fs::create_dir_all(&host_provider_dir).expect("mkdir host provider dir");

        let mut env = HashMap::new();
        env.insert("PATH".to_string(), host_provider_dir.to_string_lossy().to_string());

        let spec = ContainerExecSpec {
            container_id: "ctx-harness-1".to_string(),
            user: None,
            podman_path: None,
        };

        let err = build_container_exec_command(
            &spec,
            Path::new("/workspace"),
            &env,
            "/bin/true",
            &[],
        )
        .expect_err("expected missing linux bundle path error");

        let err_text = format!("{err:#}");
        assert!(
            err_text.contains("missing linux bundled path"),
            "unexpected error: {err_text}"
        );
    }
}
