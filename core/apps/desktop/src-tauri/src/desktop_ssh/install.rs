use super::*;

#[path = "install/artifacts.rs"]
mod artifacts;

pub(super) use artifacts::*;
fn render_remote_bundle_sync_cmd(
    remote_data_dir: &str,
    remote_bundle_dir: &str,
    remote_bundle_backup_dir: &str,
    remote_tmp_root: &str,
    remote_appimage_path: &str,
    remote_extract_root: &str,
    remote_staged_bundle_dir: &str,
) -> String {
    format!(
        "set -eu; \
tmp_root={tmp_root}; \
appimage={appimage}; \
extract_root={extract_root}; \
staged_bundle={staged_bundle}; \
dest={dest}; \
backup={backup}; \
no_previous_marker=\"$backup/.ctx-no-previous-bundle\"; \
cleanup() {{ rm -rf \"$tmp_root\"; }}; \
trap cleanup EXIT INT TERM; \
mkdir -p {data_dir}; \
rm -rf \"$tmp_root\"; \
mkdir -p \"$extract_root\" \"$staged_bundle\"; \
cat > \"$appimage\"; \
chmod 755 \"$appimage\"; \
(cd \"$extract_root\" && \"$appimage\" --appimage-extract >/dev/null 2>&1); \
bundle_src=$(find \"$extract_root\"/squashfs-root -type d -path '*/bundles' -print -quit); \
if [ -z \"$bundle_src\" ]; then echo 'managed remote desktop artifact missing bundles directory' >&2; exit 1; fi; \
cp -R \"$bundle_src\"/. \"$staged_bundle\"/; \
rm -rf \"$backup\"; \
if [ -e \"$dest\" ]; then mv \"$dest\" \"$backup\"; else mkdir -p \"$backup\"; touch \"$no_previous_marker\"; fi; \
if mv \"$staged_bundle\" \"$dest\"; then :; else status=$?; if [ -e \"$no_previous_marker\" ]; then rm -rf \"$dest\" \"$backup\"; elif [ -e \"$backup\" ]; then rm -rf \"$dest\"; mv \"$backup\" \"$dest\"; fi; exit \"$status\"; fi;",
        tmp_root = remote_path_expr(remote_tmp_root),
        appimage = remote_path_expr(remote_appimage_path),
        extract_root = remote_path_expr(remote_extract_root),
        staged_bundle = remote_path_expr(remote_staged_bundle_dir),
        data_dir = remote_path_expr(remote_data_dir),
        dest = remote_path_expr(remote_bundle_dir),
        backup = remote_path_expr(remote_bundle_backup_dir),
    )
}

pub(super) fn install_remote_daemon_over_ssh(
    app: &tauri::AppHandle,
    host: &str,
    user: Option<&str>,
    remote_platform: RemoteLinuxPlatform,
    remote_ctx_bin: &str,
) -> Result<()> {
    let target = ssh_target(host, user);
    let channel = normalize_update_channel(std::env::var("CTX_DESKTOP_CHANNEL").ok().as_deref())
        .map_err(anyhow::Error::msg)?;
    let local_bin = ensure_managed_remote_daemon_binary(app, remote_platform.arch, &channel)?;
    let parent_dir = remote_ctx_bin_parent_dir(remote_ctx_bin)?;
    let remote_ctx_bin = validate_remote_ctx_bin(remote_ctx_bin)?;
    let temp_remote_path = format!("{remote_ctx_bin}.tmp-{}", std::process::id());
    let install_cmd = format!(
        "mkdir -p {parent} && cat > {tmp} && chmod 755 {tmp} && mv -f {tmp} {dest}",
        parent = remote_path_expr(&parent_dir),
        tmp = remote_path_expr(&temp_remote_path),
        dest = remote_path_expr(&remote_ctx_bin),
    );
    let mut child = new_ssh_command()
        .arg("-o")
        .arg("BatchMode=yes")
        .arg("-o")
        .arg("ConnectTimeout=15")
        .arg("-o")
        .arg("ServerAliveInterval=15")
        .arg("-o")
        .arg("ServerAliveCountMax=2")
        .arg(target)
        .arg(format!("sh -lc {}", shell_escape(&install_cmd)))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("spawning ssh for remote daemon install")?;
    {
        let mut file = std::fs::File::open(&local_bin)
            .with_context(|| format!("opening managed daemon at {}", local_bin.display()))?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("ssh stdin unavailable for daemon install"))?;
        std::io::copy(&mut file, &mut stdin).context("streaming daemon binary over ssh")?;
    }
    let output = child
        .wait_with_output()
        .context("waiting for remote daemon install ssh command")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            anyhow::bail!("remote daemon install failed");
        }
        anyhow::bail!("remote daemon install failed: {stderr}");
    }
    Ok(())
}

