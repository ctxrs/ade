use serde::Serialize;

use ctx_resource_utilization::process_limits::OpenFileLimitSnapshot;
use ctx_storage_admission::StorageGuardStatus;
use ctx_update_service::BuildIdentity;

use crate::daemon::{CoreHandle, DaemonState};

const MOBILE_API_MIN_VERSION: i64 = 1;
const MOBILE_API_MAX_VERSION: i64 = 1;

pub type HealthSnapshotError = anyhow::Error;

#[derive(Debug, Serialize)]
pub struct HealthCompatibility {
    desktop_exact_version: String,
    desktop_build_id: String,
    desktop_dev_instance_id: String,
    protocol_compatibility_token: String,
    mobile_api_min: i64,
    mobile_api_max: i64,
}

#[derive(Debug, Serialize)]
pub struct DaemonHealthSnapshot {
    version: String,
    daemon_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data_root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    daemon_url: Option<String>,
    auth_required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    open_file_limit: Option<OpenFileLimitSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    storage: Option<StorageGuardStatus>,
    compatibility: HealthCompatibility,
}

fn build_health_snapshot(
    state: &DaemonState,
    identity: &BuildIdentity,
    include_sensitive: bool,
) -> DaemonHealthSnapshot {
    let version = identity.exact_version.clone();
    let compatibility_token = if include_sensitive {
        identity.compatibility_token.clone()
    } else {
        String::new()
    };

    DaemonHealthSnapshot {
        version: version.clone(),
        daemon_version: version.clone(),
        pid: include_sensitive.then_some(std::process::id()),
        data_root: include_sensitive.then(|| state.core.data_root.to_string_lossy().to_string()),
        daemon_url: include_sensitive.then(|| state.core.daemon_url.clone()),
        auth_required: state.core.auth_token.is_some(),
        open_file_limit: if include_sensitive {
            ctx_resource_utilization::process_limits::current_open_file_limit()
        } else {
            None
        },
        storage: include_sensitive.then(|| state.storage_guard_snapshot()),
        compatibility: HealthCompatibility {
            desktop_exact_version: version,
            desktop_build_id: identity.build_id.clone(),
            desktop_dev_instance_id: compatibility_token.clone(),
            protocol_compatibility_token: compatibility_token,
            mobile_api_min: MOBILE_API_MIN_VERSION,
            mobile_api_max: MOBILE_API_MAX_VERSION,
        },
    }
}

impl CoreHandle {
    pub fn health_snapshot(
        &self,
        package_version: &'static str,
        include_sensitive: bool,
    ) -> Result<DaemonHealthSnapshot, HealthSnapshotError> {
        let identity = ctx_update_service::current_build_identity(package_version)?;
        Ok(build_health_snapshot(
            self.state.as_ref(),
            identity,
            include_sensitive,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity_for_test(package_version: &'static str) -> BuildIdentity {
        BuildIdentity {
            schema_version: 1,
            exact_version: package_version.to_string(),
            build_id: package_version.to_string(),
            compatibility_token: "test-compatibility-token".to_string(),
        }
    }

    #[test]
    fn health_snapshot_identity_uses_caller_supplied_build_identity() {
        let identity = identity_for_test("ctx-http-package-version");
        let compatibility = HealthCompatibility {
            desktop_exact_version: identity.exact_version.clone(),
            desktop_build_id: identity.build_id.clone(),
            desktop_dev_instance_id: identity.compatibility_token.clone(),
            protocol_compatibility_token: identity.compatibility_token.clone(),
            mobile_api_min: MOBILE_API_MIN_VERSION,
            mobile_api_max: MOBILE_API_MAX_VERSION,
        };

        let serialized = serde_json::to_value(compatibility).unwrap();
        assert_eq!(
            serialized["desktop_exact_version"].as_str(),
            Some("ctx-http-package-version")
        );
        assert_eq!(
            serialized["desktop_build_id"].as_str(),
            Some("ctx-http-package-version")
        );
    }
}
