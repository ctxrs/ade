use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::{database_error, TunnelStore, TunnelStoreError};

pub const DEFAULT_TUNNEL_EVENT_RETENTION_DAYS: i64 = 90;
pub const DEFAULT_INACTIVE_TUNNEL_RETENTION_DAYS: i64 = 90;

const RETENTION_DELETE_EVENTS_SQL: &str = r#"
            delete from public.mobile_tunnel_event
             where created_at < $1
            "#;
pub(crate) const RETENTION_DELETE_INACTIVE_TUNNELS_SQL: &str = r#"
            delete from public.mobile_tunnel as t
             where t.status = 'revoked'
               and t.disabled_at is not null
               and t.updated_at < $1
               and not exists (
                 select 1
                   from public.mobile_tunnel_event e
                  where e.tunnel_id = t.tunnel_id
                    and e.created_at >= $1
               )
            "#;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetentionCleanupPolicy {
    pub event_retention_days: i64,
    pub inactive_tunnel_retention_days: i64,
    pub now: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetentionCleanupOutcome {
    pub deleted_events: u64,
    pub deleted_inactive_tunnels: u64,
}

impl TunnelStore {
    pub async fn cleanup_retention(
        &self,
        policy: RetentionCleanupPolicy,
    ) -> Result<RetentionCleanupOutcome, TunnelStoreError> {
        let event_cutoff = policy.event_cutoff()?;
        let inactive_tunnel_cutoff = policy.inactive_tunnel_cutoff()?;
        let mut tx = self.pool().begin().await.map_err(database_error)?;
        let deleted_inactive_tunnels = sqlx::query(RETENTION_DELETE_INACTIVE_TUNNELS_SQL)
            .bind(inactive_tunnel_cutoff)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?
            .rows_affected();
        let deleted_events = sqlx::query(RETENTION_DELETE_EVENTS_SQL)
            .bind(event_cutoff)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?
            .rows_affected();
        tx.commit().await.map_err(database_error)?;
        Ok(RetentionCleanupOutcome {
            deleted_events,
            deleted_inactive_tunnels,
        })
    }
}

impl RetentionCleanupPolicy {
    pub fn new(
        event_retention_days: i64,
        inactive_tunnel_retention_days: i64,
        now: DateTime<Utc>,
    ) -> Result<Self, TunnelStoreError> {
        let policy = Self {
            event_retention_days,
            inactive_tunnel_retention_days,
            now,
        };
        policy.event_cutoff()?;
        policy.inactive_tunnel_cutoff()?;
        Ok(policy)
    }

    pub(crate) fn event_cutoff(&self) -> Result<DateTime<Utc>, TunnelStoreError> {
        retention_cutoff(self.now, self.event_retention_days, "event_retention_days")
    }

    pub(crate) fn inactive_tunnel_cutoff(&self) -> Result<DateTime<Utc>, TunnelStoreError> {
        retention_cutoff(
            self.now,
            self.inactive_tunnel_retention_days,
            "inactive_tunnel_retention_days",
        )
    }
}

fn retention_cutoff(
    now: DateTime<Utc>,
    days: i64,
    field: &'static str,
) -> Result<DateTime<Utc>, TunnelStoreError> {
    if days <= 0 {
        return Err(TunnelStoreError::InvalidInput(format!(
            "{field} must be positive"
        )));
    }
    let duration = Duration::try_days(days).ok_or_else(|| {
        TunnelStoreError::InvalidInput(format!("{field} is too large to represent"))
    })?;
    now.checked_sub_signed(duration)
        .ok_or_else(|| TunnelStoreError::InvalidInput(format!("{field} is too large to apply")))
}
