use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::process::Command;

use crate::harness_runtime;

const BUILDER_READY_TIMEOUT: Duration = Duration::from_secs(2 * 60);

fn builder_platform_for_arch(arch: &str) -> Result<&'static str> {
    match arch {
        "x86_64" => Ok("linux/amd64"),
        "aarch64" => Ok("linux/arm64"),
        other => anyhow::bail!("unsupported container builder architecture: {other}"),
    }
}

fn data_root_bind_mount(data_root: &Path) -> String {
    format!(
        "type=bind,src={},dst={},rw",
        data_root.to_string_lossy(),
        data_root.to_string_lossy()
    )
}

fn configure_builder_run(
    cmd: &mut Command,
    data_root: &Path,
    cwd: &Path,
    env: &[(String, String)],
    argv: &[String],
) -> Result<()> {
    cmd.args(builder_run_args(data_root, cwd, env, argv)?);
    Ok(())
}

fn builder_run_args(
    data_root: &Path,
    cwd: &Path,
    env: &[(String, String)],
    argv: &[String],
) -> Result<Vec<String>> {
    let platform = builder_platform_for_arch(std::env::consts::ARCH)?;
    let mut args = vec![
        "run".to_string(),
        "--rm".to_string(),
        "--pull=never".to_string(),
        "--platform".to_string(),
        platform.to_string(),
        "--mount".to_string(),
        data_root_bind_mount(data_root),
        "--workdir".to_string(),
        cwd.to_string_lossy().to_string(),
    ];
    for (key, value) in env {
        args.push("--env".to_string());
        args.push(format!("{key}={value}"));
    }
    args.push(harness_runtime::default_container_image().to_string());
    args.extend(argv.iter().cloned());
    Ok(args)
}

pub async fn ensure_builder_ready(data_root: &Path) -> Result<()> {
    if !harness_runtime::container_runtime_available(data_root) {
        anyhow::bail!("container runtime unavailable");
    }
    harness_runtime::prefetch_container_image(
        data_root,
        harness_runtime::default_container_image(),
    )
    .await
    .context("ensuring builder image availability")?;

    let mut cmd = harness_runtime::podman_command(data_root)?;
    configure_builder_run(
        &mut cmd,
        data_root,
        data_root,
        &[],
        &["/bin/sh".to_string(), "-lc".to_string(), "true".to_string()],
    )?;
    let output = harness_runtime::command_output_with_timeout(cmd, BUILDER_READY_TIMEOUT)
        .await
        .context("running builder readiness command")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let combined = format!("{stderr}\n{stdout}").trim().to_string();
        if combined.is_empty() {
            anyhow::bail!(
                "container builder readiness command failed (status: {})",
                output.status
            );
        }
        anyhow::bail!(
            "container builder readiness command failed (status: {}): {}",
            output.status,
            combined
        );
    }
    Ok(())
}

pub async fn run_command(
    data_root: &Path,
    cwd: &Path,
    env: &[(String, String)],
    argv: &[String],
    timeout_dur: Duration,
) -> Result<std::process::Output> {
    let mut cmd = harness_runtime::podman_command(data_root)?;
    configure_builder_run(&mut cmd, data_root, cwd, env, argv)?;
    harness_runtime::command_output_with_timeout(cmd, timeout_dur)
        .await
        .context("running container builder command")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_shell_command(script: &str) -> Command {
        #[cfg(windows)]
        {
            let mut cmd = Command::new("cmd");
            cmd.arg("/C").arg(script);
            cmd
        }
        #[cfg(not(windows))]
        {
            let mut cmd = Command::new("sh");
            cmd.arg("-lc").arg(script);
            cmd
        }
    }

    #[test]
    fn platform_maps_supported_arches() {
        assert_eq!(
            builder_platform_for_arch("x86_64").expect("x86_64"),
            "linux/amd64"
        );
        assert_eq!(
            builder_platform_for_arch("aarch64").expect("aarch64"),
            "linux/arm64"
        );
    }

    #[test]
    fn data_root_bind_mount_uses_rw_mount() {
        let path = Path::new("/tmp/ctx-data");
        let mount = data_root_bind_mount(path);
        assert!(mount.contains("type=bind"));
        assert!(mount.contains("src=/tmp/ctx-data"));
        assert!(mount.contains("dst=/tmp/ctx-data"));
        assert!(mount.ends_with(",rw"));
    }

    #[test]
    fn builder_run_args_puts_env_flags_before_image() {
        let args = builder_run_args(
            Path::new("/tmp/ctx-data"),
            Path::new("/tmp/ctx-data/work"),
            &[("NPM_CONFIG_CACHE".to_string(), "/tmp/cache".to_string())],
            &[
                "/bin/sh".to_string(),
                "-lc".to_string(),
                "echo ok".to_string(),
            ],
        )
        .expect("builder args");
        let image = harness_runtime::default_container_image();
        let image_index = args.iter().position(|arg| arg == image).expect("image arg");
        let env_flag_index = args
            .iter()
            .position(|arg| arg == "--env")
            .expect("--env flag");
        assert!(
            env_flag_index < image_index,
            "--env flags must be Podman run options before image"
        );
    }

    #[tokio::test]
    async fn timeout_helper_reports_timeout_for_long_process() {
        #[cfg(windows)]
        let cmd = make_shell_command("ping -n 6 127.0.0.1 >NUL");
        #[cfg(not(windows))]
        let cmd = make_shell_command("sleep 5");
        let err = harness_runtime::command_output_with_timeout(cmd, Duration::from_millis(50))
            .await
            .expect_err("command should time out");
        assert!(err.to_string().contains("timed out"));
    }

    #[tokio::test]
    async fn timeout_helper_returns_output_for_fast_process() {
        let cmd = make_shell_command("echo ok");
        let out = harness_runtime::command_output_with_timeout(cmd, Duration::from_secs(2))
            .await
            .expect("fast command should succeed");
        assert!(out.status.success());
    }
}
