use anyhow::Result;
use sqlx::{Pool, Sqlite};

const TOOL_ORDER_SEQ_LEGACY_MIGRATION_VERSION: i64 = 49;
const TOOL_ORDER_SEQ_MIGRATION_VERSION: i64 = 55;
const TOOL_ORDER_SEQ_MIGRATION_DESCRIPTION: &str = "tool order seq";

pub(super) async fn repair_historical_tool_order_seq_migration_version(
    pool: &Pool<Sqlite>,
) -> Result<()> {
    let migrations_table_exists = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations')",
    )
    .fetch_one(pool)
    .await?;
    if migrations_table_exists == 0 {
        return Ok(());
    }

    let tool_order_version = sqlx::query_scalar::<_, i64>(
        "SELECT version FROM _sqlx_migrations WHERE description = ? LIMIT 1",
    )
    .bind(TOOL_ORDER_SEQ_MIGRATION_DESCRIPTION)
    .fetch_optional(pool)
    .await?;

    if tool_order_version != Some(TOOL_ORDER_SEQ_LEGACY_MIGRATION_VERSION) {
        return Ok(());
    }

    let renamed_version_exists = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM _sqlx_migrations WHERE version = ?)",
    )
    .bind(TOOL_ORDER_SEQ_MIGRATION_VERSION)
    .fetch_one(pool)
    .await?;

    if renamed_version_exists == 0 {
        sqlx::query(
            "UPDATE _sqlx_migrations SET version = ? WHERE version = ? AND description = ?",
        )
        .bind(TOOL_ORDER_SEQ_MIGRATION_VERSION)
        .bind(TOOL_ORDER_SEQ_LEGACY_MIGRATION_VERSION)
        .bind(TOOL_ORDER_SEQ_MIGRATION_DESCRIPTION)
        .execute(pool)
        .await?;
    } else {
        sqlx::query("DELETE FROM _sqlx_migrations WHERE version = ? AND description = ?")
            .bind(TOOL_ORDER_SEQ_LEGACY_MIGRATION_VERSION)
            .bind(TOOL_ORDER_SEQ_MIGRATION_DESCRIPTION)
            .execute(pool)
            .await?;
    }

    Ok(())
}
