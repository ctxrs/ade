use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPoolOptions;
use sqlx::{Pool, Postgres, Row};
use thiserror::Error;
use url::Url;

const HEALTHY_RELAY_WINDOW_SECS: i64 = 60;
const REGISTER_RELAY_SQL: &str = r#"
            insert into public.mobile_tunnel_relay_node
              (relay_id, region, public_base_url, internal_base_url, status, max_active_tunnels)
            values ($1, $2, $3, $4, 'active', $5)
            on conflict (relay_id) do update
              set region = excluded.region,
                  public_base_url = excluded.public_base_url,
                  internal_base_url = excluded.internal_base_url,
                  status = case
                    when public.mobile_tunnel_relay_node.status in ('disabled', 'draining')
                      then public.mobile_tunnel_relay_node.status
                    else 'active'
                  end,
                  max_active_tunnels = excluded.max_active_tunnels
            "#;

#[derive(Debug, Error)]
pub enum TunnelStoreError {
    #[error("MOBILE_TUNNEL_DATABASE_URL must be a postgres:// or postgresql:// URL")]
    InvalidDatabaseUrl,
    #[error("database error: {0}")]
    Database(String),
    #[error("invalid URL for {field}: {value}")]
    InvalidUrl { field: &'static str, value: String },
    #[error("no healthy relay is available in region {region}")]
    NoHealthyRelay { region: String },
    #[error("invalid tunnel store input: {0}")]
    InvalidInput(String),
}

#[derive(Clone, Debug)]
pub struct TunnelStore {
    pool: Pool<Postgres>,
}

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

impl TunnelStore {
    pub async fn connect(database_url: &str) -> Result<Self, TunnelStoreError> {
        validate_postgres_database_url(database_url)?;
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect(database_url)
            .await
            .map_err(database_error)?;
        Ok(Self { pool })
    }

    pub fn from_pool(pool: Pool<Postgres>) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &Pool<Postgres> {
        &self.pool
    }

