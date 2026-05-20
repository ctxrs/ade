use std::collections::HashMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use anyhow::{Context, Result};
use semver::Version;
use serde::{Deserialize, Serialize};

mod appimage;
mod build_identity;
mod drain;
mod fs_ops;
mod managed_daemon;
mod manifest;
pub mod route_contract;
mod self_update;

pub use appimage::{
    appimage_candidate_meta_path, appimage_candidate_partial_path, appimage_candidate_path,
    appimage_path_env, atomic_replace_file, clear_appimage_candidate, download_and_verify,
    download_verified_appimage_candidate, read_verified_appimage_candidate_meta, updates_dir,
    validate_verified_appimage_candidate, AppImageCandidateRequest, VerifiedAppImageCandidateMeta,
};
pub use build_identity::{current_build_identity, BuildIdentity, BUILD_IDENTITY_PATH_ENV};
pub use drain::{UpdateDrainCoordinator, UpdateDrainState};
pub use fs_ops::{
    atomic_replace_exe, atomic_replace_exe_with_backup, download_to_path, sha256_hex_file,
};
pub use managed_daemon::{
    managed_daemon_auto_update_configured_from_env, managed_daemon_auto_update_status_snapshot,
    spawn_managed_daemon_auto_update, ManagedDaemonAutoUpdateConfig, ManagedDaemonAutoUpdateHooks,
    ManagedDaemonAutoUpdateStatus,
};
pub use manifest::{
    default_download_base_url, fetch_latest_manifest, fetch_latest_manifest_with_params,
    in_place_update_capability, is_update_available, join_url, normalize_release_artifact_sha256,
    normalize_release_channel, normalize_version_str, platform_key, platform_supported,
    release_manifest_url, resolve_release_artifact_url, ReleaseArtifact, ReleaseManifest,
    ReleasePlatform,
};
pub use self_update::self_update_daemon;

#[cfg(test)]
mod tests {
    use super::appimage::in_place_update_capability_with_appimage_path;
    use super::managed_daemon::{
        activate_managed_daemon_bundle, managed_daemon_auto_update_source_from_env,
        restore_managed_daemon_bundle, write_managed_daemon_auto_update_status,
        ManagedDaemonAutoUpdateSource,
    };
    use super::*;

