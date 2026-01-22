use std::collections::HashMap;
use std::env;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use chrono::Utc;
use gpui::Context;
use gpui_tokio::Tokio;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Pool, Row, Sqlite};
use tokio::sync::OnceCell;

use ctx_core::ids::{SessionId, TurnId, WorkspaceId};
use ctx_core::models::{
    SessionHeadSnapshot, SessionHeadWindow, SessionHistoryPage, SessionSummaryCheckpoint,
    WorkspaceActiveSnapshot, WorkspaceArchivedPage,
};

use super::session::SessionThreadCache;
use super::ShellView;
use super::super::models::TurnToolSnapshot;

const HISTORY_CACHE_MAX_ROWS: i64 = 200;

#[derive(Clone)]
pub(crate) struct AtsCache {
    inner: Arc<AtsCacheInner>,
}

struct AtsCacheInner {
    db_path: PathBuf,
    pool: OnceCell<Pool<Sqlite>>,
}

impl AtsCache {
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(AtsCacheInner {
                db_path: cache_db_path(),
                pool: OnceCell::new(),
            }),
        }
    }

    async fn pool(&self) -> Option<&Pool<Sqlite>> {
        self.inner
            .pool
            .get_or_try_init(|| async { init_db(&self.inner.db_path).await })
            .await
            .ok()
    }

    pub(crate) async fn save_session_head(
        &self,
        session_id: SessionId,
        cache: &SessionThreadCache,
        meta: Option<&SessionHeadMeta>,
        last_event_seq: Option<i64>,
    ) -> Result<()> {
        let Some(last_event_seq) = last_event_seq else {
            return Ok(());
        };
        let Some(pool) = self.pool().await else {
            return Ok(());
        };

        let messages_json = serde_json::to_string(&cache.messages)?;
        let turns_json = serde_json::to_string(&cache.session_turns)?;
        let turn_tools_json = serde_json::to_string(&cache.session_turn_tools)?;
        let events_json = serde_json::to_string(&cache.session_events)?;
        let summary_checkpoint_json = meta
            .and_then(|meta| meta.summary_checkpoint.as_ref())
            .map(serde_json::to_string)
            .transpose()?;
        let head_window = meta
            .map(|meta| meta.head_window.clone())
            .unwrap_or_default();
        let head_window_json = serde_json::to_string(&head_window)?;
        let updated_at = Utc::now().timestamp_millis();

        sqlx::query(
            r#"
            INSERT INTO ats_session_heads (
                session_id,
                messages_json,
                turns_json,
                turn_tools_json,
                events_json,
                history_cursor,
                has_more_history,
                last_event_seq,
                summary_checkpoint_json,
                head_window_json,
                updated_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(session_id) DO UPDATE SET
                messages_json = excluded.messages_json,
                turns_json = excluded.turns_json,
                turn_tools_json = excluded.turn_tools_json,
                events_json = excluded.events_json,
                history_cursor = excluded.history_cursor,
                has_more_history = excluded.has_more_history,
                last_event_seq = excluded.last_event_seq,
                summary_checkpoint_json = excluded.summary_checkpoint_json,
                head_window_json = excluded.head_window_json,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(session_id.0.to_string())
        .bind(messages_json)
        .bind(turns_json)
        .bind(turn_tools_json)
        .bind(events_json)
        .bind(cache.history_cursor)
        .bind(if cache.has_more_history { 1 } else { 0 })
        .bind(last_event_seq)
        .bind(summary_checkpoint_json)
        .bind(head_window_json)
        .bind(updated_at)
        .execute(pool)
        .await?;

        Ok(())
    }

    pub(crate) async fn save_active_snapshot(
        &self,
        snapshot: &WorkspaceActiveSnapshot,
    ) -> Result<()> {
        let Some(pool) = self.pool().await else {
            return Ok(());
        };

        let snapshot_json = serde_json::to_string(snapshot)?;
        let updated_at = Utc::now().timestamp_millis();

        sqlx::query(
            r#"
            INSERT INTO ats_active_snapshots (
                workspace_id,
                snapshot_json,
                updated_at
            )
            VALUES (?, ?, ?)
            ON CONFLICT(workspace_id) DO UPDATE SET
                snapshot_json = excluded.snapshot_json,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(snapshot.workspace_id.0.to_string())
        .bind(snapshot_json)
        .bind(updated_at)
        .execute(pool)
        .await?;

        Ok(())
    }

    pub(crate) async fn load_active_snapshot(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Option<WorkspaceActiveSnapshot>> {
        let Some(pool) = self.pool().await else {
            return Ok(None);
        };

        let row = sqlx::query(
            r#"
            SELECT snapshot_json
            FROM ats_active_snapshots
            WHERE workspace_id = ?
            "#,
        )
        .bind(workspace_id.0.to_string())
        .fetch_optional(pool)
        .await?;

        let Some(row) = row else {
            return Ok(None);
        };

        let snapshot_json: String = row.try_get("snapshot_json")?;
        let snapshot = serde_json::from_str(&snapshot_json)?;
        Ok(Some(snapshot))
    }

    pub(crate) async fn load_session_head(
        &self,
        session_id: SessionId,
    ) -> Result<Option<CachedSessionHead>> {
        let Some(pool) = self.pool().await else {
            return Ok(None);
        };

        let row = sqlx::query(
            r#"
            SELECT
                messages_json,
                turns_json,
                turn_tools_json,
                events_json,
                history_cursor,
                has_more_history,
                last_event_seq,
                summary_checkpoint_json,
                head_window_json
            FROM ats_session_heads
            WHERE session_id = ?
            "#,
        )
        .bind(session_id.0.to_string())
        .fetch_optional(pool)
        .await?;

        let Some(row) = row else {
            return Ok(None);
        };

        let messages_json: String = row.try_get("messages_json")?;
        let turns_json: String = row.try_get("turns_json")?;
        let turn_tools_json: String = row.try_get("turn_tools_json")?;
        let events_json: String = row.try_get("events_json")?;
        let history_cursor: Option<i64> = row.try_get("history_cursor")?;
        let has_more_history: i64 = row.try_get("has_more_history")?;
        let last_event_seq: i64 = row.try_get("last_event_seq")?;
        let summary_checkpoint_json: Option<String> =
            row.try_get("summary_checkpoint_json")?;
        let head_window_json: String = row.try_get("head_window_json")?;

        let messages = serde_json::from_str(&messages_json)?;
        let session_turns = serde_json::from_str(&turns_json)?;
        let session_turn_tools: HashMap<TurnId, Vec<TurnToolSnapshot>> =
            serde_json::from_str(&turn_tools_json)?;
        let session_events = serde_json::from_str(&events_json)?;
        let summary_checkpoint = summary_checkpoint_json
            .as_deref()
            .map(serde_json::from_str)
            .transpose()?;
        let head_window = serde_json::from_str(&head_window_json)?;

        let cache = SessionThreadCache {
            messages,
            session_turns,
            session_turn_tools,
            session_events,
            history_cursor,
            has_more_history: has_more_history != 0,
        }
        .stripped_partials();

        Ok(Some(CachedSessionHead {
            cache,
            meta: SessionHeadMeta {
                head_window,
                summary_checkpoint,
            },
            last_event_seq,
        }))
    }

    pub(crate) async fn save_archived_head_window(
        &self,
        workspace_id: WorkspaceId,
        page: &WorkspaceArchivedPage,
    ) -> Result<()> {
        let Some(pool) = self.pool().await else {
            return Ok(());
        };

        let page_json = serde_json::to_string(page)?;
        let updated_at = Utc::now().timestamp_millis();

        sqlx::query(
            r#"
            INSERT INTO ats_archived_head_windows (
                workspace_id,
                page_json,
                updated_at
            )
            VALUES (?, ?, ?)
            ON CONFLICT(workspace_id) DO UPDATE SET
                page_json = excluded.page_json,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(workspace_id.0.to_string())
        .bind(page_json)
        .bind(updated_at)
        .execute(pool)
        .await?;

        Ok(())
    }

    pub(crate) async fn load_archived_head_window(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Option<WorkspaceArchivedPage>> {
        let Some(pool) = self.pool().await else {
            return Ok(None);
        };

        let row = sqlx::query(
            r#"
            SELECT page_json
            FROM ats_archived_head_windows
            WHERE workspace_id = ?
            "#,
        )
        .bind(workspace_id.0.to_string())
        .fetch_optional(pool)
        .await?;

        let Some(row) = row else {
            return Ok(None);
        };

        let page_json: String = row.try_get("page_json")?;
        let page = serde_json::from_str(&page_json)?;
        Ok(Some(page))
    }

    pub(crate) async fn load_session_history_page(
        &self,
        session_id: SessionId,
        before_seq: i64,
        limit: u32,
    ) -> Result<Option<SessionHistoryPage>> {
        let Some(pool) = self.pool().await else {
            return Ok(None);
        };

        let row = sqlx::query(
            r#"
            SELECT turns_json, messages_json, has_more, next_cursor
            FROM session_history_pages
            WHERE session_id = ? AND before_seq = ? AND "limit" = ?
            "#,
        )
        .bind(session_id.0.to_string())
        .bind(before_seq)
        .bind(i64::from(limit))
        .fetch_optional(pool)
        .await?;

        let Some(row) = row else {
            return Ok(None);
        };

        let turns_json: String = row.try_get("turns_json")?;
        let messages_json: String = row.try_get("messages_json")?;
        let has_more: i64 = row.try_get("has_more")?;
        let next_cursor: Option<i64> = row.try_get("next_cursor")?;

        let turns = serde_json::from_str(&turns_json)?;
        let messages = serde_json::from_str(&messages_json)?;

        let updated_at = Utc::now().timestamp_millis();
        sqlx::query(
            r#"
            UPDATE session_history_pages
            SET updated_at = ?
            WHERE session_id = ? AND before_seq = ? AND "limit" = ?
            "#,
        )
        .bind(updated_at)
        .bind(session_id.0.to_string())
        .bind(before_seq)
        .bind(i64::from(limit))
        .execute(pool)
        .await?;

        Ok(Some(SessionHistoryPage {
            session_id,
            turns,
            messages,
            next_cursor,
            has_more: has_more != 0,
        }))
    }

    pub(crate) async fn save_session_history_page(
        &self,
        before_seq: i64,
        limit: u32,
        history: &SessionHistoryPage,
    ) -> Result<()> {
        let Some(pool) = self.pool().await else {
            return Ok(());
        };

        let turns_json = serde_json::to_string(&history.turns)?;
        let messages_json = serde_json::to_string(&history.messages)?;
        let updated_at = Utc::now().timestamp_millis();

        sqlx::query(
            r#"
            INSERT INTO session_history_pages (
                session_id,
                before_seq,
                "limit",
                turns_json,
                messages_json,
                has_more,
                next_cursor,
                updated_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(session_id, before_seq, "limit") DO UPDATE SET
                turns_json = excluded.turns_json,
                messages_json = excluded.messages_json,
                has_more = excluded.has_more,
                next_cursor = excluded.next_cursor,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(history.session_id.0.to_string())
        .bind(before_seq)
        .bind(i64::from(limit))
        .bind(turns_json)
        .bind(messages_json)
        .bind(if history.has_more { 1 } else { 0 })
        .bind(history.next_cursor)
        .bind(updated_at)
        .execute(pool)
        .await?;

        prune_history_pages(pool).await?;
        Ok(())
    }
}

pub(crate) struct CachedSessionHead {
    pub(crate) cache: SessionThreadCache,
    pub(crate) meta: SessionHeadMeta,
    pub(crate) last_event_seq: i64,
}

#[derive(Clone, Default)]
pub(crate) struct SessionHeadMeta {
    pub(crate) head_window: SessionHeadWindow,
    pub(crate) summary_checkpoint: Option<SessionSummaryCheckpoint>,
}

impl SessionHeadMeta {
    pub(crate) fn from_head(head: &SessionHeadSnapshot) -> Self {
        Self {
            head_window: head.head_window.clone(),
            summary_checkpoint: head.summary_checkpoint.clone(),
        }
    }
}

impl ShellView {
    pub(crate) fn update_session_head_meta(
        &mut self,
        session_id: SessionId,
        head: &SessionHeadSnapshot,
    ) {
        self.session_head_meta
            .insert(session_id, SessionHeadMeta::from_head(head));
    }

    pub(crate) fn persist_cached_session_head(
        &self,
        session_id: SessionId,
        cx: &mut Context<Self>,
    ) {
        let Some(cache) = self.session_thread_cache.get(&session_id).cloned() else {
            return;
        };
        let cache = cache.stripped_partials();
        let meta = self.session_head_meta.get(&session_id).cloned();
        let last_event_seq = self.session_last_event_seq.get(&session_id).copied();
        let ats_cache = self.ats_cache.clone();

        Tokio::spawn(cx, async move {
            let _ = ats_cache
                .save_session_head(session_id, &cache, meta.as_ref(), last_event_seq)
                .await;
        })
        .detach();
    }
}

fn cache_db_path() -> PathBuf {
    let base = env::var("CTX_DATA_DIR")
        .ok()
        .or_else(|| env::var("HOME").ok())
        .or_else(|| env::var("USERPROFILE").ok())
        .or_else(|| {
            let drive = env::var("HOMEDRIVE").ok()?;
            let path = env::var("HOMEPATH").ok()?;
            Some(format!("{}{}", drive, path))
        })
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    base.join(".ctx").join("native_ats_cache.sqlite")
}

async fn init_db(path: &PathBuf) -> Result<Pool<Sqlite>> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!("creating cache directory {}", parent.display())
        })?;
    }

    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .context("opening native cache sqlite db")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS ats_active_snapshots (
            workspace_id TEXT PRIMARY KEY,
            snapshot_json TEXT NOT NULL,
            updated_at INTEGER NOT NULL
        )
        "#,
    )
    .execute(&pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS ats_archived_head_windows (
            workspace_id TEXT PRIMARY KEY,
            page_json TEXT NOT NULL,
            updated_at INTEGER NOT NULL
        )
        "#,
    )
    .execute(&pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS ats_session_heads (
            session_id TEXT PRIMARY KEY,
            messages_json TEXT NOT NULL,
            turns_json TEXT NOT NULL,
            turn_tools_json TEXT NOT NULL,
            events_json TEXT NOT NULL,
            history_cursor INTEGER,
            has_more_history INTEGER NOT NULL,
            last_event_seq INTEGER NOT NULL,
            summary_checkpoint_json TEXT,
            head_window_json TEXT NOT NULL,
            updated_at INTEGER NOT NULL
        )
        "#,
    )
    .execute(&pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS session_history_pages (
            session_id TEXT NOT NULL,
            before_seq INTEGER NOT NULL,
            "limit" INTEGER NOT NULL,
            turns_json TEXT NOT NULL,
            messages_json TEXT NOT NULL,
            has_more INTEGER NOT NULL,
            next_cursor INTEGER,
            updated_at INTEGER NOT NULL,
            PRIMARY KEY (session_id, before_seq, "limit")
        )
        "#,
    )
    .execute(&pool)
    .await?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_session_history_pages_updated_at
        ON session_history_pages(updated_at)
        "#,
    )
    .execute(&pool)
    .await?;

    Ok(pool)
}

async fn prune_history_pages(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        r#"
        DELETE FROM session_history_pages
        WHERE rowid NOT IN (
            SELECT rowid
            FROM session_history_pages
            ORDER BY updated_at DESC
            LIMIT ?
        )
        "#,
    )
    .bind(HISTORY_CACHE_MAX_ROWS)
    .execute(pool)
    .await?;
    Ok(())
}
