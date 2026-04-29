use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ctx_bundled_assets as bundled_assets;
use ctx_core::boolish::parse_boolish;
use sha2::Digest;

const CTX_MCP_COMMAND_ENV: &str = "CTX_MCP_COMMAND";
const CTX_MCP_DISABLED_ENV: &str = "CTX_MCP_DISABLED";
const CTX_MCP_RUNTIME_ID: &str = "ctx-mcp";

pub(crate) fn configure_runtime_mcp_command(
    provider_env: &mut HashMap<String, String>,
    data_root: &Path,
) -> Result<()> {
    if !mcp_enabled(provider_env) {
        return Ok(());
    }

    if provider_env_targets_linux_sandbox(provider_env) {
        let bundled = bundled_assets::bundled_runtime_for(
            CTX_MCP_RUNTIME_ID,
            "linux",
            std::env::consts::ARCH,
        )
        .ok_or_else(|| {
            anyhow::anyhow!(
                "linux sandbox ctx-mcp runtime is unavailable for {}",
                std::env::consts::ARCH
            )
        })?;

        let staged_path = stage_linux_sandbox_mcp_runtime(data_root, &bundled)?;
        provider_env.insert(
            CTX_MCP_COMMAND_ENV.to_string(),
            staged_path.to_string_lossy().to_string(),
        );
        return Ok(());
    }

    if let Some(command) = provider_env
        .get(CTX_MCP_COMMAND_ENV)
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        validate_explicit_mcp_command(command)?;
        return Ok(());
    }

    let bundled = bundled_assets::bundled_runtime_for(
        CTX_MCP_RUNTIME_ID,
        std::env::consts::OS,
        std::env::consts::ARCH,
    )
    .ok_or_else(|| {
        anyhow::anyhow!(
            "host ctx-mcp runtime is unavailable for {}/{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })?;
    provider_env.insert(
        CTX_MCP_COMMAND_ENV.to_string(),
        bundled.bin.to_string_lossy().to_string(),
    );
    Ok(())
}

fn validate_explicit_mcp_command(command: &str) -> Result<()> {
    let path = Path::new(command);
    if !path.is_absolute() && !looks_like_windows_absolute_path(command) {
        anyhow::bail!("CTX_MCP_COMMAND must be an explicit absolute path, got `{command}`");
    }
    if !path.exists() {
        anyhow::bail!("CTX_MCP_COMMAND path does not exist: {command}");
    }
    Ok(())
}

fn looks_like_windows_absolute_path(command: &str) -> bool {
    let bytes = command.as_bytes();
    bytes.len() >= 3
        && bytes[1] == b':'
        && bytes[0].is_ascii_alphabetic()
        && (bytes[2] == b'\\' || bytes[2] == b'/')
}

fn mcp_enabled(provider_env: &HashMap<String, String>) -> bool {
    provider_env
        .get(CTX_MCP_DISABLED_ENV)
        .and_then(|value| parse_boolish(value))
        .map(|disabled| !disabled)
        .unwrap_or(true)
}

fn provider_env_targets_linux_sandbox(provider_env: &HashMap<String, String>) -> bool {
    provider_env
        .get(ctx_harness_runtime::CTX_HARNESS_LINUX_SANDBOX_ENV)
        .is_some_and(|value| value == "1")
        || provider_env.contains_key("CTX_HARNESS_CONTAINER_ID")
}

fn stage_linux_sandbox_mcp_runtime(
    data_root: &Path,
    bundled: &bundled_assets::BundledRuntimePaths,
) -> Result<PathBuf> {
    let expected_sha256 = validate_runtime_sha256_metadata(&bundled.sha256)?;
    let runtime_dir = data_root
        .join("runtimes")
        .join(CTX_MCP_RUNTIME_ID)
        .join(&bundled.version);
    std::fs::create_dir_all(&runtime_dir).with_context(|| {
        format!(
            "creating linux sandbox ctx-mcp runtime dir {}",
            runtime_dir.display()
        )
    })?;

    let file_name = bundled
        .bin
        .file_name()
        .context("bundled ctx-mcp runtime missing binary file name")?;
    let staged_path = runtime_dir.join(file_name);
    if staged_path.exists() {
        let digest = sha256_file(&staged_path).with_context(|| {
            format!("verifying staged ctx-mcp runtime {}", staged_path.display())
        })?;
        if digest.eq_ignore_ascii_case(&expected_sha256) {
            return Ok(staged_path);
        }
        std::fs::remove_file(&staged_path).with_context(|| {
            format!(
                "removing checksum-mismatched staged ctx-mcp runtime {}",
                staged_path.display()
            )
        })?;
    }

    let bundled_digest = sha256_file(&bundled.bin).with_context(|| {
        format!(
            "verifying bundled linux sandbox ctx-mcp runtime {}",
            bundled.bin.display()
        )
    })?;
    if !bundled_digest.eq_ignore_ascii_case(&expected_sha256) {
        anyhow::bail!(
            "bundled linux sandbox ctx-mcp runtime checksum mismatch: expected {}, got {}",
            expected_sha256,
            bundled_digest
        );
    }

    let staged_tmp = runtime_dir.join(format!(
        ".{}.tmp-{}-{}",
        file_name.to_string_lossy(),
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default(),
    ));

    std::fs::copy(&bundled.bin, &staged_tmp).with_context(|| {
        format!(
            "staging linux sandbox ctx-mcp runtime from {} to {}",
            bundled.bin.display(),
            staged_tmp.display()
        )
    })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&staged_tmp)
            .with_context(|| format!("stat staged ctx-mcp runtime {}", staged_tmp.display()))?
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&staged_tmp, perms).with_context(|| {
            format!(
                "marking staged linux sandbox ctx-mcp runtime executable at {}",
                staged_tmp.display()
            )
        })?;
    }

    let staged_tmp_digest = sha256_file(&staged_tmp).with_context(|| {
        format!(
            "verifying staged linux sandbox ctx-mcp runtime {}",
            staged_tmp.display()
        )
    })?;
    if !staged_tmp_digest.eq_ignore_ascii_case(&expected_sha256) {
        let _ = std::fs::remove_file(&staged_tmp);
        anyhow::bail!(
            "staged linux sandbox ctx-mcp runtime checksum mismatch: expected {}, got {}",
            expected_sha256,
            staged_tmp_digest
        );
    }

    if let Err(err) = std::fs::rename(&staged_tmp, &staged_path) {
        if staged_path.exists() {
            let digest = sha256_file(&staged_path).with_context(|| {
                format!(
                    "verifying concurrently staged ctx-mcp runtime {}",
                    staged_path.display()
                )
            })?;
            if !digest.eq_ignore_ascii_case(&expected_sha256) {
                let _ = std::fs::remove_file(&staged_tmp);
                anyhow::bail!(
                    "concurrently staged linux sandbox ctx-mcp runtime checksum mismatch: expected {}, got {}",
                    expected_sha256,
                    digest
                );
            }
            let _ = std::fs::remove_file(&staged_tmp);
            return Ok(staged_path);
        }
        return Err(err).with_context(|| {
            format!(
                "finalizing staged linux sandbox ctx-mcp runtime {} -> {}",
                staged_tmp.display(),
                staged_path.display()
            )
        });
    }

    let final_digest = sha256_file(&staged_path).with_context(|| {
        format!(
            "verifying finalized linux sandbox ctx-mcp runtime {}",
            staged_path.display()
        )
    })?;
    if !final_digest.eq_ignore_ascii_case(&expected_sha256) {
        let _ = std::fs::remove_file(&staged_path);
        anyhow::bail!(
            "finalized linux sandbox ctx-mcp runtime checksum mismatch: expected {}, got {}",
            expected_sha256,
            final_digest
        );
    }

    Ok(staged_path)
}

