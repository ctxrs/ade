use super::*;
use ctx_core::boolish::parse_boolish;

pub(super) const DEFAULT_EVENT_LOG_FLUSH_MS: u64 = 250;
pub(super) const DEFAULT_EVENT_LOG_BATCH_SIZE: usize = 256;
pub(super) const DEFAULT_EVENT_LOG_CHECKPOINT_MS: u64 = 5_000;
pub(super) const EVENT_LOG_QUEUE_CAPACITY: usize = 4096;

#[derive(Clone, Copy, Debug)]
pub(super) struct EventLogConfig {
    flush_interval: Duration,
    batch_size: usize,
    checkpoint_interval: Duration,
}

impl EventLogConfig {
    fn from_env() -> Self {
        let flush_ms = env_u64("CTX_EVENT_LOG_FLUSH_MS").unwrap_or(DEFAULT_EVENT_LOG_FLUSH_MS);
        let batch_size =
            env_usize("CTX_EVENT_LOG_BATCH_SIZE").unwrap_or(DEFAULT_EVENT_LOG_BATCH_SIZE);
        let checkpoint_ms =
            env_u64("CTX_EVENT_LOG_CHECKPOINT_MS").unwrap_or(DEFAULT_EVENT_LOG_CHECKPOINT_MS);
        Self {
            flush_interval: Duration::from_millis(flush_ms.max(1)),
            batch_size: batch_size.max(1),
            checkpoint_interval: Duration::from_millis(checkpoint_ms.max(1)),
        }
    }
}

pub(super) struct EventLogRuntime {
    next_seq: AtomicI64,
    config: EventLogConfig,
    persister: OnceLock<EventLogPersister>,
}

impl EventLogRuntime {
    pub(super) async fn load(pool: &Pool<Sqlite>) -> Result<Self> {
        let last_seq: Option<i64> = sqlx::query_scalar("SELECT MAX(seq) FROM session_events")
            .fetch_one(pool)
            .await?;
        let checkpoint_seq: Option<i64> =
            sqlx::query_scalar("SELECT checkpoint_seq FROM event_log_checkpoints WHERE id = 1")
                .fetch_optional(pool)
                .await?
                .flatten();
        let max_seq = last_seq.unwrap_or(0).max(checkpoint_seq.unwrap_or(0));
        Ok(Self {
            next_seq: AtomicI64::new(max_seq.saturating_add(1)),
            config: EventLogConfig::from_env(),
            persister: OnceLock::new(),
        })
    }

    pub(super) fn start_persister(&self, store: Store) {
        let _ = self.persister.get_or_init(|| {
            let initial_seq = self.next_seq.load(Ordering::Relaxed).saturating_sub(1);
            EventLogPersister::spawn(store, self.config, initial_seq)
        });
    }

    pub(super) fn next_seq(&self) -> i64 {
        self.next_seq.fetch_add(1, Ordering::Relaxed)
    }

    pub(super) async fn enqueue(&self, event: SessionEvent) -> Result<()> {
        match self.persister.get() {
            Some(persister) => persister.enqueue(event).await,
            None => Err(anyhow::anyhow!("event log persister unavailable")),
        }
    }

    pub(super) async fn flush(&self) -> Result<()> {
        match self.persister.get() {
            Some(persister) => persister.flush().await,
            None => Ok(()),
        }
    }
}

#[derive(Clone)]
pub(super) struct EventLogPersister {
    tx: mpsc::Sender<EventLogCommand>,
}

pub(super) enum EventLogCommand {
    Event(SessionEvent),
    Flush(oneshot::Sender<Result<()>>),
}

