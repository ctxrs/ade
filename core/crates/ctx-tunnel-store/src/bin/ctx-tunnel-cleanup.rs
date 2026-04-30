use std::env;
use std::error::Error;

use chrono::Utc;
use ctx_tunnel_store::{
    RetentionCleanupPolicy, TunnelStore, DEFAULT_INACTIVE_TUNNEL_RETENTION_DAYS,
    DEFAULT_TUNNEL_EVENT_RETENTION_DAYS,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let database_url = required_env("MOBILE_TUNNEL_DATABASE_URL")?;
    let policy = RetentionCleanupPolicy::new(
        retention_days(
            "CTX_TUNNEL_EVENT_RETENTION_DAYS",
            DEFAULT_TUNNEL_EVENT_RETENTION_DAYS,
        )?,
        retention_days(
            "CTX_TUNNEL_INACTIVE_TUNNEL_RETENTION_DAYS",
            DEFAULT_INACTIVE_TUNNEL_RETENTION_DAYS,
        )?,
        Utc::now(),
    )?;
    let store = TunnelStore::connect(&database_url).await?;
    let outcome = store.cleanup_retention(policy).await?;
    println!("{}", serde_json::to_string(&outcome)?);
    Ok(())
}

fn required_env(name: &str) -> Result<String, Box<dyn Error>> {
    let value = env::var(name)?;
    if value.trim().is_empty() {
        return Err(format!("{name} must not be empty").into());
    }
    Ok(value)
}

fn retention_days(name: &str, default_value: i64) -> Result<i64, Box<dyn Error>> {
    let Ok(raw) = env::var(name) else {
        return Ok(default_value);
    };
    let value = raw.trim().parse::<i64>()?;
    if value <= 0 {
        return Err(format!("{name} must be positive").into());
    }
    Ok(value)
}
