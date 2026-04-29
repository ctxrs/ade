use std::net::SocketAddr;

use anyhow::{Context, Result};
use clap::Parser;
use ctx_llm_relay_authority::{
    relay_authority_router_with_bearer, relay_authority_router_with_config, GrantVerifier,
    InMemoryAuthorityStore, PostgresAuthorityStore, RelayAuthorityConfig,
};
use sqlx::postgres::PgPoolOptions;
use tracing::info;

#[derive(Parser, Debug)]
#[command(name = "ctx-llm-relay-authority", version)]
struct Args {
    #[arg(long, default_value = "127.0.0.1:8792")]
    listen: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args = Args::parse();
    let addr: SocketAddr = args.listen.parse().context("parsing --listen")?;
    let database_url = std::env::var("RELAY_AUTHORITY_DATABASE_URL").ok();
    let bearer_token = std::env::var("RELAY_AUTHORITY_BEARER_TOKEN")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let use_dev_memory = std::env::var("CTX_RELAY_AUTHORITY_DEV_IN_MEMORY")
        .ok()
        .is_some_and(|value| value == "1" || value.eq_ignore_ascii_case("true"));
    if database_url.is_none() && !use_dev_memory {
        anyhow::bail!(
            "missing RELAY_AUTHORITY_DATABASE_URL; set CTX_RELAY_AUTHORITY_DEV_IN_MEMORY=1 only for local fake-provider development"
        );
    }
    if database_url.is_some() && bearer_token.is_none() {
        anyhow::bail!("missing RELAY_AUTHORITY_BEARER_TOKEN for Postgres-backed relay authority");
    }
    let control_plane_jwks = std::env::var("RELAY_AUTHORITY_CONTROL_PLANE_JWKS")
        .ok()
        .filter(|value| !value.trim().is_empty());
    if database_url.is_some() && control_plane_jwks.is_none() {
        anyhow::bail!(
            "missing RELAY_AUTHORITY_CONTROL_PLANE_JWKS for Postgres-backed relay authority"
        );
    }
    info!("relay authority listening on http://{addr}");
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .context("binding listener")?;
    if let Some(database_url) = database_url {
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect(&database_url)
            .await
            .context("connecting to relay authority database")?;
        let verifier = GrantVerifier::from_control_plane_jwks_json(
            control_plane_jwks
                .as_deref()
                .context("missing control-plane JWKS")?,
        )
        .context("parsing control-plane JWKS")?;
        axum::serve(
            listener,
            relay_authority_router_with_config(
                PostgresAuthorityStore::new(pool),
                RelayAuthorityConfig {
                    bearer_token,
                    grant_verifier: Some(verifier),
                    ..RelayAuthorityConfig::default()
                },
            ),
        )
        .await
        .context("serving relay authority")?;
    } else {
        axum::serve(
            listener,
            relay_authority_router_with_bearer(InMemoryAuthorityStore::default(), bearer_token),
        )
        .await
        .context("serving relay authority")?;
    }
    Ok(())
}
