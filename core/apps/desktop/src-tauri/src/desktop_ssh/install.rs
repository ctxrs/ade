use super::*;
use sha2::Digest;

#[derive(Debug, Clone, Deserialize)]
struct ReleaseArtifact {
    url_path: String,
    sha256: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ReleasePlatformEntry {
    #[serde(default)]
    daemon: Option<ReleaseArtifact>,
    #[serde(default)]
    appimage: Option<ReleaseArtifact>,
    #[serde(default)]
    desktop: Option<ReleaseArtifact>,
}

#[derive(Debug, Clone, Deserialize)]
struct ReleaseManifest {
    #[serde(default)]
    platforms: std::collections::HashMap<String, ReleasePlatformEntry>,
}

pub(super) fn validate_remote_ctx_bin(value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        anyhow::bail!("remote_ctx_bin is required");
    }
    let valid_absolute = trimmed.starts_with('/');
    let valid_home_relative = trimmed == "~" || trimmed.starts_with("~/");
    if !valid_absolute && !valid_home_relative {
        anyhow::bail!(
            "remote_ctx_bin must be an absolute path or ~/ path (for example ~/.ctx/bin/ctx)"
        );
    }
    Ok(trimmed.to_string())
}

pub(super) fn remote_ctx_bin_parent_dir(remote_ctx_bin: &str) -> Result<String> {
    let path = validate_remote_ctx_bin(remote_ctx_bin)?;
    if path == "~" {
        return Ok("~".to_string());
    }
    let Some((parent, _name)) = path.rsplit_once('/') else {
        anyhow::bail!("remote_ctx_bin must include a file name");
    };
    if parent.is_empty() {
        return Ok("/".to_string());
    }
    Ok(parent.to_string())
}

fn bootstrap_download_base_url() -> String {
    std::env::var("CTX_DOWNLOAD_BASE_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_DOWNLOAD_BASE_URL.to_string())
}

fn remote_daemon_platform_key_for_arch(arch: &str) -> Result<&'static str> {
    match arch {
        "x86_64" => Ok("linux-x64"),
        "aarch64" => Ok("linux-arm64"),
        other => anyhow::bail!("unsupported remote daemon arch for bootstrap: {other}"),
    }
}

fn normalize_sha256_hex(raw: &str) -> Result<String> {
    let normalized = raw.trim().to_ascii_lowercase();
    if normalized.len() != 64 || !normalized.chars().all(|ch| ch.is_ascii_hexdigit()) {
        anyhow::bail!("invalid sha256 digest format");
    }
    Ok(normalized)
}

fn join_url(base: &str, path_or_url: &str) -> String {
    let trimmed = path_or_url.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return trimmed.to_string();
    }
    format!("{}{}", base.trim_end_matches('/'), trimmed)
}

fn fetch_release_manifest_for_channel(channel: &str, base_url: &str) -> Result<ReleaseManifest> {
    let url = format!(
        "{}/releases/{}/latest.json",
        base_url.trim_end_matches('/'),
        channel
    );
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(REMOTE_DAEMON_DOWNLOAD_TIMEOUT_SECS))
        .build()
        .context("building release manifest http client")?;
    let resp = client
        .get(&url)
        .send()
        .with_context(|| format!("fetching release manifest: {url}"))?
        .error_for_status()
        .with_context(|| format!("release manifest http error: {url}"))?;
    resp.json::<ReleaseManifest>()
        .context("parsing release manifest JSON")
}

fn sha256_hex_file(path: &std::path::Path) -> Result<String> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let mut hasher = sha2::Sha256::new();
    hasher.update(&bytes);
    Ok(format!("{:x}", hasher.finalize()))
}

fn remote_bundle_dir_for_data_dir(remote_data_dir: &str) -> String {
    format!("{}/bundles", remote_data_dir.trim_end_matches('/'))
}

fn managed_remote_artifact_download_path(
    app: &tauri::AppHandle,
    namespace: &str,
    channel: &str,
    platform_key: &str,
    sha256: &str,
    file_name: &str,
) -> Result<PathBuf> {
    let channel = channel.trim();
    if channel.is_empty() {
        anyhow::bail!("channel is required");
    }
    let data_dir = daemon_data_dir(app)?;
    Ok(data_dir
        .join("updates")
        .join(namespace)
        .join(channel)
        .join(platform_key)
        .join(format!("sha256-{sha256}"))
        .join(file_name))
}

