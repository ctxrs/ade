use base64::Engine;
use serde::{Deserialize, Serialize};

const KEYCHAIN_SERVICE: &str = "rs.ctx.tauri.mobile";
const DEVICE_IDENTITY_ACCOUNT: &str = "managed-mobile-device-identity-v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DeviceIdentity {
    device_id: String,
    public_key: String,
    secret_key: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MobilePreparePairingRequest {
    pairing_token: String,
    daemon_public_key: String,
    device_label: Option<String>,
    platform: Option<String>,
    app_version: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MobilePairingEnvelope {
    device_id: String,
    public_key: String,
    seq: i64,
    nonce: String,
    ciphertext: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MobilePreparePairingResponse {
    device_id: String,
    public_key: String,
    envelope: MobilePairingEnvelope,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SecureEnvelope {
    device_id: String,
    seq: i64,
    nonce: String,
    ciphertext: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PairingResponsePayload {
    paired: bool,
    device_id: String,
    daemon_public_key: String,
    paired_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ServerPairingResponsePayload {
    paired: bool,
    device_id: String,
    daemon_public_key: String,
    paired_at: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MobileDecryptPairingResponseRequest {
    daemon_public_key: String,
    envelope: SecureEnvelope,
}

#[derive(Debug, Deserialize, Serialize)]
struct SecureRequestPayload {
    method: String,
    path: String,
    query: Option<String>,
    headers: Vec<(String, String)>,
    #[serde(rename = "body_b64", alias = "bodyB64")]
    body_b64: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SecureResponsePayload {
    status: u16,
    headers: Vec<(String, String)>,
    body_b64: String,
}

#[derive(Debug, Deserialize)]
struct ServerSecureResponsePayload {
    status: u16,
    headers: Vec<(String, String)>,
    body_b64: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MobileEncryptSecureRequest {
    daemon_public_key: String,
    seq: i64,
    payload: SecureRequestPayload,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MobileDecryptSecureResponseRequest {
    daemon_public_key: String,
    envelope: SecureEnvelope,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MobileDeriveStreamTokenRequest {
    daemon_public_key: String,
    workspace_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MobileDeriveStreamTokenResponse {
    device_id: String,
    token: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MobileDecryptEnvelopeRequest {
    daemon_public_key: String,
    envelope: SecureEnvelope,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MobileDecryptEnvelopeResponse {
    plaintext_b64: String,
}

fn command_error(err: impl std::fmt::Display) -> String {
    err.to_string()
}

fn load_or_create_device_identity() -> Result<DeviceIdentity, String> {
    if let Some(identity) = secure_store::load_device_identity()? {
        return Ok(identity);
    }
    let (public_key, secret_key) = ctx_transport_runtime::mobile_e2ee::generate_keypair();
    let identity = DeviceIdentity {
        device_id: uuid::Uuid::new_v4().to_string(),
        public_key,
        secret_key,
    };
    secure_store::store_device_identity(&identity)?;
    Ok(identity)
}

fn derive_client_key(
    identity: &DeviceIdentity,
    daemon_public_key: &str,
) -> Result<ctx_transport_runtime::mobile_e2ee::E2eeKey, String> {
    ctx_transport_runtime::mobile_e2ee::derive_client_key(
        &identity.device_id,
        &identity.secret_key,
        daemon_public_key.trim(),
    )
    .map_err(command_error)
}

#[tauri::command]
fn mobile_prepare_pairing_request(
    req: MobilePreparePairingRequest,
) -> Result<MobilePreparePairingResponse, String> {
    let identity = load_or_create_device_identity()?;
    let key = derive_client_key(&identity, &req.daemon_public_key)?;
    let payload = serde_json::json!({
      "pairing_token": req.pairing_token.trim(),
      "device_label": req.device_label.map(|value| value.trim().to_string()).filter(|value| !value.is_empty()),
      "platform": req.platform.map(|value| value.trim().to_string()).filter(|value| !value.is_empty()),
      "app_version": req.app_version.map(|value| value.trim().to_string()).filter(|value| !value.is_empty()),
    });
    let plaintext = serde_json::to_vec(&payload).map_err(command_error)?;
    let envelope = ctx_transport_runtime::mobile_e2ee::encrypt_pairing_request(
        &key,
        &identity.device_id,
        &identity.public_key,
        &plaintext,
    )
    .map_err(command_error)?;
    Ok(MobilePreparePairingResponse {
        device_id: identity.device_id.clone(),
        public_key: identity.public_key.clone(),
        envelope: MobilePairingEnvelope {
            device_id: envelope.device_id,
            public_key: identity.public_key,
            seq: envelope.seq,
            nonce: envelope.nonce_b64,
            ciphertext: envelope.ciphertext_b64,
        },
    })
}

#[tauri::command]
fn mobile_decrypt_pairing_response(
    req: MobileDecryptPairingResponseRequest,
) -> Result<PairingResponsePayload, String> {
    let identity = load_or_create_device_identity()?;
    if req.envelope.device_id.trim() != identity.device_id {
        return Err("pairing response device_id does not match stored mobile identity".into());
    }
    let key = derive_client_key(&identity, &req.daemon_public_key)?;
    let plaintext = ctx_transport_runtime::mobile_e2ee::decrypt(
        &key,
        &identity.device_id,
        req.envelope.seq,
        &req.envelope.nonce,
        &req.envelope.ciphertext,
    )
    .map_err(command_error)?;
    let payload = serde_json::from_slice::<ServerPairingResponsePayload>(&plaintext)
        .map_err(command_error)?;
    Ok(PairingResponsePayload {
        paired: payload.paired,
        device_id: payload.device_id,
        daemon_public_key: payload.daemon_public_key,
        paired_at: payload.paired_at,
    })
}

#[tauri::command]
fn mobile_encrypt_secure_request(
    req: MobileEncryptSecureRequest,
) -> Result<SecureEnvelope, String> {
    let identity = load_or_create_device_identity()?;
    let key = derive_client_key(&identity, &req.daemon_public_key)?;
    let plaintext = serde_json::to_vec(&req.payload).map_err(command_error)?;
    let envelope =
        ctx_transport_runtime::mobile_e2ee::encrypt(&key, &identity.device_id, req.seq, &plaintext)
            .map_err(command_error)?;
    Ok(SecureEnvelope {
        device_id: envelope.device_id,
        seq: envelope.seq,
        nonce: envelope.nonce_b64,
        ciphertext: envelope.ciphertext_b64,
    })
}

#[tauri::command]
fn mobile_decrypt_secure_response(
    req: MobileDecryptSecureResponseRequest,
) -> Result<SecureResponsePayload, String> {
    let plaintext = decrypt_for_stored_identity(&req.daemon_public_key, req.envelope)?;
    let payload =
        serde_json::from_slice::<ServerSecureResponsePayload>(&plaintext).map_err(command_error)?;
    Ok(SecureResponsePayload {
        status: payload.status,
        headers: payload.headers,
        body_b64: payload.body_b64,
    })
}

#[tauri::command]
fn mobile_derive_stream_token(
    req: MobileDeriveStreamTokenRequest,
) -> Result<MobileDeriveStreamTokenResponse, String> {
    let identity = load_or_create_device_identity()?;
    let key = derive_client_key(&identity, &req.daemon_public_key)?;
    Ok(MobileDeriveStreamTokenResponse {
        device_id: identity.device_id,
        token: ctx_transport_runtime::mobile_e2ee::derive_stream_token(&key, &req.workspace_id),
    })
}

#[tauri::command]
fn mobile_decrypt_envelope(
    req: MobileDecryptEnvelopeRequest,
) -> Result<MobileDecryptEnvelopeResponse, String> {
    let plaintext = decrypt_for_stored_identity(&req.daemon_public_key, req.envelope)?;
    Ok(MobileDecryptEnvelopeResponse {
        plaintext_b64: base64::engine::general_purpose::STANDARD.encode(plaintext),
    })
}

#[tauri::command]
fn mobile_clear_secure_identity() -> Result<(), String> {
    secure_store::clear_device_identity()
}

fn decrypt_for_stored_identity(
    daemon_public_key: &str,
    envelope: SecureEnvelope,
) -> Result<Vec<u8>, String> {
    let identity = load_or_create_device_identity()?;
    if envelope.device_id.trim() != identity.device_id {
        return Err("secure envelope device_id does not match stored mobile identity".into());
    }
    let key = derive_client_key(&identity, daemon_public_key)?;
    ctx_transport_runtime::mobile_e2ee::decrypt(
        &key,
        &identity.device_id,
        envelope.seq,
        &envelope.nonce,
        &envelope.ciphertext,
    )
    .map_err(command_error)
}

#[cfg(any(target_os = "ios", target_os = "macos"))]
mod secure_store {
    use super::{DeviceIdentity, DEVICE_IDENTITY_ACCOUNT, KEYCHAIN_SERVICE};
    use security_framework::passwords::{
        delete_generic_password, get_generic_password, set_generic_password,
    };

    const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

    pub fn load_device_identity() -> Result<Option<DeviceIdentity>, String> {
        match get_generic_password(KEYCHAIN_SERVICE, DEVICE_IDENTITY_ACCOUNT) {
            Ok(bytes) => serde_json::from_slice::<DeviceIdentity>(&bytes)
                .map(Some)
                .map_err(|err| format!("failed to parse mobile identity from Keychain: {err}")),
            Err(err) if err.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(None),
            Err(err) => Err(format!(
                "failed to read mobile identity from Keychain: {err}"
            )),
        }
    }

    pub fn store_device_identity(identity: &DeviceIdentity) -> Result<(), String> {
        let bytes = serde_json::to_vec(identity)
            .map_err(|err| format!("failed to encode mobile identity: {err}"))?;
        set_generic_password(KEYCHAIN_SERVICE, DEVICE_IDENTITY_ACCOUNT, &bytes)
            .map_err(|err| format!("failed to store mobile identity in Keychain: {err}"))
    }

    pub fn clear_device_identity() -> Result<(), String> {
        match delete_generic_password(KEYCHAIN_SERVICE, DEVICE_IDENTITY_ACCOUNT) {
            Ok(()) => Ok(()),
            Err(err) if err.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(()),
            Err(err) => Err(format!(
                "failed to clear mobile identity from Keychain: {err}"
            )),
        }
    }
}

#[cfg(not(any(target_os = "ios", target_os = "macos")))]
mod secure_store {
    use super::DeviceIdentity;

    pub fn load_device_identity() -> Result<Option<DeviceIdentity>, String> {
        Err("managed mobile QR pairing requires platform secure storage; this build does not implement it for the current OS".into())
    }

    pub fn store_device_identity(_identity: &DeviceIdentity) -> Result<(), String> {
        Err("managed mobile QR pairing requires platform secure storage; this build does not implement it for the current OS".into())
    }

    pub fn clear_device_identity() -> Result<(), String> {
        Err("managed mobile QR pairing requires platform secure storage; this build does not implement it for the current OS".into())
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();
    #[cfg(mobile)]
    let builder = builder.plugin(tauri_plugin_barcode_scanner::init());

    if let Err(err) = builder
        .invoke_handler(tauri::generate_handler![
            mobile_prepare_pairing_request,
            mobile_decrypt_pairing_response,
            mobile_encrypt_secure_request,
            mobile_decrypt_secure_response,
            mobile_derive_stream_token,
            mobile_decrypt_envelope,
            mobile_clear_secure_identity,
        ])
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            Ok(())
        })
        .run(tauri::generate_context!())
    {
        eprintln!("error while running tauri application: {err}");
        std::process::exit(1);
    }
}
