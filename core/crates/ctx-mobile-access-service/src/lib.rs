mod auth;
mod lifecycle;
mod profiles;
mod secure;
mod tokens;
mod types;

pub use auth::{
    default_mobile_profile_scopes, load_mobile_auth_context_for_profile,
    mobile_scope_set_from_strings, resolve_mobile_auth_context, verify_mobile_api_token_hash,
    MobileAuthContext, MobileAuthContextError, MobileScope, MobileScopeSet,
};
pub use lifecycle::{
    finish_mobile_access_disable_cleanup, persist_mobile_access_disable_cleanup,
    persist_mobile_access_disabled_state, persist_mobile_access_enable_bootstrap,
    MobileAccessDisableCleanupError, MobileAccessDisablePersistedStateError,
    MobileAccessDisableRemainingCleanupError, PersistMobileAccessEnableBootstrapRequest,
    PersistMobileAccessEnableBootstrapResult, PersistedMobileAccessDisable,
};
pub use profiles::{
    create_mobile_connection_profile, delete_mobile_connection_profile,
    list_mobile_connection_profiles, list_mobile_devices_for_profile, register_mobile_device,
    CreateMobileConnectionProfileRequest, CreateMobileConnectionProfileResult,
    RegisterMobileDeviceRequest,
};
pub use secure::{
    admit_mobile_secure_workspace_stream, encrypt_mobile_secure_response,
    open_mobile_secure_request, pair_mobile_device, prepare_mobile_secure_proxy_request,
    require_mobile_secure_stream_access, MobileSecureProxyAdmission,
    MobileSecureProxyAdmittedRequest, MobileSecureProxyDenyReason, MobileSecureStreamAccessError,
};
pub use tokens::{
    generate_mobile_api_token, generate_pairing_token, hash_api_token, hash_pairing_token,
};
pub use types::{
    MobileAccessConfigSnapshot, MobileAccessConfigUpsert, MobileDeviceRegistrationUpdate,
    MobileDeviceSequenceAdvance, MobileSecureEnvelope, MobileSecureEnvelopeForRoute,
    MobileSecureProxyPayload, MobileSecureProxyResponsePayload, MobileSecureResponseEncryption,
    MobileSecureStreamContext, MobileSecureWorkspaceStreamAdmission,
    MobileSecureWorkspaceStreamRouteParams, OpenMobileSecureRequestResult, PairMobileDevicePayload,
    PairMobileDeviceRequest,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MobileAccessServiceErrorKind {
    BadRequest,
    Unauthorized,
    Conflict,
    NotFound,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MobileAccessServiceError {
    kind: MobileAccessServiceErrorKind,
    message: String,
}

impl MobileAccessServiceError {
    pub fn new(kind: MobileAccessServiceErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(MobileAccessServiceErrorKind::BadRequest, message)
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(MobileAccessServiceErrorKind::Unauthorized, message)
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(MobileAccessServiceErrorKind::Conflict, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(MobileAccessServiceErrorKind::NotFound, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(MobileAccessServiceErrorKind::Internal, message)
    }

    pub fn kind(&self) -> MobileAccessServiceErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}
