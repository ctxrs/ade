use super::*;
pub(super) use ctx_desktop_ipc::{
    DesktopStorageBatchOp, DesktopStorageBatchReq, DesktopStorageGetReq, DesktopStorageNotice,
    DesktopUiStateResetReason,
};

#[derive(Default)]
pub(super) struct DesktopStorage {
    pool: OnceCell<SqlitePool>,
}

const UI_KV_CREATE_SQL: &str =
    "CREATE TABLE IF NOT EXISTS ui_kv (key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at_ms INTEGER NOT NULL)";
const UI_META_CREATE_SQL: &str =
    "CREATE TABLE IF NOT EXISTS ui_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at_ms INTEGER NOT NULL)";
const UI_STATE_RESET_NOTICE_KEY: &str = "ui_state_reset_notice";

impl DesktopStorage {
    pub(super) async fn pool(&self, app: &tauri::AppHandle) -> Result<&SqlitePool> {
        self.pool
            .get_or_try_init(|| async {
                let path = desktop_storage_path(app)?;
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).ok();
                }
                let options = SqliteConnectOptions::new()
                    .filename(&path)
                    .create_if_missing(true)
                    .journal_mode(SqliteJournalMode::Wal)
                    .synchronous(SqliteSynchronous::Normal)
                    .busy_timeout(Duration::from_secs(5));
                let pool = SqlitePoolOptions::new()
                    .max_connections(1)
                    .connect_with(options)
                    .await
                    .context("opening desktop storage sqlite db")?;
                ensure_ui_kv_schema(&pool).await?;
                Ok(pool)
            })
            .await
    }
}

fn now_ms_i64() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

async fn ensure_ui_meta_schema(pool: &SqlitePool) -> Result<()> {
    sqlx::query(UI_META_CREATE_SQL)
        .execute(pool)
        .await
        .context("creating ui_meta table")?;
    Ok(())
}

async fn read_ui_kv_schema(pool: &SqlitePool) -> Result<Vec<(String, i64, i64)>> {
    let rows = sqlx::query("PRAGMA table_info(ui_kv)")
        .fetch_all(pool)
        .await
        .context("reading ui_kv schema")?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let name: String = row.try_get("name").context("reading ui_kv column name")?;
        let notnull: i64 = row.try_get("notnull").context("reading ui_kv notnull")?;
        let pk: i64 = row.try_get("pk").context("reading ui_kv pk")?;
        out.push((name, notnull, pk));
    }
    Ok(out)
}

fn is_current_ui_kv_schema(columns: &[(String, i64, i64)]) -> bool {
    if columns.len() != 3 {
        return false;
    }
    let (name0, _notnull0, pk0) = &columns[0];
    let (name1, notnull1, pk1) = &columns[1];
    let (name2, notnull2, pk2) = &columns[2];
    name0 == "key"
        && *pk0 == 1
        && name1 == "value"
        && *notnull1 == 1
        && *pk1 == 0
        && name2 == "updated_at_ms"
        && *notnull2 == 1
        && *pk2 == 0
}

async fn record_ui_state_reset_notice(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    reason: DesktopUiStateResetReason,
) -> Result<()> {
    let value = serde_json::to_string(&DesktopStorageNotice::UiStateReset { reason })
        .context("serializing ui state reset notice")?;
    sqlx::query(
        "INSERT INTO ui_meta (key, value, updated_at_ms) VALUES (?1, ?2, ?3) \
         ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at_ms=excluded.updated_at_ms",
    )
    .bind(UI_STATE_RESET_NOTICE_KEY)
    .bind(value)
    .bind(now_ms_i64())
    .execute(&mut **tx)
    .await
    .context("writing ui state reset notice")?;
    Ok(())
}

async fn reset_ui_kv_state(pool: &SqlitePool, reason: DesktopUiStateResetReason) -> Result<()> {
    let mut tx = pool
        .begin()
        .await
        .context("starting ui state reset transaction")?;
    sqlx::query("DROP TABLE IF EXISTS ui_kv")
        .execute(&mut *tx)
        .await
        .context("dropping ui_kv table during reset")?;
    sqlx::query(UI_KV_CREATE_SQL)
        .execute(&mut *tx)
        .await
        .context("recreating ui_kv table during reset")?;
    sqlx::query(UI_META_CREATE_SQL)
        .execute(&mut *tx)
        .await
        .context("ensuring ui_meta table during reset")?;
    record_ui_state_reset_notice(&mut tx, reason).await?;
    tx.commit()
        .await
        .context("committing ui state reset transaction")?;
    eprintln!(
        "desktop_ui_state_db_reset reason={}",
        desktop_ui_state_reset_reason_str(reason)
    );
    Ok(())
}

