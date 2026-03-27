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
    harness_runtime::ensure_builder_backend_launch_ready_with_observer(data_root, None)
        .await
        .context("ensuring sandbox runtime launch readiness")?;
    harness_runtime::prefetch_container_image(
        data_root,
        harness_runtime::default_container_image(),
    )
    .await
    .context("ensuring builder image availability")?;

    let mut cmd = harness_runtime::sandbox_container_command(data_root)?;
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
    let mut cmd = harness_runtime::sandbox_container_command(data_root)?;
    configure_builder_run(&mut cmd, data_root, cwd, env, argv)?;
    harness_runtime::command_output_with_timeout(cmd, timeout_dur)
        .await
        .context("running container builder command")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::tempdir;

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
            "--env flags must be container run options before image"
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

        fn unset(key: &'static str) -> Self {
            let prev = std::env::var(key).ok();
            std::env::remove_var(key);
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

    #[cfg(unix)]
    fn write_default_harness_bundle(root: &Path) -> PathBuf {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        let bundle_dir = root.join("bundle");
        let images_dir = bundle_dir.join("images");
        fs::create_dir_all(&images_dir).expect("create bundle image dir");
        let tar_path = images_dir.join("ctx-harness.tar");
        fs::write(&tar_path, b"bundle image tar").expect("write bundle image tar");
        let manifest = serde_json::json!({
            "version": 1,
            "providers": [],
            "runtimes": [],
            "images": [{
                "id": "ctx-harness",
                "version": "test",
                "os": "linux",
                "arch": std::env::consts::ARCH,
                "sha256": "test-sha",
                "tar": "images/ctx-harness.tar",
                "image": harness_runtime::default_container_image(),
            }],
        });
        fs::write(
            bundle_dir.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).expect("serialize manifest"),
        )
        .expect("write bundle manifest");
        fs::set_permissions(&tar_path, fs::Permissions::from_mode(0o644))
            .expect("chmod bundle image tar");
        bundle_dir
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn ensure_builder_ready_starts_clean_cold_runtime_before_image_prewarm() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        let _serial = crate::test_support::sandbox_cli_env_test_lock()
            .lock()
            .await;
        let temp = tempdir().expect("tempdir");
        let bundle_dir = write_default_harness_bundle(temp.path());
        let log_path = temp.path().join("sandbox-cli.log");
        let machine_present = temp.path().join("machine-present");
        let machine_started = temp.path().join("machine-started");
        let image_present = temp.path().join("image-present");
        let sandbox_cli_path = temp.path().join("sandbox-cli.sh");
        fs::write(
            &sandbox_cli_path,
            format!(
                "#!/bin/sh\nLOG=\"{log}\"\nMACHINE_PRESENT=\"{machine_present}\"\nSTARTED=\"{machine_started}\"\nIMAGE_PRESENT=\"{image_present}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"info\" ]; then\n  if [ -f \"$STARTED\" ]; then\n    printf '{{}}\\n'\n    exit 0\n  fi\n  echo 'engine unavailable' >&2\n  exit 125\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"inspect\" ]; then\n  if [ -f \"$MACHINE_PRESENT\" ]; then\n    printf '[{{\"Name\":\"%s\"}}]\\n' \"$3\"\n    exit 0\n  fi\n  echo 'machine not found' >&2\n  exit 1\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"init\" ]; then\n  : > \"$MACHINE_PRESENT\"\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"start\" ]; then\n  : > \"$STARTED\"\n  exit 0\nfi\nif [ \"$1\" = \"image\" ] && [ \"$2\" = \"inspect\" ]; then\n  if [ -f \"$IMAGE_PRESENT\" ]; then\n    printf '[{{}}]\\n'\n    exit 0\n  fi\n  echo 'image missing' >&2\n  exit 1\nfi\nif [ \"$1\" = \"load\" ] && [ \"$2\" = \"-i\" ]; then\n  : > \"$IMAGE_PRESENT\"\n  printf 'Loaded image: {image}\\n'\n  exit 0\nfi\nif [ \"$1\" = \"run\" ]; then\n  exit 0\nfi\necho \"unexpected sandbox CLI invocation: $*\" >&2\nexit 1\n",
                log = log_path.display(),
                machine_present = machine_present.display(),
                machine_started = machine_started.display(),
                image_present = image_present.display(),
                image = harness_runtime::default_container_image(),
            ),
        )
        .expect("write sandbox CLI shim");
        fs::set_permissions(&sandbox_cli_path, fs::Permissions::from_mode(0o755))
            .expect("chmod sandbox CLI shim");

        let _bundle = EnvVarGuard::set("CTX_BUNDLE_DIR", &bundle_dir.to_string_lossy());
        let _sandbox_cli = EnvVarGuard::set(
            crate::harness_runtime::CTX_HARNESS_SANDBOX_CLI_PATH_ENV,
            &sandbox_cli_path.to_string_lossy(),
        );
        let _test_override = EnvVarGuard::unset("CTX_TEST_SANDBOX_CLI_AVAILABLE");

        ensure_builder_ready(temp.path())
            .await
            .expect("builder should become ready from a clean cold runtime");

        let log = fs::read_to_string(&log_path).expect("read sandbox CLI log");
        assert!(
            log.contains("machine start ctx-"),
            "expected cold builder readiness to start the sandbox machine: {log}"
        );
        assert!(
            log.contains("load -i"),
            "expected builder readiness to load the harness image after runtime startup: {log}"
        );
        assert!(
            log.contains("run --rm"),
            "expected builder readiness command to execute once the runtime and image were ready: {log}"
        );
    }
}
