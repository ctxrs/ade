use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use ctx_core::ids::{MessageId, RunId, TurnId};
use ctx_core::models::{Message, MessageDelivery, MessageRole, SessionEventType, VcsKind};
use ctx_store::Store;
use sqlx::migrate::Migrator;
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::Row;
use tokio::process::Command;
use tokio::time::sleep;

const LOCK_CHILD_ENV: &str = "CTX_STORE_LOCK_CHILD";
const LOCK_DB_PATH_ENV: &str = "CTX_STORE_LOCK_DB_PATH";
const LOCK_READY_PATH_ENV: &str = "CTX_STORE_LOCK_READY_PATH";
const LOCK_RELEASE_PATH_ENV: &str = "CTX_STORE_LOCK_RELEASE_PATH";

#[test]
fn migration_versions_are_unique() -> Result<()> {
    let migrations = migration_files()?;
    let mut seen = std::collections::BTreeMap::new();

    for migration in migrations {
        let version = migration_version(&migration)?;
        if let Some(previous) = seen.insert(version, migration.clone()) {
            anyhow::bail!(
                "duplicate migration version {version}: {} and {}",
                previous.display(),
                migration.display()
            );
        }
    }

    Ok(())
}

#[tokio::test]
async fn migrations_upgrade_cleanly_from_every_historical_prefix() -> Result<()> {
    let migrations = migration_files()?;
    assert!(
        migrations.len() > 3,
        "expected enough historical migrations to exercise upgrade prefixes"
    );

    for prefix_len in 1..migrations.len() {
        let tempdir = tempfile::tempdir().context("creating tempdir for migration prefix")?;
        let subset_dir = tempdir.path().join("subset-migrations");
        fs::create_dir_all(&subset_dir).context("creating subset migration dir")?;
        for migration in &migrations[..prefix_len] {
            let filename = migration
                .file_name()
                .context("migration file missing name")?;
            fs::copy(migration, subset_dir.join(filename)).with_context(|| {
                format!(
                    "copying {} into subset prefix {}",
                    migration.display(),
                    prefix_len
                )
            })?;
        }

        let db_path = tempdir.path().join("db.sqlite");
        fs::File::create(&db_path).context("creating sqlite file")?;
        let sqlite_url = sqlite_url(&db_path);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&sqlite_url)
            .await
            .with_context(|| {
                format!("connecting partial-migration pool for prefix {prefix_len}")
            })?;
        let subset_migrator = Migrator::new(subset_dir.clone())
            .await
            .with_context(|| format!("loading subset migrator for prefix {prefix_len}"))?;
        subset_migrator
            .run(&pool)
            .await
            .with_context(|| format!("running subset migrator for prefix {prefix_len}"))?;
        pool.close().await;

        let store = Store::open(&db_path)
            .await
            .with_context(|| format!("opening store from prefix {prefix_len}"))?;
        store
            .create_workspace(
                format!("upgrade-{prefix_len}"),
                format!("/tmp/upgrade-{prefix_len}"),
                VcsKind::Git,
            )
            .await
            .with_context(|| format!("creating workspace after upgrading prefix {prefix_len}"))?;
        store.close().await;

        assert_store_integrity(&db_path).await?;
        assert_eq!(
            applied_migration_count(&db_path).await?,
            migrations.len() as i64,
            "expected all migrations applied after upgrading prefix {prefix_len}"
        );
    }

    Ok(())
}