fn desktop_ui_state_reset_reason_str(reason: DesktopUiStateResetReason) -> &'static str {
    match reason {
        DesktopUiStateResetReason::SchemaMismatch => "schema_mismatch",
        DesktopUiStateResetReason::InvalidUiStateDb => "invalid_ui_state_db",
    }
}

async fn ensure_ui_kv_schema(pool: &SqlitePool) -> Result<()> {
    ensure_ui_meta_schema(pool).await?;
    sqlx::query(UI_KV_CREATE_SQL)
        .execute(pool)
        .await
        .context("creating ui_kv table")?;

    let schema = match read_ui_kv_schema(pool).await {
        Ok(schema) => schema,
        Err(err) => {
            eprintln!(
                "desktop_ui_state_db_schema_read_failed reason=invalid_ui_state_db error={err:#}"
            );
            reset_ui_kv_state(pool, DesktopUiStateResetReason::InvalidUiStateDb).await?;
            return Ok(());
        }
    };
    if !is_current_ui_kv_schema(&schema) {
        reset_ui_kv_state(pool, DesktopUiStateResetReason::SchemaMismatch).await?;
    }

    Ok(())
}

async fn consume_desktop_storage_notice(pool: &SqlitePool) -> Result<Option<DesktopStorageNotice>> {
    let mut tx = pool
        .begin()
        .await
        .context("starting desktop storage notice transaction")?;
    let row: Option<(String,)> = sqlx::query_as("SELECT value FROM ui_meta WHERE key = ?1")
        .bind(UI_STATE_RESET_NOTICE_KEY)
        .fetch_optional(&mut *tx)
        .await
        .context("reading desktop storage notice")?;
    if row.is_none() {
        tx.commit()
            .await
            .context("committing desktop storage notice transaction")?;
        return Ok(None);
    }

    sqlx::query("DELETE FROM ui_meta WHERE key = ?1")
        .bind(UI_STATE_RESET_NOTICE_KEY)
        .execute(&mut *tx)
        .await
        .context("deleting desktop storage notice")?;
    tx.commit()
        .await
        .context("committing desktop storage notice transaction")?;

    let Some((raw_value,)) = row else {
        return Ok(None);
    };
    match serde_json::from_str::<DesktopStorageNotice>(&raw_value) {
        Ok(notice) => Ok(Some(notice)),
        Err(err) => {
            eprintln!("dropping invalid desktop storage notice payload: {err}");
            Ok(None)
        }
    }
}

