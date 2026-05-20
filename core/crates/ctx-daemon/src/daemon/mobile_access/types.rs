use ctx_store::store::MobileDeviceSeqAdvance;
use ctx_transport_runtime::mobile_e2ee;
use serde::{Deserialize, Serialize};

pub use ctx_mobile_access_service::{
    MobileAccessConfigSnapshot, MobileAccessConfigUpsert, MobileDeviceRegistrationUpdate,
};

#[derive(Debug, Clone)]
pub struct EnableMobileAccessRequest {
    pub supabase_token: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairMobileDeviceRequest {
    pub device_id: String,
    pub public_key: String,
    pub seq: i64,
    pub nonce: String,
    pub ciphertext: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairMobileDevicePayload {
    pub pairing_token: String,
    pub device_label: Option<String>,
    pub platform: Option<String>,
    pub app_version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MobileSecureEnvelope {
    pub device_id: String,
    pub seq: i64,
    pub nonce: String,
    pub ciphertext: String,
}

impl From<mobile_e2ee::Envelope> for MobileSecureEnvelope {
    fn from(envelope: mobile_e2ee::Envelope) -> Self {
        Self {
            device_id: envelope.device_id,
            seq: envelope.seq,
            nonce: envelope.nonce_b64,
            ciphertext: envelope.ciphertext_b64,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MobileDeviceSequenceAdvance {
    Advanced,
    Stale { current: i64 },
    Missing,
}

impl From<MobileDeviceSeqAdvance> for MobileDeviceSequenceAdvance {
    fn from(outcome: MobileDeviceSeqAdvance) -> Self {
        match outcome {
            MobileDeviceSeqAdvance::Advanced => Self::Advanced,
            MobileDeviceSeqAdvance::Stale { current } => Self::Stale { current },
            MobileDeviceSeqAdvance::Missing => Self::Missing,
        }
    }
}