#[tokio::test]
async fn open_repairs_partially_applied_duplicate_tool_display_migration() -> Result<()> {
    let tempdir = tempfile::tempdir().context("creating tempdir for duplicate migration repair")?;
    let subset_dir = tempdir.path().join("subset-migrations");
    fs::create_dir_all(&subset_dir).context("creating subset migration dir")?;

    let migrations = migration_files()?;
    let tool_display_migration = migrations
        .iter()
        .find(|path| path.file_name().is_some_and(|name| name == "0047_tool_display_fields.sql"))
        .cloned()
        .context("finding tool display migration")?;

    for migration in &migrations {
        if migration_version(migration)? <= 45 {
            let filename = migration
                .file_name()
                .context("migration file missing name")?;
            fs::copy(migration, subset_dir.join(filename)).with_context(|| {
                format!(
                    "copying {} into duplicate-version subset",
                    migration.display()
                )
            })?;
        }
    }

    fs::copy(
        &tool_display_migration,
        subset_dir.join("0046_tool_display_fields.sql"),
    )
    .with_context(|| {
        format!(
            "copying {} into duplicate-version subset as 0046",
            tool_display_migration.display()
        )
    })?;

    let db_path = tempdir.path().join("db.sqlite");
    fs::File::create(&db_path).context("creating sqlite file")?;
    let sqlite_url = sqlite_url(&db_path);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url)
        .await
        .context("connecting partial-migration pool for duplicate repair")?;
    let subset_migrator = Migrator::new(subset_dir.clone())
        .await
        .context("loading subset migrator for duplicate repair")?;
    subset_migrator
        .run(&pool)
        .await
        .context("running subset migrator for duplicate repair")?;
    pool.close().await;

    let store = Store::open(&db_path)
        .await
        .context("opening store after duplicate-version partial migration")?;
    store.close().await;

    assert_store_integrity(&db_path).await?;
    assert!(column_exists(&db_path, "sessions", "reasoning_effort").await?);
    assert!(column_exists(&db_path, "session_turn_tools", "provider_tool_name").await?);
    assert!(column_exists(&db_path, "session_turn_tools", "subtitle").await?);

    let applied = applied_migrations(&db_path).await?;
    assert!(
        applied
            .iter()
            .any(|(version, description)| *version == 46 && description == "session reasoning effort")
    );
    assert!(
        applied
            .iter()
            .any(|(version, description)| *version == 47 && description == "tool display fields")
    );

    Ok(())
}

#[tokio::test]
async fn sqlite_pragmas_and_integrity_hold_after_reopen() -> Result<()> {
    let tempdir = tempfile::tempdir().context("creating tempdir")?;
    let db_path = tempdir.path().join("db.sqlite");
    let store = Store::open(&db_path).await.context("opening store")?;

    let workspace = store
        .create_workspace("hardening".into(), "/tmp/hardening".into(), VcsKind::Git)
        .await
        .context("creating workspace")?;
    let task = store
        .create_task(workspace.id, "exercise store".into(), None)
        .await
        .context("creating task")?;
    let worktree = store
        .create_worktree(
            workspace.id,
            "/tmp/hardening".into(),
            "deadbeef".into(),
            None,
        )
        .await
        .context("creating worktree")?;
    let session = store
        .create_session(
            task.id,
            workspace.id,
            worktree.id,
            ctx_core::models::ExecutionEnvironment::Host,
            "fake".into(),
            "model".into(),
            "assistant".into(),
            None,
            None,
            None,
        )
        .await
        .context("creating session")?;

    let run_id = RunId::new();
    let turn_id = TurnId::new();
    store
        .append_session_event(
            session.id,
            Some(run_id),
            Some(turn_id),
            SessionEventType::Notice,
            serde_json::json!({ "kind": "hardening_check" }),
        )
        .await
        .context("appending session event")?;
    store
        .insert_message(Message {
            id: MessageId::new(),
            session_id: session.id,
            task_id: task.id,
            run_id: Some(run_id),
            turn_id: Some(turn_id),
            turn_sequence: None,
            order_seq: None,
            role: MessageRole::Assistant,
            content: "store hardening message".into(),
            attachments: vec![],
            delivery: MessageDelivery::Immediate,
            delivered_at: None,
            created_at: chrono::Utc::now(),
        })
        .await
        .context("inserting message")?;
    store.close().await;

    let reopened = Store::open(&db_path).await.context("reopening store")?;
    let events = reopened
        .list_session_events(session.id)
        .await
        .context("listing session events after reopen")?;
    let messages = reopened
        .list_messages_for_session(session.id)
        .await
        .context("listing messages after reopen")?;
    assert_eq!(events.len(), 1, "event log should survive reopen");
    assert_eq!(messages.len(), 1, "message write should survive reopen");
    reopened.close().await;

    assert_store_integrity(&db_path).await?;
    assert_eq!(journal_mode(&db_path).await?, "wal");

    Ok(())
}

