use std::sync::Arc;

use ctx_core::ids::ConnectionProfileId;
use ctx_core::models::{MobileConnectionProfile, MobileDeviceRegistration};
use ctx_mobile_access_service::{
    route_contract::{
        CreateMobileConnectionProfileForRouteRequest, CreateMobileConnectionProfileForRouteResult,
        MobileAccessRouteError, MobileConnectionProfileRouteParams,
        RegisterMobileDeviceForRouteRequest,
    },
    MobileAuthContext,
};

use crate::daemon::DaemonState;

pub(super) async fn create_mobile_connection_profile_for_route(
    state: &Arc<DaemonState>,
    request: CreateMobileConnectionProfileForRouteRequest,
) -> Result<CreateMobileConnectionProfileForRouteResult, MobileAccessRouteError> {
    ctx_mobile_access_service::create_mobile_connection_profile(
        state.global_store(),
        request.into(),
    )
    .await
    .map(Into::into)
    .map_err(Into::into)
}

pub(super) async fn list_mobile_connection_profiles_for_route(
    state: &Arc<DaemonState>,
) -> Result<Vec<MobileConnectionProfile>, MobileAccessRouteError> {
    ctx_mobile_access_service::list_mobile_connection_profiles(state.global_store())
        .await
        .map_err(Into::into)
}

pub(super) async fn delete_mobile_connection_profile_for_route(
    state: &Arc<DaemonState>,
    profile_id: ConnectionProfileId,
) -> Result<(), MobileAccessRouteError> {
    ctx_mobile_access_service::delete_mobile_connection_profile(state.global_store(), profile_id)
        .await
        .map_err(Into::into)
}

pub(super) async fn delete_mobile_connection_profile_for_route_params(
    state: &Arc<DaemonState>,
    params: MobileConnectionProfileRouteParams,
) -> Result<(), MobileAccessRouteError> {
    let profile_id = params.into_profile_id()?;
    delete_mobile_connection_profile_for_route(state, profile_id).await
}

pub(super) async fn list_mobile_devices_for_profile_for_route(
    state: &Arc<DaemonState>,
    profile_id: ConnectionProfileId,
) -> Result<Vec<MobileDeviceRegistration>, MobileAccessRouteError> {
    ctx_mobile_access_service::list_mobile_devices_for_profile(state.global_store(), profile_id)
        .await
        .map_err(Into::into)
}

pub(super) async fn list_mobile_devices_for_profile_for_route_params(
    state: &Arc<DaemonState>,
    params: MobileConnectionProfileRouteParams,
) -> Result<Vec<MobileDeviceRegistration>, MobileAccessRouteError> {
    let profile_id = params.into_profile_id()?;
    list_mobile_devices_for_profile_for_route(state, profile_id).await
}

pub(super) async fn register_mobile_device_for_route(
    state: &Arc<DaemonState>,
    auth: MobileAuthContext,
    request: RegisterMobileDeviceForRouteRequest,
) -> Result<MobileDeviceRegistration, MobileAccessRouteError> {
    ctx_mobile_access_service::register_mobile_device(state.global_store(), auth, request.into())
        .await
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_params_reject_invalid_uuid() {
        let error = MobileConnectionProfileRouteParams::new("not-a-uuid")
            .into_profile_id()
            .unwrap_err();
        assert_eq!(
            error.kind(),
            ctx_mobile_access_service::route_contract::MobileAccessRouteErrorKind::BadRequest
        );
        assert_eq!(error.message(), "connection profile id must be a UUID");
    }
}
