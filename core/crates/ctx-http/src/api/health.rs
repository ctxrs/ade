use super::*;
use ctx_storage_admission::StorageGuardStatus;

const MOBILE_API_MIN_VERSION: i64 = 1;
const MOBILE_API_MAX_VERSION: i64 = 1;

#[derive(Debug, Serialize)]
pub(in crate::api) struct HealthCompatibility {
    desktop_exact_version: String,
    desktop_build_id: String,
    desktop_dev_instance_id: String,
    protocol_compatibility_token: String,
    mobile_api_min: i64,
    mobile_api_max: i64,
}

#[derive(Debug, Serialize)]
pub(in crate::api) struct HealthResp {
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
    open_file_limit: Option<ctx_resource_utilization::process_limits::OpenFileLimitSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    storage: Option<StorageGuardStatus>,
    compatibility: HealthCompatibility,
}

pub(in crate::api) async fn health(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<HealthResp>, StatusCode> {
    let identity = ctx_update_service::current_build_identity(env!("CARGO_PKG_VERSION"))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let include_sensitive = health_request_is_authorized(&state, &headers);
    Ok(Json(build_health_response(
        &state,
        identity,
        include_sensitive,
    )))
}

#[derive(Debug, Serialize)]
pub(in crate::api) struct DevClockResp {
    pub daemon_unix_ms: i64,
}

pub(in crate::api) async fn dev_clock() -> Json<DevClockResp> {
    Json(DevClockResp {
        daemon_unix_ms: chrono::Utc::now().timestamp_millis(),
    })
}

fn health_request_is_authorized(state: &Arc<AppState>, headers: &HeaderMap) -> bool {
    let Some(expected) = state.core.auth_token.as_deref() else {
        return true;
    };
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .is_some_and(|value| value == expected)
}

pub(in crate::api) fn build_health_response(
    state: &Arc<AppState>,
    identity: &ctx_update_service::BuildIdentity,
    include_sensitive: bool,
) -> HealthResp {
    let version = identity.exact_version.clone();
    let compatibility_token = if include_sensitive {
        identity.compatibility_token.clone()
    } else {
        String::new()
    };
    HealthResp {
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