    pub async fn health_snapshot(&self) -> Result<HealthSnapshot, TunnelStoreError> {
        let row = sqlx::query(
            r#"
            select
              count(*)::bigint as relay_count,
              count(*) filter (
                where status = 'active'
                  and heartbeat_expires_at > now()
                  and active_tunnel_count < max_active_tunnels
              )::bigint as healthy_relay_count
            from public.mobile_tunnel_relay_node
            "#,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(database_error)?;
        Ok(HealthSnapshot {
            relay_count: row.try_get("relay_count").map_err(database_error)?,
            healthy_relay_count: row.try_get("healthy_relay_count").map_err(database_error)?,
        })
    }

    pub async fn register_relay(
        &self,
        registration: RelayRegistration,
    ) -> Result<(), TunnelStoreError> {
        registration.validate()?;
        sqlx::query(REGISTER_RELAY_SQL)
            .bind(&registration.relay_id)
            .bind(&registration.region)
            .bind(&registration.public_base_url)
            .bind(&registration.internal_base_url)
            .bind(registration.max_active_tunnels)
            .execute(&self.pool)
            .await
            .map_err(database_error)?;
        self.insert_event(
            None,
            Some(&registration.relay_id),
            None,
            "relay_registered",
            serde_json::json!({
                "region": registration.region,
                "public_base_url": registration.public_base_url,
                "internal_base_url": registration.internal_base_url,
                "max_active_tunnels": registration.max_active_tunnels,
            }),
        )
        .await?;
        Ok(())
    }

    pub async fn heartbeat_relay(&self, heartbeat: RelayHeartbeat) -> Result<(), TunnelStoreError> {
        if heartbeat.relay_id.trim().is_empty() {
            return Err(TunnelStoreError::InvalidInput(
                "relay_id must not be empty".to_string(),
            ));
        }
        if heartbeat.active_tunnel_count < 0 {
            return Err(TunnelStoreError::InvalidInput(
                "active_tunnel_count must be non-negative".to_string(),
            ));
        }
        let expires_at = heartbeat.observed_at + Duration::seconds(HEALTHY_RELAY_WINDOW_SECS);
        let result = sqlx::query(
            r#"
            update public.mobile_tunnel_relay_node
               set active_tunnel_count = $2,
                   last_heartbeat_at = $3,
                   heartbeat_expires_at = $4
             where relay_id = $1
            "#,
        )
        .bind(&heartbeat.relay_id)
        .bind(heartbeat.active_tunnel_count)
        .bind(heartbeat.observed_at)
        .bind(expires_at)
        .execute(&self.pool)
        .await
        .map_err(database_error)?;
        if result.rows_affected() != 1 {
            return Err(TunnelStoreError::InvalidInput(format!(
                "relay {} is not registered",
                heartbeat.relay_id
            )));
        }
        Ok(())
    }

    pub async fn assign_relay(&self, region: &str) -> Result<RelayRecord, TunnelStoreError> {
        if region.trim().is_empty() {
            return Err(TunnelStoreError::InvalidInput(
                "region must not be empty".to_string(),
            ));
        }
        let Some(row) = sqlx::query(
            r#"
            select relay_id, region, public_base_url, internal_base_url,
                   active_tunnel_count, max_active_tunnels,
                   last_heartbeat_at, heartbeat_expires_at
              from public.mobile_tunnel_relay_node
             where region = $1
               and status = 'active'
               and heartbeat_expires_at > now()
               and active_tunnel_count < max_active_tunnels
             order by active_tunnel_count asc, last_heartbeat_at desc, relay_id asc
             limit 1
            "#,
        )
        .bind(region)
        .fetch_optional(&self.pool)
        .await
        .map_err(database_error)?
        else {
            return Err(TunnelStoreError::NoHealthyRelay {
                region: region.to_string(),
            });
        };
        relay_from_row(&row)
    }

    pub async fn load_active_tunnel_for_user(
        &self,
        user_id: &str,
    ) -> Result<Option<TunnelAssignment>, TunnelStoreError> {
        let Some(row) = sqlx::query(
            r#"
            select t.tunnel_id, t.user_id, t.billing_subject_id, t.relay_id,
                   r.public_base_url as relay_public_base_url,
                   r.internal_base_url as relay_internal_base_url,
                   t.public_base_url, t.created_at
              from public.mobile_tunnel t
              join public.mobile_tunnel_relay_node r on r.relay_id = t.relay_id
             where t.user_id = $1
               and t.status = 'active'
               and t.disabled_at is null
             order by t.created_at desc
             limit 1
            "#,
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(database_error)?
        else {
            return Ok(None);
        };
        Ok(Some(tunnel_assignment_from_row(&row)?))
    }

    pub async fn create_tunnel(
        &self,
        request: CreateTunnelRequest,
    ) -> Result<TunnelAssignment, TunnelStoreError> {
        request.validate()?;
        let insert = sqlx::query(
            r#"
            insert into public.mobile_tunnel
              (tunnel_id, user_id, billing_subject_id, relay_id, public_base_url, status)
            values ($1, $2, $3, $4, $5, 'active')
            "#,
        )
        .bind(&request.tunnel_id)
        .bind(&request.user_id)
        .bind(&request.billing_subject_id)
        .bind(&request.relay_id)
        .bind(&request.public_base_url)
        .execute(&self.pool)
        .await;

        match insert {
            Ok(_) => {
                self.insert_event(
                    Some(&request.tunnel_id),
                    Some(&request.relay_id),
                    Some(&request.user_id),
                    "tunnel_created",
                    serde_json::json!({
                        "public_base_url": request.public_base_url,
                        "billing_subject_id": request.billing_subject_id,
                    }),
                )
                .await?;
            }
            Err(err) if is_unique_violation(&err) => {
                if let Some(existing) = self.load_active_tunnel_for_user(&request.user_id).await? {
                    return Ok(existing);
                }
                return Err(database_error(err));
            }
            Err(err) => return Err(database_error(err)),
        }

        self.load_active_tunnel_for_user(&request.user_id)
            .await?
            .ok_or_else(|| TunnelStoreError::Database("created tunnel was not readable".into()))
    }

    pub async fn revoke_active_tunnels_for_user(
        &self,
        user_id: &str,
    ) -> Result<u64, TunnelStoreError> {
        if user_id.trim().is_empty() {
            return Err(TunnelStoreError::InvalidInput(
                "user_id must not be empty".to_string(),
            ));
        }
        let result = sqlx::query(
            r#"
            update public.mobile_tunnel
               set status = 'revoked',
                   disabled_at = now()
             where user_id = $1
               and status = 'active'
               and disabled_at is null
            "#,
        )
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(database_error)?;
        if result.rows_affected() > 0 {
            self.insert_event(
                None,
                None,
                Some(user_id),
                "tunnels_revoked",
                serde_json::json!({
                    "count": result.rows_affected(),
                }),
            )
            .await?;
        }
        Ok(result.rows_affected())
    }

    pub async fn resolve_tunnel_target(
        &self,
        tunnel_id: &str,
    ) -> Result<TunnelResolveResult, TunnelStoreError> {
        let Some(row) = sqlx::query(
            r#"
            select t.tunnel_id, t.status as tunnel_status, t.disabled_at,
                   t.public_base_url, t.relay_id,
                   r.public_base_url as relay_public_base_url,
                   r.internal_base_url as relay_internal_base_url,
                   r.status as relay_status,
                   r.heartbeat_expires_at
              from public.mobile_tunnel t
              join public.mobile_tunnel_relay_node r on r.relay_id = t.relay_id
             where t.tunnel_id = $1
             limit 1
            "#,
        )
        .bind(tunnel_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(database_error)?
        else {
            return Ok(TunnelResolveResult::UnknownTunnel);
        };

        let tunnel_status: String = row.try_get("tunnel_status").map_err(database_error)?;
        let disabled_at: Option<DateTime<Utc>> =
            row.try_get("disabled_at").map_err(database_error)?;
        if tunnel_status != "active" || disabled_at.is_some() {
            return Ok(TunnelResolveResult::TunnelDisabled);
        }

        let relay_status: String = row.try_get("relay_status").map_err(database_error)?;
        let heartbeat_expires_at: Option<DateTime<Utc>> = row
            .try_get("heartbeat_expires_at")
            .map_err(database_error)?;
        if relay_status != "active" || heartbeat_expires_at.is_none_or(|ts| ts <= Utc::now()) {
            return Ok(TunnelResolveResult::RelayUnavailable);
        }

        Ok(TunnelResolveResult::Resolved(ResolvedTunnelTarget {
            tunnel_id: row.try_get("tunnel_id").map_err(database_error)?,
            relay_id: row.try_get("relay_id").map_err(database_error)?,
            relay_public_base_url: row
                .try_get("relay_public_base_url")
                .map_err(database_error)?,
            relay_internal_base_url: row
                .try_get("relay_internal_base_url")
                .map_err(database_error)?,
            public_base_url: row.try_get("public_base_url").map_err(database_error)?,
        }))
    }

    pub async fn validate_tunnel_relay_assignment(
        &self,
        tunnel_id: &str,
        relay_id: &str,
    ) -> Result<RelayAssignmentValidation, TunnelStoreError> {
        let Some(row) = sqlx::query(
            r#"
            select relay_id, status, disabled_at
              from public.mobile_tunnel
             where tunnel_id = $1
             limit 1
            "#,
        )
        .bind(tunnel_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(database_error)?
        else {
            return Ok(RelayAssignmentValidation::UnknownTunnel);
        };
        let status: String = row.try_get("status").map_err(database_error)?;
        let disabled_at: Option<DateTime<Utc>> =
            row.try_get("disabled_at").map_err(database_error)?;
        if status != "active" || disabled_at.is_some() {
            return Ok(RelayAssignmentValidation::TunnelDisabled);
        }
        let expected_relay_id: String = row.try_get("relay_id").map_err(database_error)?;
        if expected_relay_id != relay_id {
            return Ok(RelayAssignmentValidation::WrongRelay { expected_relay_id });
        }
        Ok(RelayAssignmentValidation::Valid)
    }

    pub async fn record_desktop_connected(
        &self,
        tunnel_id: &str,
        relay_id: &str,
    ) -> Result<(), TunnelStoreError> {
        sqlx::query(
            r#"
            update public.mobile_tunnel
               set last_connected_at = now(),
                   last_accessed_at = now()
             where tunnel_id = $1
               and relay_id = $2
               and status = 'active'
               and disabled_at is null
            "#,
        )
        .bind(tunnel_id)
        .bind(relay_id)
        .execute(&self.pool)
        .await
        .map_err(database_error)?;
        self.insert_event(
            Some(tunnel_id),
            Some(relay_id),
            None,
            "desktop_connected",
            serde_json::json!({}),
        )
        .await
    }

    pub async fn record_desktop_disconnected(
        &self,
        tunnel_id: &str,
        relay_id: &str,
    ) -> Result<(), TunnelStoreError> {
        self.insert_event(
            Some(tunnel_id),
            Some(relay_id),
            None,
            "desktop_disconnected",
            serde_json::json!({}),
        )
        .await
    }

    pub async fn mark_tunnel_accessed(&self, tunnel_id: &str) -> Result<(), TunnelStoreError> {
        sqlx::query(
            "update public.mobile_tunnel set last_accessed_at = now() where tunnel_id = $1",
        )
        .bind(tunnel_id)
        .execute(&self.pool)
        .await
        .map_err(database_error)?;
        Ok(())
    }

    async fn insert_event(
        &self,
        tunnel_id: Option<&str>,
        relay_id: Option<&str>,
        user_id: Option<&str>,
        event_type: &str,
        metadata: serde_json::Value,
    ) -> Result<(), TunnelStoreError> {
        sqlx::query(
            r#"
            insert into public.mobile_tunnel_event
              (tunnel_id, relay_id, user_id, event_type, metadata)
            values ($1, $2, $3, $4, $5)
            "#,
        )
        .bind(tunnel_id)
        .bind(relay_id)
        .bind(user_id)
        .bind(event_type)
        .bind(metadata)
        .execute(&self.pool)
        .await
        .map_err(database_error)?;
        Ok(())
    }
}

impl RelayRegistration {
    fn validate(&self) -> Result<(), TunnelStoreError> {
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
    fn validate(&self) -> Result<(), TunnelStoreError> {
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

fn tunnel_assignment_from_row(
    row: &sqlx::postgres::PgRow,
) -> Result<TunnelAssignment, TunnelStoreError> {
    Ok(TunnelAssignment {
        tunnel_id: row.try_get("tunnel_id").map_err(database_error)?,
        user_id: row.try_get("user_id").map_err(database_error)?,
        billing_subject_id: row.try_get("billing_subject_id").map_err(database_error)?,
        relay_id: row.try_get("relay_id").map_err(database_error)?,
        relay_public_base_url: row
            .try_get("relay_public_base_url")
            .map_err(database_error)?,
        relay_internal_base_url: row
            .try_get("relay_internal_base_url")
            .map_err(database_error)?,
        public_base_url: row.try_get("public_base_url").map_err(database_error)?,
        created_at: row.try_get("created_at").map_err(database_error)?,
    })
}

fn relay_from_row(row: &sqlx::postgres::PgRow) -> Result<RelayRecord, TunnelStoreError> {
    Ok(RelayRecord {
        relay_id: row.try_get("relay_id").map_err(database_error)?,
        region: row.try_get("region").map_err(database_error)?,
        public_base_url: row.try_get("public_base_url").map_err(database_error)?,
        internal_base_url: row.try_get("internal_base_url").map_err(database_error)?,
        active_tunnel_count: row.try_get("active_tunnel_count").map_err(database_error)?,
        max_active_tunnels: row.try_get("max_active_tunnels").map_err(database_error)?,
        last_heartbeat_at: row.try_get("last_heartbeat_at").map_err(database_error)?,
        heartbeat_expires_at: row
            .try_get("heartbeat_expires_at")
            .map_err(database_error)?,
    })
}

fn validate_postgres_database_url(database_url: &str) -> Result<(), TunnelStoreError> {
    let parsed = Url::parse(database_url).map_err(|_| TunnelStoreError::InvalidDatabaseUrl)?;
    match parsed.scheme() {
        "postgres" | "postgresql" => Ok(()),
        _ => Err(TunnelStoreError::InvalidDatabaseUrl),
    }
}

fn validate_url(field: &'static str, value: &str) -> Result<(), TunnelStoreError> {
    ensure_non_empty(field, value)?;
    let parsed = Url::parse(value).map_err(|_| TunnelStoreError::InvalidUrl {
        field,
        value: value.to_string(),
    })?;
    match parsed.scheme() {
        "http" | "https" => Ok(()),
        _ => Err(TunnelStoreError::InvalidUrl {
            field,
            value: value.to_string(),
        }),
    }
}

fn ensure_non_empty(field: &str, value: &str) -> Result<(), TunnelStoreError> {
    if value.trim().is_empty() {
        return Err(TunnelStoreError::InvalidInput(format!(
            "{field} must not be empty"
        )));
    }
    Ok(())
}

fn database_error(err: impl std::fmt::Display) -> TunnelStoreError {
    TunnelStoreError::Database(err.to_string())
}

fn is_unique_violation(err: &sqlx::Error) -> bool {
    err.as_database_error()
        .and_then(|db| db.code())
        .is_some_and(|code| code.as_ref() == "23505")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_postgres_database_urls() {
        assert!(matches!(
            validate_postgres_database_url("sqlite:///tmp/control-plane.sqlite"),
            Err(TunnelStoreError::InvalidDatabaseUrl)
        ));
        assert!(validate_postgres_database_url("postgres://user:pw@example/db").is_ok());
        assert!(validate_postgres_database_url("postgresql://user:pw@example/db").is_ok());
    }

    #[test]
    fn validates_relay_registration_urls_and_capacity() {
        let valid = RelayRegistration {
            relay_id: "relay-1".to_string(),
            region: "us".to_string(),
            public_base_url: "https://relay-1.tunnel.ctx.rs".to_string(),
            internal_base_url: "http://127.0.0.1:8787".to_string(),
            max_active_tunnels: 100,
        };
        assert!(valid.validate().is_ok());

        let mut invalid = valid.clone();
        invalid.public_base_url = "not-url".to_string();
        assert!(matches!(
            invalid.validate(),
            Err(TunnelStoreError::InvalidUrl {
                field: "public_base_url",
                ..
            })
        ));

        let mut invalid = valid;
        invalid.max_active_tunnels = 0;
        assert!(matches!(
            invalid.validate(),
            Err(TunnelStoreError::InvalidInput(_))
        ));
    }

    #[test]
    fn register_relay_preserves_ops_disabled_and_draining_statuses() {
        assert!(REGISTER_RELAY_SQL
            .contains("public.mobile_tunnel_relay_node.status in ('disabled', 'draining')"));
        assert!(REGISTER_RELAY_SQL.contains("then public.mobile_tunnel_relay_node.status"));
    }

    #[test]
    fn validates_create_tunnel_request() {
        let valid = CreateTunnelRequest {
            tunnel_id: "tun_1".to_string(),
            user_id: "user_1".to_string(),
            billing_subject_id: None,
            relay_id: "relay-1".to_string(),
            public_base_url: "https://tunnel.ctx.rs/t/tun_1".to_string(),
        };
        assert!(valid.validate().is_ok());

        let mut invalid = valid;
        invalid.user_id = " ".to_string();
        assert!(matches!(
            invalid.validate(),
            Err(TunnelStoreError::InvalidInput(_))
        ));
    }
}
