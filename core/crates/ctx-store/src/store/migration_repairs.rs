use anyhow::{bail, Result};
use sqlx::{Pool, Sqlite};

const SANDBOX_BINDINGS_LEGACY_MIGRATION_VERSION: i64 = 55;
const SANDBOX_BINDINGS_MIGRATION_VERSION: i64 = 56;
const SANDBOX_BINDINGS_MIGRATION_DESCRIPTION: &str = "sandbox bindings";
const SANDBOX_BINDING_EXEC_SETTINGS_LEGACY_MIGRATION_VERSION: i64 = 56;
const SANDBOX_BINDING_EXEC_SETTINGS_MIGRATION_VERSION: i64 = 57;
const SANDBOX_BINDING_EXEC_SETTINGS_MIGRATION_DESCRIPTION: &str =
    "sandbox binding execution settings";
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

async fn rename_historical_migration_version(
    pool: &Pool<Sqlite>,
    description: &str,
    legacy_version: i64,
    new_version: i64,
) -> Result<()> {
    if applied_migration_version(pool, description).await? != Some(legacy_version) {
        return Ok(());
    }

    match migration_description_for_version(pool, new_version).await? {
        None => {
            sqlx::query(
                "UPDATE _sqlx_migrations SET version = ? WHERE version = ? AND description = ?",
            )
            .bind(new_version)
            .bind(legacy_version)
            .bind(description)
            .execute(pool)
            .await?;
        }
        Some(existing_description) if existing_description == description => {
            sqlx::query("DELETE FROM _sqlx_migrations WHERE version = ? AND description = ?")
                .bind(legacy_version)
                .bind(description)
                .execute(pool)
                .await?;
        }
        Some(existing_description) => {
            bail!(
                "cannot remap migration '{description}' from version {legacy_version} to {new_version}: version {new_version} is already occupied by '{existing_description}'"
            );
        }
    }

    Ok(())
}

pub(super) async fn repair_rebased_sandbox_binding_migration_versions(
    pool: &Pool<Sqlite>,
) -> Result<()> {
    if !migrations_table_exists(pool).await? {
        return Ok(());
    }

    // Move the newer sandbox-binding migration first so version 56 is free before remapping the
    // original sandbox_bindings row from 55 -> 56.
    rename_historical_migration_version(
        pool,
        SANDBOX_BINDING_EXEC_SETTINGS_MIGRATION_DESCRIPTION,
        SANDBOX_BINDING_EXEC_SETTINGS_LEGACY_MIGRATION_VERSION,
        SANDBOX_BINDING_EXEC_SETTINGS_MIGRATION_VERSION,
    )
    .await?;
    rename_historical_migration_version(
        pool,
        SANDBOX_BINDINGS_MIGRATION_DESCRIPTION,
        SANDBOX_BINDINGS_LEGACY_MIGRATION_VERSION,
        SANDBOX_BINDINGS_MIGRATION_VERSION,
    )
    .await?;

    Ok(())
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
                "cannot remap migration '{}' from version {} to {}: version {} is already occupied by '{}'",
                TOOL_ORDER_SEQ_MIGRATION_DESCRIPTION,
                TOOL_ORDER_SEQ_LEGACY_MIGRATION_VERSION,
                TOOL_ORDER_SEQ_MIGRATION_VERSION,
                TOOL_ORDER_SEQ_MIGRATION_VERSION,
                existing_description
            );
        }
    }

    Ok(())
}
