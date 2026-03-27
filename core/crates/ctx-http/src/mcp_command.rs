use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ctx_core::boolish::parse_boolish;

use crate::bundled_assets;

const CTX_MCP_COMMAND_ENV: &str = "CTX_MCP_COMMAND";
const CTX_MCP_DISABLED_ENV: &str = "CTX_MCP_DISABLED";
const CTX_MCP_RUNTIME_ID: &str = "ctx-mcp";

pub(crate) fn configure_runtime_mcp_command(
    provider_env: &mut HashMap<String, String>,
    data_root: &Path,
) -> Result<()> {
    if !mcp_enabled(provider_env) || !provider_env_targets_linux_sandbox(provider_env) {
        return Ok(());
    }
    if provider_env
        .get(CTX_MCP_COMMAND_ENV)
        .map(String::as_str)
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
    {
        return Ok(());
    }

    let bundled =
        bundled_assets::bundled_runtime_for(CTX_MCP_RUNTIME_ID, "linux", std::env::consts::ARCH)
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
    Ok(())
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
        .get(crate::harness_runtime::CTX_HARNESS_LINUX_SANDBOX_ENV)
        .is_some_and(|value| value == "1")
        || provider_env.contains_key("CTX_HARNESS_CONTAINER_ID")
}

fn stage_linux_sandbox_mcp_runtime(
    data_root: &Path,
    bundled: &bundled_assets::BundledRuntimePaths,
) -> Result<PathBuf> {
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
        return Ok(staged_path);
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

    if let Err(err) = std::fs::rename(&staged_tmp, &staged_path) {
        if staged_path.exists() {
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

    Ok(staged_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bundled_assets::{
        bundled_assets_manifest_test_lock, override_bundled_assets_manifest_for_test,
        BundledAssetsManifest, BundledRuntime,
    };

    fn current_linux_arch() -> &'static str {
        std::env::consts::ARCH
    }

    #[test]
    fn configure_runtime_mcp_command_stages_linux_runtime_for_sandbox_env() {
        let _guard = bundled_assets_manifest_test_lock()
            .lock()
            .expect("bundled assets manifest test lock poisoned");
        let bundle_root = tempfile::tempdir().expect("bundle root");
        let runtime_root = bundle_root
            .path()
            .join("runtimes")
            .join("ctx-mcp")
            .join("linux")
            .join(current_linux_arch());
        std::fs::create_dir_all(&runtime_root).expect("mkdir runtime root");
        let bundled_bin = runtime_root.join("ctx-mcp");
        std::fs::write(&bundled_bin, b"linux ctx-mcp").expect("write bundled ctx-mcp");

        let _bundle_guard = override_bundled_assets_manifest_for_test(
            bundle_root.path().to_path_buf(),
            BundledAssetsManifest {
                version: 1,
                generated_at: None,
                providers: vec![],
                runtimes: vec![BundledRuntime {
                    id: "ctx-mcp".to_string(),
                    version: "0.1.0".to_string(),
                    os: "linux".to_string(),
                    arch: current_linux_arch().to_string(),
                    sha256: "deadbeef".to_string(),
                    root: format!("runtimes/ctx-mcp/linux/{}", current_linux_arch()),
                    bin: "ctx-mcp".to_string(),
                    npm_cli: None,
                }],
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
    fn configure_runtime_mcp_command_skips_when_disabled() {
        let _guard = bundled_assets_manifest_test_lock()
            .lock()
            .expect("bundled assets manifest test lock poisoned");
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
    fn configure_runtime_mcp_command_preserves_existing_command() {
        let _guard = bundled_assets_manifest_test_lock()
            .lock()
            .expect("bundled assets manifest test lock poisoned");
        let data_root = tempfile::tempdir().expect("data root");
        let mut provider_env = HashMap::from([
            (
                "CTX_HARNESS_CONTAINER_ID".to_string(),
                "ctx-harness-123".to_string(),
            ),
            (
                CTX_MCP_COMMAND_ENV.to_string(),
                "/usr/local/bin/ctx-mcp".to_string(),
            ),
        ]);

        configure_runtime_mcp_command(&mut provider_env, data_root.path())
            .expect("existing ctx mcp command should be preserved");
        assert_eq!(
            provider_env.get(CTX_MCP_COMMAND_ENV).map(String::as_str),
            Some("/usr/local/bin/ctx-mcp")
        );
    }

    #[test]
    fn configure_runtime_mcp_command_fails_closed_without_linux_runtime() {
        let _guard = bundled_assets_manifest_test_lock()
            .lock()
            .expect("bundled assets manifest test lock poisoned");
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
