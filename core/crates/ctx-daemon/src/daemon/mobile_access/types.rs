pub use ctx_mobile_access_service::{
    MobileAccessConfigSnapshot, MobileAccessConfigUpsert, MobileDeviceRegistrationUpdate,
};

#[derive(Debug, Clone)]
pub struct EnableMobileAccessRequest {
    pub supabase_token: String,
}
