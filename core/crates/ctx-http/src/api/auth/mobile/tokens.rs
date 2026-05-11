use base64::Engine;
use rand_core::RngCore;
use sha2::Digest;

pub(in crate::api) fn hash_api_token(token: &str) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

pub(in crate::api) fn hash_pairing_token(token: &str) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

pub(in crate::api) fn generate_mobile_api_token() -> String {
    format!("ctxm_{}", uuid::Uuid::new_v4().to_string().replace('-', ""))
}

pub(in crate::api) fn generate_pairing_token() -> String {
    let mut bytes = [0u8; 32];
    rand_core::OsRng.fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}
