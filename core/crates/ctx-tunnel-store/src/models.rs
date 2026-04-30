use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{ensure_non_empty, validate_url, TunnelStoreError};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayRegistration {
    pub relay_id: String,
    pub region: String,
    pub public_base_url: String,
    pub internal_base_url: String,
    pub max_active_tunnels: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayHeartbeat {
    pub relay_id: String,
    pub active_tunnel_count: i32,
    pub observed_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayRecord {
    pub relay_id: String,
    pub region: String,
    pub public_base_url: String,
    pub internal_base_url: String,
    pub active_tunnel_count: i32,
    pub max_active_tunnels: i32,
    pub last_heartbeat_at: Option<DateTime<Utc>>,
    pub heartbeat_expires_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateTunnelRequest {
    pub tunnel_id: String,
    pub user_id: String,
    pub billing_subject_id: Option<String>,
    pub relay_id: String,
    pub public_base_url: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TunnelAssignment {
    pub tunnel_id: String,
    pub user_id: String,
    pub billing_subject_id: Option<String>,
    pub relay_id: String,
    pub relay_public_base_url: String,
    pub relay_internal_base_url: String,
    pub public_base_url: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedTunnelTarget {
    pub tunnel_id: String,
    pub relay_id: String,
    pub relay_public_base_url: String,
    pub relay_internal_base_url: String,
    pub public_base_url: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TunnelResolveResult {
    Resolved(ResolvedTunnelTarget),
    UnknownTunnel,
    TunnelDisabled,
    RelayUnavailable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RelayAssignmentValidation {
    Valid,
    UnknownTunnel,
    TunnelDisabled,
    WrongRelay { expected_relay_id: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthSnapshot {
    pub relay_count: i64,
    pub healthy_relay_count: i64,
}

impl RelayRegistration {
    pub(crate) fn validate(&self) -> Result<(), TunnelStoreError> {
        ensure_non_empty("relay_id", &self.relay_id)?;
        ensure_non_empty("region", &self.region)?;
        validate_url("public_base_url", &self.public_base_url)?;
        validate_url("internal_base_url", &self.internal_base_url)?;
        if self.max_active_tunnels <= 0 {
            return Err(TunnelStoreError::InvalidInput(
                "max_active_tunnels must be positive".to_string(),
            ));
        }
        Ok(())
    }
}

impl CreateTunnelRequest {
    pub(crate) fn validate(&self) -> Result<(), TunnelStoreError> {
        ensure_non_empty("tunnel_id", &self.tunnel_id)?;
        ensure_non_empty("user_id", &self.user_id)?;
        ensure_non_empty("relay_id", &self.relay_id)?;
        validate_url("public_base_url", &self.public_base_url)?;
        if let Some(subject) = self.billing_subject_id.as_ref() {
            ensure_non_empty("billing_subject_id", subject)?;
        }
        Ok(())
    }
}