pub(super) fn sync_remote_bundle_metadata_over_ssh(
    app: &tauri::AppHandle,
    host: &str,
    user: Option<&str>,
    remote_data_dir: Option<&str>,
    remote_arch: &str,
    channel: &str,
) -> Result<()> {
    let local_appimage = ensure_managed_remote_bundle_appimage(app, remote_arch, channel)?;
    let data_dir = remote_data_dir
        .filter(|d| !d.trim().is_empty())
        .unwrap_or("~/.ctx");
    let remote_bundle_dir = remote_bundle_dir_for_data_dir(data_dir);
    let remote_bundle_backup_dir = remote_bundle_backup_dir_for_data_dir(data_dir);
    let remote_tmp_root = join_remote_path(
        data_dir,
        &format!(".bundle-sync.tmp-{}", std::process::id()),
    );
    let remote_appimage_path = join_remote_path(&remote_tmp_root, "ctx.AppImage");
    let remote_extract_root = join_remote_path(&remote_tmp_root, "extract");
    let remote_staged_bundle_dir = join_remote_path(&remote_tmp_root, "bundles");
    let target = ssh_target(host, user);
    let remote_cmd = render_remote_bundle_sync_cmd(
        data_dir,
        &remote_bundle_dir,
        &remote_bundle_backup_dir,
        &remote_tmp_root,
        &remote_appimage_path,
        &remote_extract_root,
        &remote_staged_bundle_dir,
    );
    let mut ssh_child = new_ssh_command()
        .arg("-o")
        .arg("BatchMode=yes")
        .arg("-o")
        .arg("ConnectTimeout=15")
        .arg("-o")
        .arg("ServerAliveInterval=15")
        .arg("-o")
        .arg("ServerAliveCountMax=2")
        .arg(target)
        .arg(format!("sh -lc {}", shell_escape(&remote_cmd)))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("spawning ssh for remote bundle sync")?;
    {
        let mut local_appimage_file = std::fs::File::open(&local_appimage).with_context(|| {
            format!(
                "opening managed remote desktop artifact at {}",
                local_appimage.display()
            )
        })?;
        let mut ssh_stdin = ssh_child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("ssh stdin unavailable for remote bundle sync"))?;
        std::io::copy(&mut local_appimage_file, &mut ssh_stdin)
            .context("streaming managed remote desktop artifact over ssh")?;
    }
    let ssh_output = ssh_child
        .wait_with_output()
        .context("waiting for remote bundle metadata ssh command")?;
    if !ssh_output.status.success() {
        let stderr = String::from_utf8_lossy(&ssh_output.stderr)
            .trim()
            .to_string();
        anyhow::bail!("remote bundle metadata sync failed: {stderr}");
    }
    Ok(())
}

pub(super) fn restore_remote_bundle_backup_over_ssh(
    host: &str,
    user: Option<&str>,
    remote_data_dir: Option<&str>,
) -> Result<()> {
    let data_dir = remote_data_dir
        .filter(|d| !d.trim().is_empty())
        .unwrap_or("~/.ctx");
    let bundle_dir = remote_bundle_dir_for_data_dir(data_dir);
    let backup_dir = remote_bundle_backup_dir_for_data_dir(data_dir);
    let cmd = format!(
        "bundle={bundle}; backup={backup}; no_previous_marker=\"$backup/.ctx-no-previous-bundle\"; \
if [ -e \"$no_previous_marker\" ]; then rm -rf \"$bundle\" \"$backup\"; \
elif [ -e \"$backup\" ]; then rm -rf \"$bundle\" && mv \"$backup\" \"$bundle\"; \
else echo 'remote bundle backup missing after failed update' >&2; exit 127; fi",
        bundle = remote_path_expr(&bundle_dir),
        backup = remote_path_expr(&backup_dir),
    );
    let output =
        run_remote_ssh_shell(host, user, &cmd).context("restoring remote bundle backup")?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let detail = if !stderr.is_empty() { stderr } else { stdout };
    anyhow::bail!("remote bundle restore failed: {detail}");
}