fn managed_remote_daemon_download_path(
    app: &tauri::AppHandle,
    channel: &str,
    platform_key: &str,
    sha256: &str,
) -> Result<PathBuf> {
    managed_remote_artifact_download_path(
        app,
        "remote-daemon",
        channel,
        platform_key,
        sha256,
        "ctx",
    )
}

fn managed_remote_bundle_appimage_download_path(
    app: &tauri::AppHandle,
    channel: &str,
    platform_key: &str,
    sha256: &str,
) -> Result<PathBuf> {
    managed_remote_artifact_download_path(
        app,
        "remote-bundles",
        channel,
        platform_key,
        sha256,
        "ctx.AppImage",
    )
}

fn cache_release_artifact(
    final_path: &std::path::Path,
    artifact: &ReleaseArtifact,
    base_url: &str,
    error_label: &str,
    make_executable: bool,
) -> Result<PathBuf> {
    let expected_sha = normalize_sha256_hex(&artifact.sha256)?;
    if final_path.exists() {
        let digest = sha256_hex_file(final_path)
            .with_context(|| format!("computing sha256 for {}", final_path.display()))?;
        if digest.eq_ignore_ascii_case(&expected_sha) {
            return Ok(final_path.to_path_buf());
        }
        std::fs::remove_file(final_path).with_context(|| {
            format!(
                "removing corrupted cached {error_label} at {}",
                final_path.display()
            )
        })?;
    }
    let parent = final_path
        .parent()
        .ok_or_else(|| anyhow!("invalid download target path: {}", final_path.display()))?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("creating {error_label} cache dir {}", parent.display()))?;
    let file_name = final_path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| anyhow!("invalid artifact file name: {}", final_path.display()))?;
    let tmp_path = parent.join(format!("{file_name}.tmp-{}", std::process::id()));
    let artifact_url = join_url(base_url, &artifact.url_path);
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(REMOTE_DAEMON_DOWNLOAD_TIMEOUT_SECS))
        .build()
        .context("building managed artifact download client")?;
    let mut resp = client
        .get(&artifact_url)
        .send()
        .with_context(|| format!("downloading {error_label} artifact: {artifact_url}"))?
        .error_for_status()
        .with_context(|| format!("{error_label} artifact download http error: {artifact_url}"))?;
    {
        let mut file = std::fs::File::create(&tmp_path)
            .with_context(|| format!("creating {}", tmp_path.display()))?;
        std::io::copy(&mut resp, &mut file)
            .with_context(|| format!("writing {}", tmp_path.display()))?;
    }
    let digest = sha256_hex_file(&tmp_path)
        .with_context(|| format!("computing sha256 for {}", tmp_path.display()))?;
    if !digest.eq_ignore_ascii_case(&expected_sha) {
        std::fs::remove_file(&tmp_path).ok();
        anyhow::bail!(
            "{error_label} artifact checksum mismatch (expected {}, got {})",
            expected_sha,
            digest
        );
    }
    std::fs::rename(&tmp_path, final_path).with_context(|| {
        format!(
            "moving {error_label} artifact into cache ({} -> {})",
            tmp_path.display(),
            final_path.display()
        )
    })?;
    if make_executable {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(final_path)
                .with_context(|| format!("reading metadata for {}", final_path.display()))?
                .permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(final_path, perms)
                .with_context(|| format!("setting executable bit on {}", final_path.display()))?;
        }
    }
    Ok(final_path.to_path_buf())
}

fn release_bundle_artifact_for_platform(
    platform_entry: &ReleasePlatformEntry,
) -> Option<&ReleaseArtifact> {
    platform_entry
        .appimage
        .as_ref()
        .or(platform_entry.desktop.as_ref())
}

