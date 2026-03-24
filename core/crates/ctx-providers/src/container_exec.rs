use anyhow::{Context, Result};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use sha2::Digest;
use tokio::process::Command;

use crate::crp::rewrite_bundled_path_for_linux;

const CTX_HARNESS_RUNTIME_KIND_ENV: &str = "CTX_HARNESS_RUNTIME_KIND";
const CTX_HARNESS_CONTAINER_ID_ENV: &str = "CTX_HARNESS_CONTAINER_ID";
const CTX_HARNESS_CONTAINER_USER_ENV: &str = "CTX_HARNESS_CONTAINER_USER";
const CTX_PODMAN_PATH_ENV: &str = "CTX_PODMAN_PATH";
const CTX_AVF_LINUX_HELPER_PATH_ENV: &str = "CTX_AVF_LINUX_HELPER_PATH";
const CTX_AVF_HOST_DATA_ROOT_ENV: &str = "CTX_AVF_HOST_DATA_ROOT";
const CTX_AVF_WORKSPACE_ID_ENV: &str = "CTX_AVF_WORKSPACE_ID";
const CTX_AVF_WORKTREE_ID_ENV: &str = "CTX_AVF_WORKTREE_ID";
const CTX_AVF_HOST_WORKTREE_ROOT_ENV: &str = "CTX_AVF_HOST_WORKTREE_ROOT";
const CTX_AVF_GUEST_WORKTREE_ROOT_ENV: &str = "CTX_AVF_GUEST_WORKTREE_ROOT";
const CTX_HARNESS_GUEST_WORKSPACE_ROOT_ENV: &str = "CTX_HARNESS_GUEST_WORKSPACE_ROOT";

#[derive(Debug, Clone)]
pub enum ContainerExecSpec {
    Podman {
        container_id: String,
        user: Option<String>,
        podman_path: Option<String>,
    },
    AvfLinuxVm {
        helper_path: String,
        data_root: PathBuf,
        workspace_id: String,
        worktree_id: String,
        host_worktree_root: PathBuf,
        guest_worktree_root: PathBuf,
        guest_workspace_root: PathBuf,
        user: Option<String>,
    },
}

pub fn container_exec_spec(env: &HashMap<String, String>) -> Option<ContainerExecSpec> {
    if env
        .get(CTX_HARNESS_CONTAINER_ID_ENV)
        .is_some_and(|value| !value.trim().is_empty())
    {
        return Some(ContainerExecSpec::Podman {
            container_id: env.get(CTX_HARNESS_CONTAINER_ID_ENV)?.to_string(),
            user: env.get(CTX_HARNESS_CONTAINER_USER_ENV).cloned(),
            podman_path: env.get(CTX_PODMAN_PATH_ENV).cloned(),
        });
    }

    if env.get(CTX_HARNESS_RUNTIME_KIND_ENV).map(String::as_str) != Some("avf_linux_vm") {
        return None;
    }

    Some(ContainerExecSpec::AvfLinuxVm {
        helper_path: env.get(CTX_AVF_LINUX_HELPER_PATH_ENV)?.to_string(),
        data_root: PathBuf::from(env.get(CTX_AVF_HOST_DATA_ROOT_ENV)?),
        workspace_id: env.get(CTX_AVF_WORKSPACE_ID_ENV)?.to_string(),
        worktree_id: env.get(CTX_AVF_WORKTREE_ID_ENV)?.to_string(),
        host_worktree_root: PathBuf::from(env.get(CTX_AVF_HOST_WORKTREE_ROOT_ENV)?),
        guest_worktree_root: PathBuf::from(env.get(CTX_AVF_GUEST_WORKTREE_ROOT_ENV)?),
        guest_workspace_root: PathBuf::from(env.get(CTX_HARNESS_GUEST_WORKSPACE_ROOT_ENV)?),
        user: env.get(CTX_HARNESS_CONTAINER_USER_ENV).cloned(),
    })
}