impl EventLogPersister {
    fn spawn(store: Store, config: EventLogConfig, initial_seq: i64) -> Self {
        let (tx, mut rx) = mpsc::channel(EVENT_LOG_QUEUE_CAPACITY);
        tokio::spawn(async move {
            let mut buffer: Vec<SessionEvent> = Vec::new();
            let mut flush_waiters: Vec<oneshot::Sender<Result<()>>> = Vec::new();
            let mut last_applied_seq = initial_seq;
            let mut last_checkpoint_seq = initial_seq;
            let mut flush_interval = tokio::time::interval(config.flush_interval);
            let mut checkpoint_interval = tokio::time::interval(config.checkpoint_interval);

            loop {
                tokio::select! {
                    cmd = rx.recv() => {
                        match cmd {
                            Some(EventLogCommand::Event(event)) => {
                                last_applied_seq = last_applied_seq.max(event.seq);
                                buffer.push(event);
                                if buffer.len() >= config.batch_size {
                                    if let Err(err) = flush_event_batch(&store, &mut buffer).await {
                                        tracing::warn!("event log flush failed: {err:#}");
                                    }
                                }
                            }
                            Some(EventLogCommand::Flush(tx)) => {
                                flush_waiters.push(tx);
                                let result = if buffer.is_empty() {
                                    Ok(())
                                } else {
                                    flush_event_batch(&store, &mut buffer).await
                                };
                                for waiter in flush_waiters.drain(..) {
                                    let send_result = match &result {
                                        Ok(()) => Ok(()),
                                        Err(err) => Err(anyhow::anyhow!("{err:#}")),
                                    };
                                    let _ = waiter.send(send_result);
                                }
                            }
                            None => {
                                let _ = flush_event_batch(&store, &mut buffer).await;
                                return;
                            }
                        }
                    }
                    _ = flush_interval.tick() => {
                        if !buffer.is_empty() {
                            if let Err(err) = flush_event_batch(&store, &mut buffer).await {
                                tracing::warn!("event log flush failed: {err:#}");
                            }
                        }
                    }
                    _ = checkpoint_interval.tick() => {
                        if last_applied_seq > last_checkpoint_seq {
                            let result = store
                                .upsert_event_log_checkpoint(last_applied_seq, None)
                                .await;
                            if let Err(err) = result {
                                tracing::warn!("event log checkpoint failed: {err:#}");
                            } else {
                                last_checkpoint_seq = last_applied_seq;
                            }
                        }
                    }
                }
            }
        });
        Self { tx }
    }

    async fn enqueue(&self, event: SessionEvent) -> Result<()> {
        self.tx
            .send(EventLogCommand::Event(event))
            .await
            .context("enqueueing session event for persistence")
    }

    async fn flush(&self) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(EventLogCommand::Flush(tx))
            .await
            .context("requesting event log flush")?;
        rx.await.context("waiting for event log flush")?
    }
}

pub(super) async fn flush_event_batch(store: &Store, buffer: &mut Vec<SessionEvent>) -> Result<()> {
    if buffer.is_empty() {
        return Ok(());
    }
    let batch = std::mem::take(buffer);
    if let Err(err) = store.persist_session_events_batch(&batch).await {
        buffer.extend(batch);
        return Err(err);
    }
    Ok(())
}

pub(super) fn snapshot_timing_enabled() -> bool {
    std::env::var("CTX_SNAPSHOT_TIMING")
        .ok()
        .as_deref()
        .and_then(parse_boolish)
        .unwrap_or(false)
}

pub(super) const WRITE_METRICS_INTERVAL_SECS: u64 = 10;
pub(super) const WRITE_METRICS_TABLE_COUNT: usize = 8;
pub(super) const I64_BYTES: u64 = 8;
pub(super) const BOOL_BYTES: u64 = 1;

#[derive(Clone, Copy, Debug)]
pub(super) enum WriteMetricTable {
    SessionEvents,
    SessionTurns,
    SessionTurnTools,
    Messages,
    SessionHeadMaterializations,
    SessionActiveSnapshotHeads,
    WorkspaceActiveTaskSummaries,
    SessionSnapshotSummaries,
}

impl WriteMetricTable {
    const ALL: [WriteMetricTable; WRITE_METRICS_TABLE_COUNT] = [
        WriteMetricTable::SessionEvents,
        WriteMetricTable::SessionTurns,
        WriteMetricTable::SessionTurnTools,
        WriteMetricTable::Messages,
        WriteMetricTable::SessionHeadMaterializations,
        WriteMetricTable::SessionActiveSnapshotHeads,
        WriteMetricTable::WorkspaceActiveTaskSummaries,
        WriteMetricTable::SessionSnapshotSummaries,
    ];