async fn desktop_storage_get_from_pool(
    pool: &SqlitePool,
    key: &str,
) -> Result<Option<serde_json::Value>> {
    let row: Option<(String,)> = sqlx::query_as("SELECT value FROM ui_kv WHERE key = ?1")
        .bind(key)
        .fetch_optional(pool)
        .await
        .context("reading ui_kv value")?;
    if let Some((value,)) = row {
        match serde_json::from_str(&value) {
            Ok(parsed) => Ok(Some(parsed)),
            Err(err) => {
                let _ = sqlx::query("DELETE FROM ui_kv WHERE key = ?1")
                    .bind(key)
                    .execute(pool)
                    .await;
                eprintln!("dropping corrupt ui_kv value for key {key}: {err}");
                Ok(None)
            }
        }
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod desktop_storage_tests {
    use super::*;

    async fn open_memory_pool() -> SqlitePool {
        SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap()
    }

    #[tokio::test(flavor = "current_thread")]
    async fn ensure_ui_kv_schema_initializes_current_schema() {
        let pool = open_memory_pool().await;
        ensure_ui_kv_schema(&pool).await.unwrap();
        let schema = read_ui_kv_schema(&pool).await.unwrap();
        assert!(is_current_ui_kv_schema(&schema));
        let notice = consume_desktop_storage_notice(&pool).await.unwrap();
        assert!(notice.is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn schema_mismatch_resets_ui_state_and_notice_is_one_time() {
        let pool = open_memory_pool().await;
        sqlx::query("CREATE TABLE ui_kv (key TEXT PRIMARY KEY, value TEXT NOT NULL)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO ui_kv (key, value) VALUES (?1, ?2)")
            .bind("legacy")
            .bind("\"payload\"")
            .execute(&pool)
            .await
            .unwrap();

        ensure_ui_kv_schema(&pool).await.unwrap();

        let row_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ui_kv")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            row_count, 0,
            "ui_kv contents should be reset on schema mismatch"
        );
        let schema = read_ui_kv_schema(&pool).await.unwrap();
        assert!(is_current_ui_kv_schema(&schema));

        let notice = consume_desktop_storage_notice(&pool).await.unwrap();
        assert_eq!(
            notice,
            Some(DesktopStorageNotice::UiStateReset {
                reason: DesktopUiStateResetReason::SchemaMismatch,
            })
        );
        let second = consume_desktop_storage_notice(&pool).await.unwrap();
        assert!(
            second.is_none(),
            "reset notice must be consumed after first read"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn corrupt_key_self_heal_is_log_only_and_does_not_emit_notice() {
        let pool = open_memory_pool().await;
        ensure_ui_kv_schema(&pool).await.unwrap();
        sqlx::query("INSERT INTO ui_kv (key, value, updated_at_ms) VALUES (?1, ?2, ?3)")
            .bind("bad")
            .bind("{bad-json")
            .bind(now_ms_i64())
            .execute(&pool)
            .await
            .unwrap();

        let got = desktop_storage_get_from_pool(&pool, "bad").await.unwrap();
        assert!(got.is_none());
        let still_exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ui_kv WHERE key = ?1")
            .bind("bad")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(still_exists, 0);

        let notice = consume_desktop_storage_notice(&pool).await.unwrap();
        assert!(notice.is_none(), "corrupt-key self-heal must stay log-only");
    }
}

#[tauri::command]
pub(super) async fn desktop_storage_get(
    app: tauri::AppHandle,
    storage: tauri::State<'_, DesktopStorage>,
    req: DesktopStorageGetReq,
) -> Result<Option<serde_json::Value>, String> {
    let pool = storage.pool(&app).await.map_err(to_err)?;
    desktop_storage_get_from_pool(pool, &req.key)
        .await
        .map_err(to_err)
}

#[tauri::command]
pub(super) async fn desktop_storage_batch(
    app: tauri::AppHandle,
    storage: tauri::State<'_, DesktopStorage>,
    req: DesktopStorageBatchReq,
) -> Result<(), String> {
    let ops = req.ops;
    if ops.is_empty() {
        return Ok(());
    }
    let pool = storage.pool(&app).await.map_err(to_err)?;
    let mut tx = pool.begin().await.map_err(to_err)?;
    let now = now_ms_i64();
    for op in ops {
        match op {
            DesktopStorageBatchOp::Set { key, value } => {
                let value_json = serde_json::to_string(&value).map_err(to_err)?;
                sqlx::query(
                    "INSERT INTO ui_kv (key, value, updated_at_ms) VALUES (?1, ?2, ?3) \
                     ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at_ms=excluded.updated_at_ms",
                )
                .bind(&key)
                .bind(&value_json)
                .bind(now)
                .execute(&mut *tx)
                .await
                .map_err(to_err)?;
            }
            DesktopStorageBatchOp::Delete { key } => {
                sqlx::query("DELETE FROM ui_kv WHERE key = ?1")
                    .bind(&key)
                    .execute(&mut *tx)
                    .await
                    .map_err(to_err)?;
            }
        }
    }
    tx.commit().await.map_err(to_err)?;
    Ok(())
}

#[tauri::command]
pub(super) async fn desktop_storage_consume_notice(
    app: tauri::AppHandle,
    storage: tauri::State<'_, DesktopStorage>,
) -> Result<Option<DesktopStorageNotice>, String> {
    let pool = storage.pool(&app).await.map_err(to_err)?;
    consume_desktop_storage_notice(pool).await.map_err(to_err)
}

fn desktop_storage_path(_app: &tauri::AppHandle) -> Result<PathBuf> {
    let root = desktop_local_data_root()?;
    Ok(ctx_fs::paths::ui_root(root).join("desktop-ui-state.sqlite"))
}