#[tokio::test]
async fn cross_process_writes_wait_for_lock_release() -> Result<()> {
    if std::env::var_os(LOCK_CHILD_ENV).is_some() {
        return lock_holder_child().await;
    }

    let tempdir = tempfile::tempdir().context("creating tempdir")?;
    let db_path = tempdir.path().join("db.sqlite");
    let ready_path = tempdir.path().join("lock.ready");
    let release_path = tempdir.path().join("lock.release");

    let bootstrap_store = Store::open(&db_path)
        .await
        .context("bootstrapping sqlite store before lock test")?;
    bootstrap_store.close().await;

    let exe = std::env::current_exe().context("locating current test binary")?;
    let mut child = Command::new(exe);
    child
        .arg("--exact")
        .arg("cross_process_writes_wait_for_lock_release")
        .arg("--nocapture")
        .env(LOCK_CHILD_ENV, "1")
        .env(LOCK_DB_PATH_ENV, &db_path)
        .env(LOCK_READY_PATH_ENV, &ready_path)
        .env(LOCK_RELEASE_PATH_ENV, &release_path)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    let mut child = child
        .spawn()
        .context("spawning lock-holder child process")?;
    wait_for_file(&ready_path, Duration::from_secs(10))
        .await
        .context("waiting for lock-holder ready signal")?;

    let writer_store = Store::open(&db_path)
        .await
        .context("opening writer store while child holds lock")?;
    let started_at = Instant::now();
    let writer = tokio::spawn(async move {
        writer_store
            .create_workspace("contended".into(), "/tmp/contended".into(), VcsKind::Git)
            .await
    });

    sleep(Duration::from_millis(300)).await;
    assert!(
        !writer.is_finished(),
        "writer should still be waiting while the lock-holder keeps the transaction open"
    );

    fs::write(&release_path, b"release").context("writing release signal")?;
    let write_result = tokio::time::timeout(Duration::from_secs(10), writer)
        .await
        .context("timed out waiting for writer task")?
        .context("writer task join failed")?;
    write_result.context("writer should succeed after lock release")?;
    assert!(
        started_at.elapsed() >= Duration::from_millis(300),
        "writer should have observed lock contention before succeeding"
    );

    let status = tokio::time::timeout(Duration::from_secs(10), child.wait())
        .await
        .context("timed out waiting for lock-holder child")?
        .context("waiting for lock-holder child failed")?;
    assert!(status.success(), "lock-holder child exited with {status}");

    assert_store_integrity(&db_path).await?;
    Ok(())
}

async fn lock_holder_child() -> Result<()> {
    let db_path = env_path(LOCK_DB_PATH_ENV)?;
    let ready_path = env_path(LOCK_READY_PATH_ENV)?;
    let release_path = env_path(LOCK_RELEASE_PATH_ENV)?;

    let store = Store::open(&db_path)
        .await
        .context("opening child store for lock holder")?;
    let mut conn = store
        .pool()
        .acquire()
        .await
        .context("acquiring child connection")?;
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *conn)
        .await
        .context("beginning immediate transaction in child")?;
    fs::write(&ready_path, b"ready").context("writing ready signal")?;

    wait_for_file(&release_path, Duration::from_secs(10))
        .await
        .context("waiting for release signal")?;

    sqlx::query("COMMIT")
        .execute(&mut *conn)
        .await
        .context("committing child transaction")?;
    store.close().await;
    Ok(())
}