    struct EnvGuard {
        key: &'static str,
        previous: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, previous }
        }

        fn remove(key: &'static str) -> Self {
            let previous = std::env::var(key).ok();
            std::env::remove_var(key);
            Self { key, previous }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(value) = self.previous.take() {
                std::env::set_var(self.key, value);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    fn artifact(path: &str) -> ReleaseArtifact {
        ReleaseArtifact {
            url_path: path.to_string(),
            sha256: "sha".to_string(),
        }
    }

    fn release_platform_with_dmg() -> ReleasePlatform {
        ReleasePlatform {
            desktop: None,
            appimage: None,
            deb: None,
            dmg: Some(artifact("ctx.dmg")),
            msi: None,
            nsis: None,
            exe: None,
            zip: None,
            daemon: Some(artifact("ctx-daemon")),
        }
    }

    fn release_platform_daemon_only() -> ReleasePlatform {
        ReleasePlatform {
            desktop: None,
            appimage: None,
            deb: None,
            dmg: None,
            msi: None,
            nsis: None,
            exe: None,
            zip: None,
            daemon: Some(artifact("ctx-daemon")),
        }
    }

    fn release_platform_with_appimage() -> ReleasePlatform {
        ReleasePlatform {
            desktop: None,
            appimage: Some(artifact("ctx.AppImage")),
            deb: None,
            dmg: None,
            msi: None,
            nsis: None,
            exe: None,
            zip: None,
            daemon: Some(artifact("ctx-daemon")),
        }
    }

    fn manifest_with_platforms(platforms: HashMap<String, ReleasePlatform>) -> ReleaseManifest {
        ReleaseManifest {
            channel: "stable".to_string(),
            latest_version: "1.2.3".to_string(),
            min_supported_version: None,
            published_at: "2026-02-19T00:00:00Z".to_string(),
            platforms,
        }
    }

    #[tokio::test]
    async fn managed_daemon_auto_update_status_is_persisted_for_diagnostics() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = ManagedDaemonAutoUpdateSource {
            channel: "canary".to_string(),
            base_url: "https://updates.example/functions/v1".to_string(),
        };
        write_managed_daemon_auto_update_status(
            temp.path(),
            &source,
            "failed",
            Some("network unavailable".to_string()),
        )
        .await;

        let status = managed_daemon_auto_update_status_snapshot(temp.path())
            .await
            .expect("status");
        assert!(status.enabled);
        assert_eq!(status.channel, "canary");
        assert_eq!(status.state, "failed");
        assert_eq!(status.last_error.as_deref(), Some("network unavailable"));
    }

    #[test]
    fn managed_daemon_bundle_activation_restores_previous_bundle() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bundle_dir = temp.path().join("bundles");
        let staged_dir = temp.path().join("staged");
        std::fs::create_dir_all(&bundle_dir).expect("bundle dir");
        std::fs::write(bundle_dir.join("provider_matrix.json"), b"old").expect("old bundle");
        std::fs::create_dir_all(&staged_dir).expect("staged dir");
        std::fs::write(staged_dir.join("provider_matrix.json"), b"new").expect("new bundle");

        let activation =
            activate_managed_daemon_bundle(bundle_dir.clone(), staged_dir).expect("activate");
        assert_eq!(
            std::fs::read(bundle_dir.join("provider_matrix.json")).expect("read active"),
            b"new"
        );

        restore_managed_daemon_bundle(&activation).expect("restore");
        assert_eq!(
            std::fs::read(bundle_dir.join("provider_matrix.json")).expect("read restored"),
            b"old"
        );
        assert!(!activation.backup_dir.exists());
    }

    #[test]
    fn preferred_desktop_artifact_follows_platform_order() {
        let platform = ReleasePlatform {
            desktop: Some(artifact("/desktop")),
            appimage: Some(artifact("/appimage")),
            deb: Some(artifact("/deb")),
            dmg: Some(artifact("/dmg")),
            msi: Some(artifact("/msi")),
            nsis: Some(artifact("/nsis")),
            exe: Some(artifact("/exe")),
            zip: Some(artifact("/zip")),
            daemon: Some(artifact("/daemon")),
        };

        let linux = platform
            .preferred_desktop_artifact("linux-x64")
            .expect("linux artifact");
        assert_eq!(linux.url_path, "/desktop");

        let mac = platform
            .preferred_desktop_artifact("macos-arm64")
            .expect("mac artifact");
        assert_eq!(mac.url_path, "/desktop");

        let windows = platform
            .preferred_desktop_artifact("windows-x64")
            .expect("windows artifact");
        assert_eq!(windows.url_path, "/desktop");
    }

    #[test]
    fn preferred_desktop_artifact_uses_fallbacks() {
        let linux = ReleasePlatform {
            desktop: None,
            appimage: Some(artifact("/appimage")),
            deb: Some(artifact("/deb")),
            dmg: None,
            msi: None,
            nsis: None,
            exe: None,
            zip: None,
            daemon: Some(artifact("/daemon")),
        };
        assert_eq!(
            linux
                .preferred_desktop_artifact("linux-arm64")
                .expect("linux fallback")
                .url_path,
            "/appimage"
        );

        let windows = ReleasePlatform {
            desktop: None,
            appimage: None,
            deb: None,
            dmg: None,
            msi: Some(artifact("/msi")),
            nsis: None,
            exe: None,
            zip: None,
            daemon: Some(artifact("/daemon")),
        };
        assert_eq!(
            windows
                .preferred_desktop_artifact("windows-x64")
                .expect("windows fallback")
                .url_path,
            "/msi"
        );
    }

    #[test]
    fn normalize_version_accepts_v_prefix() {
        assert_eq!(
            normalize_version_str("v1.2.3").expect("semver").to_string(),
            "1.2.3"
        );
    }

    #[test]
    fn platform_supported_false_when_platform_missing() {
        let manifest = manifest_with_platforms(HashMap::new());
        assert!(!platform_supported(&manifest, Some("macos-arm64")));
    }

    #[test]
    fn platform_supported_false_when_no_desktop_artifact() {
        let mut platforms = HashMap::new();
        platforms.insert("macos-arm64".to_string(), release_platform_daemon_only());
        let manifest = manifest_with_platforms(platforms);
        assert!(!platform_supported(&manifest, Some("macos-arm64")));
    }

    #[test]
    fn platform_supported_true_with_matching_desktop_artifact() {
        let mut platforms = HashMap::new();
        platforms.insert("macos-arm64".to_string(), release_platform_with_dmg());
        let manifest = manifest_with_platforms(platforms);
        assert!(platform_supported(&manifest, Some("macos-arm64")));
    }

    #[test]
    fn update_available_requires_supported_platform() {
        assert!(!is_update_available("1.0.0", "1.2.3", false));
        assert!(is_update_available("1.0.0", "1.2.3", true));
    }

    #[test]
    fn managed_daemon_auto_update_requires_explicit_release_source() {
        let _enabled = EnvGuard::set("CTX_MANAGED_DAEMON_AUTO_UPDATE", "1");
        let _channel = EnvGuard::remove("CTX_DAEMON_UPDATE_CHANNEL");
        let _base = EnvGuard::remove("CTX_DAEMON_UPDATE_BASE_URL");
        assert!(managed_daemon_auto_update_source_from_env().is_none());

        std::env::set_var("CTX_DAEMON_UPDATE_CHANNEL", "canary");
        std::env::set_var(
            "CTX_DAEMON_UPDATE_BASE_URL",
            "https://updates.example/functions/v1",
        );
        let source = managed_daemon_auto_update_source_from_env()
            .expect("explicit release source should enable managed daemon auto-update");
        assert_eq!(source.channel, "canary");
        assert_eq!(source.base_url, "https://updates.example/functions/v1");
    }

    #[test]
    fn release_artifact_url_preserves_configured_base_path_for_root_relative_refs() {
        let url = resolve_release_artifact_url(
            "https://api.ctx.rs/functions/v1",
            "/download/stable/9.9.9/ctx.AppImage",
        )
        .expect("artifact URL");
        assert_eq!(
            url,
            "https://api.ctx.rs/functions/v1/download/stable/9.9.9/ctx.AppImage"
        );
    }

    #[test]
    fn release_artifact_url_rejects_same_origin_paths_outside_configured_base_path() {
        let err = resolve_release_artifact_url(
            "https://api.ctx.rs/functions/v1",
            "https://api.ctx.rs/download/stable/9.9.9/ctx.AppImage",
        )
        .expect_err("same-origin artifact outside release base path should fail");
        assert!(err.to_string().contains("base path"));
    }

    #[test]
    fn normalize_release_artifact_sha256_rejects_path_material() {
        let err = normalize_release_artifact_sha256(
            "../aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .expect_err("invalid digest should fail");
        assert!(err.to_string().contains("64-character hex"));
    }

    #[test]
    fn normalize_release_channel_rejects_path_traversal() {
        let err = normalize_release_channel("x/../../../secret").expect_err("invalid channel");
        assert!(err.to_string().contains("ASCII letters"));
    }

    #[test]
    fn normalize_release_channel_rejects_dot_segments() {
        let err = normalize_release_channel("..").expect_err("invalid channel");
        assert!(err.to_string().contains("must not be"));

        let err = normalize_release_channel(".").expect_err("invalid channel");
        assert!(err.to_string().contains("must not be"));
    }

    #[test]
    fn in_place_update_capability_requires_linux_appimage_and_runtime_support() {
        let mut platforms = HashMap::new();
        platforms.insert("linux-x64".to_string(), release_platform_with_appimage());
        let manifest = manifest_with_platforms(platforms);

        let (supported_without_runtime, reason_without_runtime) =
            in_place_update_capability_with_appimage_path(
                &manifest,
                Some("linux-x64"),
                true,
                false,
            );
        assert!(!supported_without_runtime);
        assert_eq!(
            reason_without_runtime.as_deref(),
            Some("current install cannot apply AppImage in place")
        );

        let (supported_with_runtime, reason_with_runtime) =
            in_place_update_capability_with_appimage_path(&manifest, Some("linux-x64"), true, true);
        assert!(supported_with_runtime);
        assert!(reason_with_runtime.is_none());
    }

    #[test]
    fn in_place_update_capability_is_false_when_platform_is_not_supported() {
        let mut platforms = HashMap::new();
        platforms.insert("linux-x64".to_string(), release_platform_with_appimage());
        let manifest = manifest_with_platforms(platforms);

        let (supported, reason) = in_place_update_capability_with_appimage_path(
            &manifest,
            Some("linux-x64"),
            false,
            true,
        );
        assert!(!supported);
        assert_eq!(
            reason.as_deref(),
            Some("no compatible desktop artifact for this platform")
        );
    }

    #[test]
    fn in_place_update_capability_is_false_for_non_linux_platforms() {
        let mut platforms = HashMap::new();
        platforms.insert("macos-arm64".to_string(), release_platform_with_dmg());
        let manifest = manifest_with_platforms(platforms);

        let (supported, reason) = in_place_update_capability_with_appimage_path(
            &manifest,
            Some("macos-arm64"),
            true,
            true,
        );
        assert!(!supported);
        assert_eq!(
            reason.as_deref(),
            Some("in-place updates are only supported for Linux AppImage installs")
        );
    }
}