    fn index(self) -> usize {
        match self {
            WriteMetricTable::SessionEvents => 0,
            WriteMetricTable::SessionTurns => 1,
            WriteMetricTable::SessionTurnTools => 2,
            WriteMetricTable::Messages => 3,
            WriteMetricTable::SessionHeadMaterializations => 4,
            WriteMetricTable::SessionActiveSnapshotHeads => 5,
            WriteMetricTable::WorkspaceActiveTaskSummaries => 6,
            WriteMetricTable::SessionSnapshotSummaries => 7,
        }
    }

    fn is_base(self) -> bool {
        matches!(
            self,
            WriteMetricTable::SessionEvents
                | WriteMetricTable::SessionTurns
                | WriteMetricTable::SessionTurnTools
                | WriteMetricTable::Messages
        )
    }
}

pub(super) struct WriteMetrics {
    bytes: [AtomicU64; WRITE_METRICS_TABLE_COUNT],
    writes: [AtomicU64; WRITE_METRICS_TABLE_COUNT],
}

impl WriteMetrics {
    fn new() -> Self {
        Self {
            bytes: std::array::from_fn(|_| AtomicU64::new(0)),
            writes: std::array::from_fn(|_| AtomicU64::new(0)),
        }
    }

    fn record(&self, table: WriteMetricTable, rows: u64, bytes: u64) {
        let index = table.index();
        self.bytes[index].fetch_add(bytes, Ordering::Relaxed);
        self.writes[index].fetch_add(rows, Ordering::Relaxed);
    }

    fn snapshot(&self) -> WriteMetricsSnapshot {
        WriteMetricsSnapshot {
            bytes: std::array::from_fn(|i| self.bytes[i].load(Ordering::Relaxed)),
            writes: std::array::from_fn(|i| self.writes[i].load(Ordering::Relaxed)),
        }
    }
}

#[derive(Clone)]
pub(super) struct WriteMetricsSnapshot {
    bytes: [u64; WRITE_METRICS_TABLE_COUNT],
    writes: [u64; WRITE_METRICS_TABLE_COUNT],
}

impl WriteMetricsSnapshot {
    fn delta(&self, previous: &WriteMetricsSnapshot) -> WriteMetricsSnapshot {
        WriteMetricsSnapshot {
            bytes: std::array::from_fn(|i| self.bytes[i].saturating_sub(previous.bytes[i])),
            writes: std::array::from_fn(|i| self.writes[i].saturating_sub(previous.writes[i])),
        }
    }

    fn total_bytes(&self) -> u64 {
        self.bytes.iter().sum()
    }

    fn total_writes(&self) -> u64 {
        self.writes.iter().sum()
    }

    fn base_bytes(&self) -> u64 {
        WriteMetricTable::ALL
            .iter()
            .filter(|table| table.is_base())
            .map(|table| self.bytes[table.index()])
            .sum()
    }
}

pub(super) fn write_metrics_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("CTX_WRITE_METRICS")
            .ok()
            .as_deref()
            .and_then(parse_boolish)
            .unwrap_or(false)
    })
}