fn ensure_managed_remote_daemon_binary(
    app: &tauri::AppHandle,
    remote_arch: &str,
    channel: &str,
) -> Result<PathBuf> {
    let platform_key = remote_daemon_platform_key_for_arch(remote_arch)?;
    let base_url = bootstrap_download_base_url();
    let manifest = fetch_release_manifest_for_channel(channel, &base_url)?;
    let platform_entry = manifest
        .platforms
        .get(platform_key)
        .ok_or_else(|| anyhow!("release manifest missing platform entry: {platform_key}"))?;
    let daemon_artifact = platform_entry
        .daemon
        .as_ref()
        .ok_or_else(|| anyhow!("release manifest missing daemon artifact for {platform_key}"))?;
    let expected_sha = normalize_sha256_hex(&daemon_artifact.sha256)?;
    let final_path =
        managed_remote_daemon_download_path(app, channel, platform_key, &expected_sha)?;
    cache_release_artifact(
        &final_path,
        daemon_artifact,
        &base_url,
        "managed remote daemon",
        true,
    )
}

fn ensure_managed_remote_bundle_appimage(
    app: &tauri::AppHandle,
    remote_arch: &str,
    channel: &str,
) -> Result<PathBuf> {
    let platform_key = remote_daemon_platform_key_for_arch(remote_arch)?;
    let base_url = bootstrap_download_base_url();
    let manifest = fetch_release_manifest_for_channel(channel, &base_url)?;
    let platform_entry = manifest
        .platforms
        .get(platform_key)
        .ok_or_else(|| anyhow!("release manifest missing platform entry: {platform_key}"))?;
    let bundle_artifact =
        release_bundle_artifact_for_platform(platform_entry).ok_or_else(|| {
            anyhow!("release manifest missing Linux desktop artifact for {platform_key}")
        })?;
    let expected_sha = normalize_sha256_hex(&bundle_artifact.sha256)?;
    let final_path =
        managed_remote_bundle_appimage_download_path(app, channel, platform_key, &expected_sha)?;
    cache_release_artifact(
        &final_path,
        bundle_artifact,
        &base_url,
        "managed remote bundle",
        true,
    )
}

fn render_remote_bundle_sync_cmd(
    remote_data_dir: &str,
    remote_bundle_dir: &str,
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
rm -rf {dest}; \
mv \"$staged_bundle\" {dest};",
        tmp_root = remote_path_expr(remote_tmp_root),
        appimage = remote_path_expr(remote_appimage_path),
        extract_root = remote_path_expr(remote_extract_root),
        staged_bundle = remote_path_expr(remote_staged_bundle_dir),
        data_dir = remote_path_expr(remote_data_dir),
        dest = remote_path_expr(remote_bundle_dir),
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
            "~/.ctx/.bundle-sync.tmp-99",
            "~/.ctx/.bundle-sync.tmp-99/ctx.AppImage",
            "~/.ctx/.bundle-sync.tmp-99/extract",
            "~/.ctx/.bundle-sync.tmp-99/bundles",
        );
        assert!(cmd.contains("--appimage-extract"));
        assert!(cmd.contains("find \"$extract_root\"/squashfs-root -type d -path '*/bundles'"));
        assert!(cmd.contains("managed remote desktop artifact missing bundles directory"));
        assert!(cmd.contains("mv \"$staged_bundle\" \"$HOME/.ctx/bundles\""));
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
    let exec_cmd = render_remote_daemon_exec_cmd(&ctx_bin, remote_port, data_dir, &bundle_dir)?;
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
) -> Result<String> {
    let ctx_bin = validate_remote_ctx_bin(remote_ctx_bin)?;
    let ctx_bin_expr = remote_path_expr(&ctx_bin);
    let linux_sandbox_env_prefix = remote_linux_sandbox_daemon_env_prefix(remote_data_dir);
    Ok(format!(
        "{linux_sandbox_env_prefix} if [ -x {ctx_bin} ]; then CTX_BUNDLE_DIR={bundle_dir} {ctx_bin} serve --bind 127.0.0.1:{remote_port} --data-dir {dir}; else echo 'ctx not executable at configured remote path' >&2; exit 127; fi",
        linux_sandbox_env_prefix = linux_sandbox_env_prefix,
        ctx_bin = ctx_bin_expr,
        bundle_dir = remote_path_expr(remote_bundle_dir),
        dir = remote_path_expr(remote_data_dir),
    ))
}