pub(super) fn cleanup_remote_bundle_backup_over_ssh(
    host: &str,
    user: Option<&str>,
    remote_data_dir: Option<&str>,
) -> Result<()> {
    let data_dir = remote_data_dir
        .filter(|d| !d.trim().is_empty())
        .unwrap_or("~/.ctx");
    let backup_dir = remote_bundle_backup_dir_for_data_dir(data_dir);
    let cmd = format!("rm -rf {}", remote_path_expr(&backup_dir));
    let output =
        run_remote_ssh_shell(host, user, &cmd).context("cleaning up remote bundle backup")?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let detail = if !stderr.is_empty() { stderr } else { stdout };
    anyhow::bail!("remote bundle backup cleanup failed: {detail}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_bundle_artifact_prefers_appimage_then_desktop() {
        let manifest: ReleaseManifest = serde_json::from_str(
            r#"{
              "platforms": {
                "linux-x64": {
                  "desktop": { "url_path": "/desktop", "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" },
                  "appimage": { "url_path": "/appimage", "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb" }
                },
                "linux-arm64": {
                  "desktop": { "url_path": "/desktop-arm", "sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc" }
                }
              }
            }"#,
        )
        .expect("parse manifest");

        let x64 = manifest.platforms.get("linux-x64").expect("x64 entry");
        let arm64 = manifest.platforms.get("linux-arm64").expect("arm64 entry");
        assert_eq!(
            release_bundle_artifact_for_platform(x64)
                .expect("x64 bundle artifact")
                .url_path,
            "/appimage"
        );
        assert_eq!(
            release_bundle_artifact_for_platform(arm64)
                .expect("arm64 bundle artifact")
                .url_path,
            "/desktop-arm"
        );
    }

    #[test]
    fn remote_bundle_sync_command_extracts_appimage_on_remote() {
        let cmd = render_remote_bundle_sync_cmd(
            "~/.ctx",
            "~/.ctx/bundles",
            "~/.ctx/bundles.pre-update-backup",
            "~/.ctx/.bundle-sync.tmp-99",
            "~/.ctx/.bundle-sync.tmp-99/ctx.AppImage",
            "~/.ctx/.bundle-sync.tmp-99/extract",
            "~/.ctx/.bundle-sync.tmp-99/bundles",
        );
        assert!(cmd.contains("--appimage-extract"));
        assert!(cmd.contains("find \"$extract_root\"/squashfs-root -type d -path '*/bundles'"));
        assert!(cmd.contains("managed remote desktop artifact missing bundles directory"));
        assert!(cmd.contains("mv \"$staged_bundle\" \"$dest\""));
        assert!(cmd.contains("backup=\"$HOME/.ctx/bundles.pre-update-backup\""));
        assert!(cmd.contains("touch \"$no_previous_marker\""));
        assert!(
            cmd.contains("if [ -e \"$no_previous_marker\" ]; then rm -rf \"$dest\" \"$backup\";")
        );
        assert!(!cmd.contains("rm -rf \"$HOME/.ctx/bundles\""));
        assert!(cmd.contains("cat > \"$appimage\""));
        assert!(!cmd.contains("desktop_bundle_dir"));
    }
}

