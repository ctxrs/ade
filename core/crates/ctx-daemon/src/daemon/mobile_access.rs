mod auth;
mod control_plane;
mod handle;
mod lifecycle;
mod profiles;
mod runtime;
mod secure_proxy;
mod types;

pub use auth::{
    default_mobile_profile_scopes, load_mobile_auth_context_for_profile,
    mobile_scope_set_from_strings, resolve_mobile_auth_context, verify_mobile_api_token_hash,
    MobileAuthContext, MobileAuthContextError, MobileScope, MobileScopeSet,
};
pub use lifecycle::{
    mobile_public_url_is_allowed, EnableMobileAccessResult, MobileAccessRouteError,
    MobileAccessRouteErrorKind,
};
pub use profiles::{
    CreateMobileConnectionProfileForRouteRequest, CreateMobileConnectionProfileForRouteResult,
    MobileConnectionProfileRouteParams, RegisterMobileDeviceForRouteRequest,
};
pub use runtime::{
    disable_mobile_access_runtime, mobile_access_status, start_mobile_tunnel_best_effort,
    DisableMobileAccessError, MobileAccessStatusError, MobileAccessStatusSnapshot,
    StartMobileTunnelRequest,
};
pub use types::{
    EnableMobileAccessRequest, MobileAccessConfigSnapshot, MobileAccessConfigUpsert,
    MobileDeviceRegistrationUpdate,
};

pub use ctx_mobile_access_service::{
    MobileDeviceSequenceAdvance, MobileSecureEnvelope, MobileSecureEnvelopeForRoute,
    MobileSecureProxyPayload, MobileSecureProxyResponsePayload, MobileSecureResponseEncryption,
    MobileSecureStreamAccessError, MobileSecureStreamContext, MobileSecureWorkspaceStreamAdmission,
    MobileSecureWorkspaceStreamRouteParams, OpenMobileSecureRequestResult, PairMobileDevicePayload,
    PairMobileDeviceRequest,
};
