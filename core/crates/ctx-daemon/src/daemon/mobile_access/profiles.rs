use ctx_core::ids::ConnectionProfileId;
use ctx_core::models::{MobileConnectionProfile, MobileDeviceRegistration};
use ctx_mobile_access_service::{
    route_contract::{
        CreateMobileConnectionProfileForRouteRequest, CreateMobileConnectionProfileForRouteResult,
        MobileAccessRouteError, MobileAccessRouteErrorKind, MobileConnectionProfileRouteParams,
        RegisterMobileDeviceForRouteRequest,
    },
    MobileAuthContext, MobileScope,
};
use ctx_store::Store;

pub(super) async fn create_mobile_connection_profile_for_route(
    store: &Store,
    request: CreateMobileConnectionProfileForRouteRequest,
) -> Result<CreateMobileConnectionProfileForRouteResult, MobileAccessRouteError> {
    let scopes = ctx_mobile_access_service::normalize_mobile_profile_scopes(&request.scopes)
        .map_err(|error| MobileAccessRouteError::bad_request(error.to_string()))?;
    let request = ctx_mobile_access_service::CreateMobileConnectionProfileRequest {
        label: request.label,
        base_url: request.base_url,
        scopes: Some(scopes),
    };
    ctx_mobile_access_service::create_mobile_connection_profile(store, request)
        .await
        .map(Into::into)
        .map_err(Into::into)
}

pub(super) async fn list_mobile_connection_profiles_for_route(
    store: &Store,
) -> Result<Vec<MobileConnectionProfile>, MobileAccessRouteError> {
    ctx_mobile_access_service::list_mobile_connection_profiles(store)
        .await
        .map_err(Into::into)
}

pub(super) async fn delete_mobile_connection_profile_for_route(
    store: &Store,
    profile_id: ConnectionProfileId,
) -> Result<(), MobileAccessRouteError> {
    let existing = store
        .get_mobile_connection_profile(profile_id)
        .await
        .map_err(|_| {
            MobileAccessRouteError::internal("failed to read mobile connection profile")
        })?;
    if existing.is_none() {
        return Err(MobileAccessRouteError::new(
            MobileAccessRouteErrorKind::NotFound,
            "mobile connection profile not found",
        ));
    }
    ctx_mobile_access_service::delete_mobile_connection_profile(store, profile_id)
        .await
        .map_err(Into::into)
}

pub(super) async fn delete_mobile_connection_profile_for_route_params(
    store: &Store,
    params: MobileConnectionProfileRouteParams,
) -> Result<(), MobileAccessRouteError> {
    let profile_id = params.into_profile_id()?;
    delete_mobile_connection_profile_for_route(store, profile_id).await
}

pub(super) async fn list_mobile_devices_for_profile_for_route(
    store: &Store,
    profile_id: ConnectionProfileId,
) -> Result<Vec<MobileDeviceRegistration>, MobileAccessRouteError> {
    ctx_mobile_access_service::list_mobile_devices_for_profile(store, profile_id)
        .await
        .map_err(Into::into)
}

pub(super) async fn list_mobile_devices_for_profile_for_route_params(
    store: &Store,
    params: MobileConnectionProfileRouteParams,
) -> Result<Vec<MobileDeviceRegistration>, MobileAccessRouteError> {
    let profile_id = params.into_profile_id()?;
    list_mobile_devices_for_profile_for_route(store, profile_id).await
}

pub(super) async fn register_mobile_device_for_route(
    store: &Store,
    auth: MobileAuthContext,
    request: RegisterMobileDeviceForRouteRequest,
) -> Result<MobileDeviceRegistration, MobileAccessRouteError> {
    if !auth.has_scope(MobileScope::RegisterDevice) {
        return Err(MobileAccessRouteError::unauthorized(
            MobileScope::RegisterDevice.missing_error(),
        ));
    }
    ctx_mobile_access_service::register_mobile_device(store, auth, request.into())
        .await
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_params_reject_invalid_uuid() {
        let error = MobileConnectionProfileRouteParams::new("not-a-uuid".to_string())
            .into_profile_id()
            .unwrap_err();
        assert_eq!(
            error.kind(),
            ctx_mobile_access_service::route_contract::MobileAccessRouteErrorKind::BadRequest
        );
        assert_eq!(error.message(), "connection profile id must be a UUID");
    }
}