fn validate_runtime_sha256_metadata(raw: &str) -> Result<String> {
    let trimmed = raw.trim().to_ascii_lowercase();
    if trimmed.len() != 64 || !trimmed.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        anyhow::bail!("linux sandbox ctx-mcp runtime is missing valid sha256 metadata");
    }
    Ok(trimmed)
}

fn sha256_file(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("reading file for sha256 {}", path.display()))?;
    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    Ok(hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctx_bundled_assets::test_support::{
        bundled_assets_manifest_test_lock, override_bundled_assets_manifest_for_test,
        BundledAssetsManifest,
    };
    use ctx_bundled_assets::BundledRuntime;

    fn current_linux_arch() -> &'static str {
        std::env::consts::ARCH
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        let mut hasher = sha2::Sha256::new();
        hasher.update(bytes);
        hex::encode(hasher.finalize())
    }

    fn bundled_runtime_entry(os: &str, arch: &str, sha256: String) -> BundledRuntime {
        BundledRuntime {
            id: "ctx-mcp".to_string(),
            version: "0.1.0".to_string(),
            os: os.to_string(),
            arch: arch.to_string(),
            sha256,
            root: format!("runtimes/ctx-mcp/{os}/{arch}"),
            bin: "ctx-mcp".to_string(),
            npm_cli: None,
        }
    }

    fn write_bundled_mcp(root: &Path, os: &str, arch: &str, contents: &[u8]) -> PathBuf {
        let runtime_root = root.join("runtimes").join("ctx-mcp").join(os).join(arch);
        std::fs::create_dir_all(&runtime_root).expect("mkdir runtime root");
        let bundled_bin = runtime_root.join("ctx-mcp");
        std::fs::write(&bundled_bin, contents).expect("write bundled ctx-mcp");
        bundled_bin
    }

    #[test]
    fn configure_runtime_mcp_command_stages_linux_runtime_for_sandbox_env() {
        let _guard = bundled_assets_manifest_test_lock()
            .lock()
            .expect("bundled assets manifest lock poisoned");
        let bundle_root = tempfile::tempdir().expect("bundle root");
        write_bundled_mcp(
            bundle_root.path(),
            "linux",
            current_linux_arch(),
            b"linux ctx-mcp",
        );

        let _bundle_guard = override_bundled_assets_manifest_for_test(
            bundle_root.path().to_path_buf(),
            BundledAssetsManifest {
                version: 1,
                generated_at: None,
                providers: vec![],
                runtimes: vec![bundled_runtime_entry(
                    "linux",
                    current_linux_arch(),
                    sha256_hex(b"linux ctx-mcp"),
                )],
                images: vec![],
            },
        );

        let data_root = tempfile::tempdir().expect("data root");
        let mut provider_env = HashMap::from([(
            "CTX_HARNESS_CONTAINER_ID".to_string(),
            "ctx-harness-123".to_string(),
        )]);

        configure_runtime_mcp_command(&mut provider_env, data_root.path())
            .expect("configure runtime mcp command");

        let configured = PathBuf::from(
            provider_env
                .get(CTX_MCP_COMMAND_ENV)
                .expect("ctx mcp command should be set"),
        );
        assert!(configured.exists(), "staged ctx-mcp path should exist");
        assert_eq!(
            std::fs::read(&configured).expect("read staged ctx-mcp"),
            b"linux ctx-mcp"
        );
        assert!(configured.starts_with(data_root.path().join("runtimes").join("ctx-mcp")));
    }

    #[test]
    fn configure_runtime_mcp_command_replaces_checksum_mismatched_staged_linux_runtime() {
        let _guard = bundled_assets_manifest_test_lock()
            .lock()
            .expect("bundled assets manifest lock poisoned");
        let bundle_root = tempfile::tempdir().expect("bundle root");
        write_bundled_mcp(
            bundle_root.path(),
            "linux",
            current_linux_arch(),
            b"fresh linux ctx-mcp",
        );

        let _bundle_guard = override_bundled_assets_manifest_for_test(
            bundle_root.path().to_path_buf(),
            BundledAssetsManifest {
                version: 1,
                generated_at: None,
                providers: vec![],
                runtimes: vec![bundled_runtime_entry(
                    "linux",
                    current_linux_arch(),
                    sha256_hex(b"fresh linux ctx-mcp"),
                )],
                images: vec![],
            },
        );

        let data_root = tempfile::tempdir().expect("data root");
        let staged = data_root
            .path()
            .join("runtimes")
            .join("ctx-mcp")
            .join("0.1.0")
            .join("ctx-mcp");
        std::fs::create_dir_all(staged.parent().expect("staged parent"))
            .expect("mkdir staged parent");
        std::fs::write(&staged, b"stale linux ctx-mcp").expect("write stale staged runtime");
        let mut provider_env = HashMap::from([(
            "CTX_HARNESS_CONTAINER_ID".to_string(),
            "ctx-harness-123".to_string(),
        )]);

        configure_runtime_mcp_command(&mut provider_env, data_root.path())
            .expect("configure runtime mcp command");

        assert_eq!(
            std::fs::read(&staged).expect("read staged ctx-mcp"),
            b"fresh linux ctx-mcp"
        );
    }

    #[test]
    fn configure_runtime_mcp_command_fails_closed_with_invalid_linux_runtime_checksum_metadata() {
        let _guard = bundled_assets_manifest_test_lock()
            .lock()
            .expect("bundled assets manifest lock poisoned");
        let bundle_root = tempfile::tempdir().expect("bundle root");
        write_bundled_mcp(
            bundle_root.path(),
            "linux",
            current_linux_arch(),
            b"linux ctx-mcp",
        );

        let _bundle_guard = override_bundled_assets_manifest_for_test(
            bundle_root.path().to_path_buf(),
            BundledAssetsManifest {
                version: 1,
                generated_at: None,
                providers: vec![],
                runtimes: vec![bundled_runtime_entry(
                    "linux",
                    current_linux_arch(),
                    "deadbeef".to_string(),
                )],
                images: vec![],
            },
        );

        let data_root = tempfile::tempdir().expect("data root");
        let mut provider_env = HashMap::from([(
            "CTX_HARNESS_CONTAINER_ID".to_string(),
            "ctx-harness-123".to_string(),
        )]);

        let err = configure_runtime_mcp_command(&mut provider_env, data_root.path())
            .expect_err("invalid checksum metadata should fail closed");
        assert!(err.to_string().contains("valid sha256 metadata"));
    }

    #[test]
    fn configure_runtime_mcp_command_uses_bundled_host_runtime() {
        let _guard = bundled_assets_manifest_test_lock()
            .lock()
            .expect("bundled assets manifest lock poisoned");
        let bundle_root = tempfile::tempdir().expect("bundle root");
        let bundled_bin = write_bundled_mcp(
            bundle_root.path(),
            std::env::consts::OS,
            std::env::consts::ARCH,
            b"host ctx-mcp",
        );
        let _bundle_guard = override_bundled_assets_manifest_for_test(
            bundle_root.path().to_path_buf(),
            BundledAssetsManifest {
                version: 1,
                generated_at: None,
                providers: vec![],
                runtimes: vec![bundled_runtime_entry(
                    std::env::consts::OS,
                    std::env::consts::ARCH,
                    sha256_hex(b"host ctx-mcp"),
                )],
                images: vec![],
            },
        );
        let data_root = tempfile::tempdir().expect("data root");
        let mut provider_env = HashMap::new();

        configure_runtime_mcp_command(&mut provider_env, data_root.path())
            .expect("configure runtime mcp command");

        assert_eq!(
            provider_env.get(CTX_MCP_COMMAND_ENV).map(String::as_str),
            Some(bundled_bin.to_string_lossy().as_ref())
        );
    }

    #[test]
    fn configure_runtime_mcp_command_skips_when_disabled() {
        let _guard = bundled_assets_manifest_test_lock()
            .lock()
            .expect("bundled assets manifest lock poisoned");
        let data_root = tempfile::tempdir().expect("data root");
        let mut provider_env = HashMap::from([
            (
                "CTX_HARNESS_CONTAINER_ID".to_string(),
                "ctx-harness-123".to_string(),
            ),
            ("CTX_MCP_DISABLED".to_string(), "1".to_string()),
        ]);

        configure_runtime_mcp_command(&mut provider_env, data_root.path())
            .expect("disabled mcp should not error");
        assert!(!provider_env.contains_key(CTX_MCP_COMMAND_ENV));
    }

    #[test]
    fn configure_runtime_mcp_command_preserves_existing_explicit_host_command() {
        let _guard = bundled_assets_manifest_test_lock()
            .lock()
            .expect("bundled assets manifest lock poisoned");
        let data_root = tempfile::tempdir().expect("data root");
        let explicit = data_root.path().join("ctx-mcp");
        std::fs::write(&explicit, b"host ctx-mcp").expect("write explicit ctx-mcp");
        let mut provider_env = HashMap::from([(
            CTX_MCP_COMMAND_ENV.to_string(),
            explicit.to_string_lossy().to_string(),
        )]);

        configure_runtime_mcp_command(&mut provider_env, data_root.path())
            .expect("existing ctx mcp command should be preserved");
        assert_eq!(
            provider_env.get(CTX_MCP_COMMAND_ENV).map(String::as_str),
            Some(explicit.to_string_lossy().as_ref())
        );
    }

    #[test]
    fn configure_runtime_mcp_command_rejects_bare_existing_host_command() {
        let _guard = bundled_assets_manifest_test_lock()
            .lock()
            .expect("bundled assets manifest lock poisoned");
        let data_root = tempfile::tempdir().expect("data root");
        let mut provider_env =
            HashMap::from([(CTX_MCP_COMMAND_ENV.to_string(), "ctx-mcp".to_string())]);

        let err = configure_runtime_mcp_command(&mut provider_env, data_root.path())
            .expect_err("bare command should fail closed");
        assert!(err
            .to_string()
            .contains("must be an explicit absolute path"));
    }

    #[test]
    fn configure_runtime_mcp_command_fails_closed_without_host_runtime() {
        let _guard = bundled_assets_manifest_test_lock()
            .lock()
            .expect("bundled assets manifest lock poisoned");
        let bundle_root = tempfile::tempdir().expect("bundle root");
        let _bundle_guard = override_bundled_assets_manifest_for_test(
            bundle_root.path().to_path_buf(),
            BundledAssetsManifest {
                version: 1,
                generated_at: None,
                providers: vec![],
                runtimes: vec![],
                images: vec![],
            },
        );
        let data_root = tempfile::tempdir().expect("data root");
        let mut provider_env = HashMap::new();

        let err = configure_runtime_mcp_command(&mut provider_env, data_root.path())
            .expect_err("missing host runtime should fail");
        assert!(err
            .to_string()
            .contains("host ctx-mcp runtime is unavailable"));
    }

    #[test]
    fn configure_runtime_mcp_command_fails_closed_without_linux_runtime() {
        let _guard = bundled_assets_manifest_test_lock()
            .lock()
            .expect("bundled assets manifest lock poisoned");
        let bundle_root = tempfile::tempdir().expect("bundle root");
        let _bundle_guard = override_bundled_assets_manifest_for_test(
            bundle_root.path().to_path_buf(),
            BundledAssetsManifest {
                version: 1,
                generated_at: None,
                providers: vec![],
                runtimes: vec![],
                images: vec![],
            },
        );
        let data_root = tempfile::tempdir().expect("data root");
        let mut provider_env = HashMap::from([(
            "CTX_HARNESS_CONTAINER_ID".to_string(),
            "ctx-harness-123".to_string(),
        )]);

        let err = configure_runtime_mcp_command(&mut provider_env, data_root.path())
            .expect_err("missing runtime should fail");
        assert!(err
            .to_string()
            .contains("linux sandbox ctx-mcp runtime is unavailable"));
    }
}