pub fn build_container_exec_command(
    spec: &ContainerExecSpec,
    workdir: &Path,
    env: &HashMap<String, String>,
    command: &str,
    args: &[String],
) -> Result<Command> {
    match spec {
        ContainerExecSpec::Podman {
            container_id,
            user,
            podman_path,
        } => {
            let mut cmd = Command::new(podman_path.as_deref().unwrap_or("podman"));
            // Keep Podman state deterministic and tied to the daemon data root when available.
            // Without this, `podman exec` may try to use a different connection/machine and fail.
            if let Some(root) = env
                .get("CTX_DATA_ROOT_HOST")
                .or_else(|| env.get("CTX_DATA_ROOT"))
            {
                apply_podman_env(&mut cmd, root);
            }
            cmd.arg("exec").arg("--interactive");
            if let Some(user) = user.as_deref() {
                cmd.arg("--user").arg(user);
            }
            cmd.arg("--workdir").arg(workdir);
            for (k, v) in env {
                if should_skip_linux_exec_env_key(spec, k) {
                    continue;
                }
                let rewritten = rewrite_container_env_value_for_linux(k, v)
                    .with_context(|| format!("rewriting container env {k} for linux execution"))?;
                cmd.arg("--env").arg(format!("{k}={rewritten}"));
            }
            cmd.arg(container_id);
            cmd.arg(command);
            cmd.args(args);
            Ok(cmd)
        }
        ContainerExecSpec::AvfLinuxVm {
            helper_path,
            data_root,
            workspace_id,
            worktree_id: _,
            host_worktree_root,
            guest_worktree_root,
            guest_workspace_root,
            user,
        } => {
            let guest_cwd = resolve_avf_guest_cwd(
                workdir,
                host_worktree_root,
                guest_worktree_root,
                guest_workspace_root,
            )?;
            let mut cmd = Command::new(helper_path);
            cmd.arg("shared-vm-exec")
                .arg("--data-root")
                .arg(data_root)
                .arg("--cwd")
                .arg("/")
                .arg("--command")
                .arg("podman")
                .arg("--user")
                .arg("root");
            for (key, value) in podman_env_for_data_root(&data_root.to_string_lossy()) {
                cmd.arg("--env").arg(format!("{key}={value}"));
            }
            cmd.arg("--");
            cmd.arg("exec").arg("--interactive");
            if let Some(user) = user.as_deref() {
                cmd.arg("--user").arg(user);
            }
            cmd.arg("--workdir").arg(&guest_cwd);
            for (k, v) in env {
                if should_skip_linux_exec_env_key(spec, k) {
                    continue;
                }
                let rewritten = rewrite_container_env_value_for_linux(k, v)
                    .with_context(|| format!("rewriting container env {k} for linux execution"))?;
                cmd.arg("--env").arg(format!("{k}={rewritten}"));
            }
            cmd.arg(format!("ctx-harness-{workspace_id}"));
            cmd.arg(command);
            cmd.args(args);
            Ok(cmd)
        }
    }
}

fn should_skip_linux_exec_env_key(spec: &ContainerExecSpec, key: &str) -> bool {
    match spec {
        ContainerExecSpec::Podman { .. } => {
            key.starts_with("CTX_HARNESS_CONTAINER_") || key == CTX_PODMAN_PATH_ENV
        }
        ContainerExecSpec::AvfLinuxVm { .. } => key.starts_with("CTX_AVF_"),
    }
}

fn resolve_avf_guest_cwd(
    workdir: &Path,
    host_worktree_root: &Path,
    guest_worktree_root: &Path,
    guest_workspace_root: &Path,
) -> Result<PathBuf> {
    if workdir.starts_with(guest_workspace_root) {
        return Ok(workdir.to_path_buf());
    }
    if workdir == host_worktree_root {
        return Ok(guest_worktree_root.to_path_buf());
    }
    if workdir.starts_with(host_worktree_root) {
        let relative = workdir
            .strip_prefix(host_worktree_root)
            .context("mapping host worktree cwd for AVF execution")?;
        return Ok(join_guest_relative(guest_worktree_root, relative));
    }
    if workdir.starts_with(guest_worktree_root) {
        return Ok(workdir.to_path_buf());
    }
    anyhow::bail!(
        "AVF guest cwd mapping failed: workdir {} is outside host root {} and guest root {}",
        workdir.display(),
        host_worktree_root.display(),
        guest_worktree_root.display()
    );
}