pub(super) fn write_metrics() -> Option<&'static WriteMetrics> {
    if !write_metrics_enabled() {
        return None;
    }
    static METRICS: OnceLock<WriteMetrics> = OnceLock::new();
    static LOGGER: OnceLock<()> = OnceLock::new();
    let metrics = METRICS.get_or_init(WriteMetrics::new);
    LOGGER.get_or_init(|| {
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let mut interval =
                    tokio::time::interval(Duration::from_secs(WRITE_METRICS_INTERVAL_SECS));
                let mut previous = metrics.snapshot();
                loop {
                    interval.tick().await;
                    let current = metrics.snapshot();
                    let delta = current.delta(&previous);
                    previous = current;
                    let total_bytes = delta.total_bytes();
                    let total_writes = delta.total_writes();
                    if total_bytes == 0 && total_writes == 0 {
                        continue;
                    }
                    let base_bytes = delta.base_bytes();
                    let derived_bytes = total_bytes.saturating_sub(base_bytes);
                    let write_amplification =
                        (base_bytes > 0).then(|| total_bytes as f64 / base_bytes as f64);

                    info!(
                        target: "ctx_store.write_metrics",
                        interval_s = WRITE_METRICS_INTERVAL_SECS,
                        total_writes,
                        total_bytes,
                        base_bytes,
                        derived_bytes,
                        write_amplification,
                        session_events_writes =
                            delta.writes[WriteMetricTable::SessionEvents.index()],
                        session_events_bytes =
                            delta.bytes[WriteMetricTable::SessionEvents.index()],
                        session_turns_writes = delta.writes[WriteMetricTable::SessionTurns.index()],
                        session_turns_bytes = delta.bytes[WriteMetricTable::SessionTurns.index()],
                        session_turn_tools_writes =
                            delta.writes[WriteMetricTable::SessionTurnTools.index()],
                        session_turn_tools_bytes =
                            delta.bytes[WriteMetricTable::SessionTurnTools.index()],
                        messages_writes = delta.writes[WriteMetricTable::Messages.index()],
                        messages_bytes = delta.bytes[WriteMetricTable::Messages.index()],
                        session_head_materializations_writes = delta.writes
                            [WriteMetricTable::SessionHeadMaterializations.index()],
                        session_head_materializations_bytes = delta.bytes
                            [WriteMetricTable::SessionHeadMaterializations.index()],
                        session_active_snapshot_heads_writes = delta.writes
                            [WriteMetricTable::SessionActiveSnapshotHeads.index()],
                        session_active_snapshot_heads_bytes = delta.bytes
                            [WriteMetricTable::SessionActiveSnapshotHeads.index()],
                        workspace_active_task_summaries_writes = delta.writes
                            [WriteMetricTable::WorkspaceActiveTaskSummaries.index()],
                        workspace_active_task_summaries_bytes = delta.bytes
                            [WriteMetricTable::WorkspaceActiveTaskSummaries.index()],
                        session_snapshot_summaries_writes = delta.writes
                            [WriteMetricTable::SessionSnapshotSummaries.index()],
                        session_snapshot_summaries_bytes = delta.bytes
                            [WriteMetricTable::SessionSnapshotSummaries.index()],
                    );
                }
            });
        } else {
            info!(
                target: "ctx_store.write_metrics",
                "CTX_WRITE_METRICS enabled without a tokio runtime; write metrics logging disabled",
            );
        }
    });
    Some(metrics)
}

pub(super) fn record_write(table: WriteMetricTable, rows: u64, bytes_per_row: u64) {
    if rows == 0 {
        return;
    }
    if let Some(metrics) = write_metrics() {
        let bytes = bytes_per_row.saturating_mul(rows);
        metrics.record(table, rows, bytes);
    }
}

pub(super) fn bytes_str(value: &str) -> u64 {
    value.len() as u64
}

pub(super) fn bytes_opt_str(value: Option<&str>) -> u64 {
    value.map(bytes_str).unwrap_or(0)
}

pub(super) fn bytes_opt_i64(value: Option<i64>) -> u64 {
    value.map(|_| I64_BYTES).unwrap_or(0)
}

pub(super) fn env_u64(name: &str) -> Option<u64> {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
}

pub(super) fn env_usize(name: &str) -> Option<usize> {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
}

pub(super) fn env_flag_enabled(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .as_deref()
        .and_then(parse_boolish)
        .unwrap_or(false)
}

pub(super) fn disable_head_materialization_writes() -> bool {
    env_flag_enabled("CTX_DISABLE_HEAD_MATERIALIZATION")
}

pub(super) fn disable_tool_summary_persistence() -> bool {
    env_flag_enabled("CTX_DISABLE_TOOL_SUMMARY_PERSISTENCE")
}
