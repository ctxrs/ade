use anyhow::{bail, Result};
use sqlx::{Pool, Sqlite};

const TOOL_ORDER_SEQ_LEGACY_MIGRATION_VERSION: i64 = 49;
const TOOL_ORDER_SEQ_MIGRATION_VERSION: i64 = 55;
const TOOL_ORDER_SEQ_MIGRATION_DESCRIPTION: &str = "tool order seq";

async fn migrations_table_exists(pool: &Pool<Sqlite>) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations')",
    )
    .fetch_one(pool)
    .await?
        != 0)
}

async fn applied_migration_version(pool: &Pool<Sqlite>, description: &str) -> Result<Option<i64>> {
    sqlx::query_scalar::<_, i64>(
        "SELECT version FROM _sqlx_migrations WHERE description = ? LIMIT 1",
    )
    .bind(description)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

async fn migration_description_for_version(
    pool: &Pool<Sqlite>,
    version: i64,
) -> Result<Option<String>> {
    sqlx::query_scalar::<_, String>(
        "SELECT description FROM _sqlx_migrations WHERE version = ? LIMIT 1",
    )
    .bind(version)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

pub(super) async fn repair_historical_tool_order_seq_migration_version(
    pool: &Pool<Sqlite>,
) -> Result<()> {
    if !migrations_table_exists(pool).await? {
        return Ok(());
    }

    if applied_migration_version(pool, TOOL_ORDER_SEQ_MIGRATION_DESCRIPTION).await?
        != Some(TOOL_ORDER_SEQ_LEGACY_MIGRATION_VERSION)
    {
        return Ok(());
    }

    match migration_description_for_version(pool, TOOL_ORDER_SEQ_MIGRATION_VERSION).await? {
        None => {
            sqlx::query(
                "UPDATE _sqlx_migrations SET version = ? WHERE version = ? AND description = ?",
            )
            .bind(TOOL_ORDER_SEQ_MIGRATION_VERSION)
            .bind(TOOL_ORDER_SEQ_LEGACY_MIGRATION_VERSION)
            .bind(TOOL_ORDER_SEQ_MIGRATION_DESCRIPTION)
            .execute(pool)
            .await?;
        }
        Some(existing_description)
            if existing_description == TOOL_ORDER_SEQ_MIGRATION_DESCRIPTION =>
        {
            sqlx::query("DELETE FROM _sqlx_migrations WHERE version = ? AND description = ?")
                .bind(TOOL_ORDER_SEQ_LEGACY_MIGRATION_VERSION)
                .bind(TOOL_ORDER_SEQ_MIGRATION_DESCRIPTION)
                .execute(pool)
                .await?;
        }
        Some(existing_description) => {
            bail!(
                "cannot remap migration '{TOOL_ORDER_SEQ_MIGRATION_DESCRIPTION}' from version {TOOL_ORDER_SEQ_LEGACY_MIGRATION_VERSION} to {TOOL_ORDER_SEQ_MIGRATION_VERSION}: version {TOOL_ORDER_SEQ_MIGRATION_VERSION} is already occupied by '{existing_description}'"
            );
        }
    }

    Ok(())
}
