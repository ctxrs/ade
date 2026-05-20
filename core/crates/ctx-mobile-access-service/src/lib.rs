mod auth;
mod profiles;
mod tokens;
mod types;

pub use auth::{
    default_mobile_profile_scopes, load_mobile_auth_context_for_profile,
    mobile_scope_set_from_strings, resolve_mobile_auth_context, verify_mobile_api_token_hash,
    MobileAuthContext, MobileAuthContextError, MobileScope, MobileScopeSet,
};
pub use profiles::{
    create_mobile_connection_profile, delete_mobile_connection_profile,
    list_mobile_connection_profiles, list_mobile_devices_for_profile, register_mobile_device,
    CreateMobileConnectionProfileRequest, CreateMobileConnectionProfileResult,
    RegisterMobileDeviceRequest,
};
pub use tokens::{
    generate_mobile_api_token, generate_pairing_token, hash_api_token, hash_pairing_token,
};
pub use types::{
    MobileAccessConfigSnapshot, MobileAccessConfigUpsert, MobileDeviceRegistrationUpdate,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MobileAccessServiceErrorKind {
    BadRequest,
    Unauthorized,
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