fn join_guest_relative(root: &Path, relative: &Path) -> PathBuf {
    let mut out = root.to_path_buf();
    if relative != Path::new("") {
        out.push(relative);
    }
    out
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
        .map(|entry| {
            rewrite_bundled_path_for_linux(entry.to_string_lossy().as_ref()).map(PathBuf::from)
        })
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
            host_provider_dir
                .join("droid")
                .to_string_lossy()
                .to_string(),
        );

        let spec = ContainerExecSpec::Podman {
            container_id: "ctx-harness-1".to_string(),
            user: Some("1000:1000".to_string()),
            podman_path: None,
        };

        let cmd = build_container_exec_command(
            &spec,
            Path::new("/workspace"),
            &env,
            linux_provider_dir
                .join("droid-acp")
                .to_string_lossy()
                .as_ref(),
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
        env.insert(
            "PATH".to_string(),
            host_provider_dir.to_string_lossy().to_string(),
        );

        let spec = ContainerExecSpec::Podman {
            container_id: "ctx-harness-1".to_string(),
            user: None,
            podman_path: None,
        };

        let err =
            build_container_exec_command(&spec, Path::new("/workspace"), &env, "/bin/true", &[])
                .expect_err("expected missing linux bundle path error");

        let err_text = format!("{err:#}");
        assert!(
            err_text.contains("missing linux bundled path"),
            "unexpected error: {err_text}"
        );
    }

    #[test]
    fn container_exec_spec_detects_avf_linux_vm_env_contract() {
        let mut env = HashMap::new();
        env.insert(
            CTX_HARNESS_RUNTIME_KIND_ENV.to_string(),
            "avf_linux_vm".to_string(),
        );
        env.insert(
            CTX_AVF_LINUX_HELPER_PATH_ENV.to_string(),
            "/tmp/ctx-avf-linux-helper".to_string(),
        );
        env.insert(
            CTX_AVF_HOST_DATA_ROOT_ENV.to_string(),
            "/tmp/ctx-data-root".to_string(),
        );
        env.insert(CTX_AVF_WORKSPACE_ID_ENV.to_string(), "ws-123".to_string());
        env.insert(CTX_AVF_WORKTREE_ID_ENV.to_string(), "wt-456".to_string());
        env.insert(
            CTX_AVF_HOST_WORKTREE_ROOT_ENV.to_string(),
            "/Users/example-user/code/repo".to_string(),
        );
        env.insert(
            CTX_AVF_GUEST_WORKTREE_ROOT_ENV.to_string(),
            "/ctx/ws/worktrees/wt-456".to_string(),
        );
        env.insert(
            CTX_HARNESS_GUEST_WORKSPACE_ROOT_ENV.to_string(),
            "/ctx/ws".to_string(),
        );

        let spec = container_exec_spec(&env).expect("AVF exec spec");
        match spec {
            ContainerExecSpec::AvfLinuxVm {
                helper_path,
                data_root,
                workspace_id,
                worktree_id,
                host_worktree_root,
                guest_worktree_root,
                guest_workspace_root,
                user,
            } => {
                assert_eq!(helper_path, "/tmp/ctx-avf-linux-helper");
                assert_eq!(data_root, PathBuf::from("/tmp/ctx-data-root"));
                assert_eq!(workspace_id, "ws-123");
                assert_eq!(worktree_id, "wt-456");
                assert_eq!(host_worktree_root, PathBuf::from("/Users/example-user/code/repo"));
                assert_eq!(
                    guest_worktree_root,
                    PathBuf::from("/ctx/ws/worktrees/wt-456")
                );
                assert_eq!(guest_workspace_root, PathBuf::from("/ctx/ws"));
                assert_eq!(user, None);
            }
            other => panic!("expected AVF Linux VM spec, got {other:?}"),
        }
    }

    #[test]
    fn build_container_exec_command_maps_host_workdir_to_avf_guest_exec() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let host_provider_dir = tmp.path().join("bundles/providers/droid/macos/aarch64");
        let linux_provider_dir = tmp.path().join("bundles/providers/droid/linux/aarch64");
        fs::create_dir_all(&host_provider_dir).expect("mkdir host provider dir");
        fs::create_dir_all(&linux_provider_dir).expect("mkdir linux provider dir");
        fs::write(host_provider_dir.join("droid"), b"host").expect("write host droid");
        fs::write(linux_provider_dir.join("droid"), b"linux").expect("write linux droid");

        let host_worktree_root = tmp.path().join("repo");
        fs::create_dir_all(host_worktree_root.join("src")).expect("mkdir worktree");
        let helper_path = tmp.path().join("ctx-avf-linux-helper");
        fs::write(&helper_path, b"#!/bin/sh\nexit 0\n").expect("write helper");

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
            host_provider_dir
                .join("droid")
                .to_string_lossy()
                .to_string(),
        );

        let spec = ContainerExecSpec::AvfLinuxVm {
            helper_path: helper_path.to_string_lossy().to_string(),
            data_root: tmp.path().join("ctx-data-root"),
            workspace_id: "ws-123".to_string(),
            worktree_id: "wt-456".to_string(),
            host_worktree_root: host_worktree_root.clone(),
            guest_worktree_root: PathBuf::from("/ctx/ws/worktrees/wt-456"),
            guest_workspace_root: PathBuf::from("/ctx/ws"),
            user: Some("ctx-ws-123".to_string()),
        };

        let cmd = build_container_exec_command(
            &spec,
            &host_worktree_root.join("src"),
            &env,
            "/usr/bin/env",
            &["--version".to_string()],
        )
        .expect("build AVF shared-vm container exec command");

        let args = cmd
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        assert_eq!(args.first().map(String::as_str), Some("shared-vm-exec"));
        assert!(
            args.windows(2).any(|window| window[0] == "--data-root"
                && window[1] == tmp.path().join("ctx-data-root").to_string_lossy()),
            "missing --data-root in args: {args:?}"
        );
        assert!(
            args.windows(2)
                .any(|window| { window[0] == "--command" && window[1] == "podman" }),
            "missing shared-vm podman command in args: {args:?}"
        );
        assert!(
            args.windows(2)
                .any(|window| window[0] == "--cwd" && window[1] == "/"),
            "missing shared-vm cwd in args: {args:?}"
        );
        assert!(
            args.windows(2)
                .any(|window| window[0] == "--user" && window[1] == "ctx-ws-123"),
            "missing container user in args: {args:?}"
        );
        assert!(
            args.windows(2).any(|window| {
                window[0] == "--workdir" && window[1] == "/ctx/ws/worktrees/wt-456/src"
            }),
            "missing translated --workdir in args: {args:?}"
        );
        assert!(
            args.windows(2)
                .any(|window| window[0] == "--" && window[1] == "exec"),
            "missing podman exec boundary in args: {args:?}"
        );
        assert!(
            args.windows(2)
                .any(|window| window[0] == "ctx-harness-ws-123" && window[1] == "/usr/bin/env"),
            "missing container name and command in args: {args:?}"
        );
        assert!(
            args.windows(2).any(|window| {
                if window[0] != "--env" || !window[1].starts_with("PATH=") {
                    return false;
                }
                let value = window[1].trim_start_matches("PATH=");
                let parts = std::env::split_paths(OsStr::new(value)).collect::<Vec<PathBuf>>();
                parts.first() == Some(&linux_provider_dir)
                    && parts.get(1) == Some(&PathBuf::from("/usr/bin"))
            }),
            "missing rewritten PATH env in args: {args:?}"
        );
        assert!(
            args.windows(2).any(|window| {
                window[0] == "--env"
                    && window[1]
                        == format!("DROID_PATH={}", linux_provider_dir.join("droid").display())
            }),
            "missing rewritten DROID_PATH env in args: {args:?}"
        );
        assert!(
            args.iter().any(|arg| arg == "--version"),
            "missing passthrough argument in args: {args:?}"
        );
    }
}