async fn assert_store_integrity(db_path: &Path) -> Result<()> {
    let sqlite_url = sqlite_url(db_path);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url)
        .await
        .with_context(|| format!("connecting integrity pool for {}", db_path.display()))?;
    let quick_check: String = sqlx::query_scalar("PRAGMA quick_check")
        .fetch_one(&pool)
        .await
        .context("running PRAGMA quick_check")?;
    assert_eq!(quick_check.to_lowercase(), "ok");

    let foreign_key_rows = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&pool)
        .await
        .context("running PRAGMA foreign_key_check")?;
    assert!(
        foreign_key_rows.is_empty(),
        "expected foreign_key_check to be empty, found {} row(s)",
        foreign_key_rows.len()
    );
    pool.close().await;
    Ok(())
}

async fn journal_mode(db_path: &Path) -> Result<String> {
    let sqlite_url = sqlite_url(db_path);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url)
        .await
        .with_context(|| format!("connecting journal_mode pool for {}", db_path.display()))?;
    let value: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(&pool)
        .await
        .context("querying journal_mode")?;
    pool.close().await;
    Ok(value.to_lowercase())
}

async fn applied_migration_count(db_path: &Path) -> Result<i64> {
    let sqlite_url = sqlite_url(db_path);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url)
        .await
        .with_context(|| format!("connecting migration count pool for {}", db_path.display()))?;
    let count: i64 =
        sqlx::query("SELECT COUNT(*) AS count FROM _sqlx_migrations WHERE success = 1")
            .fetch_one(&pool)
            .await
            .context("counting applied migrations")?
            .get("count");
    pool.close().await;
    Ok(count)
}

async fn applied_migrations(db_path: &Path) -> Result<Vec<(i64, String)>> {
    let sqlite_url = sqlite_url(db_path);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url)
        .await
        .with_context(|| format!("connecting migration details pool for {}", db_path.display()))?;
    let rows = sqlx::query("SELECT version, description FROM _sqlx_migrations ORDER BY version")
        .fetch_all(&pool)
        .await
        .context("listing applied migrations")?;
    pool.close().await;
    Ok(rows
        .into_iter()
        .map(|row| (row.get("version"), row.get("description")))
        .collect())
}

async fn column_exists(db_path: &Path, table: &str, column: &str) -> Result<bool> {
    let sqlite_url = sqlite_url(db_path);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url)
        .await
        .with_context(|| format!("connecting column check pool for {}", db_path.display()))?;
    let query = format!("SELECT COUNT(*) AS count FROM pragma_table_info('{table}') WHERE name = ?");
    let count = sqlx::query_scalar::<_, i64>(&query)
        .bind(column)
        .fetch_one(&pool)
        .await
        .with_context(|| format!("checking {table}.{column}"))?;
    pool.close().await;
    Ok(count > 0)
}

fn migration_files() -> Result<Vec<PathBuf>> {
    let migrations_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut files = fs::read_dir(&migrations_dir)
        .with_context(|| format!("reading migration dir {}", migrations_dir.display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("collecting migration paths")?;
    files.sort();
    Ok(files)
}

fn migration_version(path: &Path) -> Result<i64> {
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .with_context(|| format!("migration path missing UTF-8 filename: {}", path.display()))?;
    let version = filename
        .split_once('_')
        .with_context(|| format!("migration filename missing version prefix: {filename}"))?
        .0
        .parse::<i64>()
        .with_context(|| format!("parsing migration version from {filename}"))?;
    Ok(version)
}

fn sqlite_url(path: &Path) -> String {
    format!("sqlite://{}", path.to_string_lossy())
}

fn env_path(var: &str) -> Result<PathBuf> {
    Ok(PathBuf::from(
        std::env::var(var).with_context(|| format!("missing env var {var}"))?,
    ))
}

async fn wait_for_file(path: &Path, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if path.exists() {
            return Ok(());
        }
        sleep(Duration::from_millis(50)).await;
    }
    anyhow::bail!("timed out waiting for {}", path.display());
}