pub(super) fn remote_ctx_bin_exists_over_ssh(
    host: &str,
    user: Option<&str>,
    remote_ctx_bin: &str,
) -> Result<bool> {
    let ctx_bin = validate_remote_ctx_bin(remote_ctx_bin)?;
    let check_cmd = format!(
        "if [ -x {ctx_bin} ]; then exit 0; else exit 1; fi",
        ctx_bin = remote_path_expr(&ctx_bin),
    );
    let output = run_remote_ssh_shell(host, user, &check_cmd)
        .context("checking remote managed daemon binary")?;
    if output.status.success() {
        return Ok(true);
    }
    if output.status.code() == Some(1) {
        return Ok(false);
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let detail = if !stderr.is_empty() { stderr } else { stdout };
    anyhow::bail!("checking remote managed daemon binary failed: {detail}");
}

pub(super) fn start_remote_daemon_over_ssh(
    host: &str,
    user: Option<&str>,
    remote_port: u16,
    remote_data_dir: Option<&str>,
    remote_ctx_bin: &str,
    update_channel: Option<&str>,
    update_base_url: Option<&str>,
) -> Result<()> {
    let target = ssh_target(host, user);
    let data_dir = remote_data_dir
        .filter(|d| !d.trim().is_empty())
        .unwrap_or("~/.ctx");
    let bundle_dir = remote_bundle_dir_for_data_dir(data_dir);
    let log_dir = format!("{}/logs", data_dir.trim_end_matches('/'));
    let log_dir_expr = remote_path_expr(&log_dir);
    let log_file = format!("{}/daemon.log", log_dir.trim_end_matches('/'));
    let log_file_expr = remote_path_expr(&log_file);
    let ctx_bin = validate_remote_ctx_bin(remote_ctx_bin)?;
    let exec_cmd = render_remote_daemon_exec_cmd(
        &ctx_bin,
        remote_port,
        data_dir,
        &bundle_dir,
        update_channel,
        update_base_url,
    )?;
    let log_cmd = format!(
        "mkdir -p {log_dir} && {exec_cmd} > {log_file} 2>&1",
        log_dir = log_dir_expr,
        log_file = log_file_expr,
    );
    let remote_cmd = format!(
        "mkdir -p {log_dir} && nohup /bin/sh -lc {cmd} >/dev/null 2>&1 < /dev/null &",
        log_dir = log_dir_expr,
        cmd = shell_escape(&log_cmd),
    );

    let output = new_ssh_command()
        .arg("-o")
        .arg("BatchMode=yes")
        .arg("-o")
        .arg("ConnectTimeout=15")
        .arg("-o")
        .arg("ServerAliveInterval=15")
        .arg("-o")
        .arg("ServerAliveCountMax=2")
        .arg(target)
        .arg(format!("sh -lc {}", shell_escape(&remote_cmd)))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .context("starting remote daemon over ssh")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!(
            "ssh start failed: {stderr}; remote_cmd={remote_cmd}"
        ));
    }
    Ok(())
}

pub(super) fn render_remote_daemon_exec_cmd(
    remote_ctx_bin: &str,
    remote_port: u16,
    remote_data_dir: &str,
    remote_bundle_dir: &str,
    update_channel: Option<&str>,
    update_base_url: Option<&str>,
) -> Result<String> {
    let ctx_bin = validate_remote_ctx_bin(remote_ctx_bin)?;
    let ctx_bin_expr = remote_path_expr(&ctx_bin);
    let linux_sandbox_env_prefix = remote_linux_sandbox_daemon_env_prefix(remote_data_dir);
    let update_env = match (
        update_channel.filter(|value| !value.trim().is_empty()),
        update_base_url.filter(|value| !value.trim().is_empty()),
    ) {
        (Some(channel), Some(base_url)) => format!(
            "CTX_MANAGED_DAEMON_AUTO_UPDATE=1 CTX_DAEMON_UPDATE_CHANNEL={channel} CTX_DAEMON_UPDATE_BASE_URL={base_url} ",
            channel = shell_escape(channel),
            base_url = shell_escape(base_url),
        ),
        _ => String::new(),
    };
    Ok(format!(
        "{linux_sandbox_env_prefix} if [ -x {ctx_bin} ]; then CTX_BUNDLE_DIR={bundle_dir} {update_env}{ctx_bin} serve --bind 127.0.0.1:{remote_port} --data-dir {dir}; else echo 'ctx not executable at configured remote path' >&2; exit 127; fi",
        linux_sandbox_env_prefix = linux_sandbox_env_prefix,
        ctx_bin = ctx_bin_expr,
        bundle_dir = remote_path_expr(remote_bundle_dir),
        update_env = update_env,
        dir = remote_path_expr(remote_data_dir),
    ))
}
