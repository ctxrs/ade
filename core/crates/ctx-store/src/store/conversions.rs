use super::*;

pub(super) fn build_mobile_connection_profile_from_row(
    row: SqliteRow,
) -> Result<MobileConnectionProfile> {
    let id: String = row.try_get("id")?;
    let scopes_json: String = row.try_get("scopes_json")?;
    let created_at: String = row.try_get("created_at")?;
    let last_used_at: Option<String> = row.try_get("last_used_at")?;
    let scopes: Vec<String> = serde_json::from_str(&scopes_json).unwrap_or_default();
    Ok(MobileConnectionProfile {
        id: ConnectionProfileId(uuid::Uuid::parse_str(&id)?),
        label: row.try_get("label")?,
        base_url: row.try_get("base_url")?,
        token_prefix: row.try_get("token_prefix")?,
        scopes,
        created_at: parse_dt(&created_at)?,
        last_used_at: last_used_at.as_deref().map(parse_dt).transpose()?,
    })
}

pub(super) fn build_mobile_device_from_row(row: SqliteRow) -> Result<MobileDeviceRegistration> {
    let id: String = row.try_get("id")?;
    let profile_id: String = row.try_get("profile_id")?;
    let created_at: String = row.try_get("created_at")?;
    let last_seen_at: String = row.try_get("last_seen_at")?;
    Ok(MobileDeviceRegistration {
        id: MobileDeviceId(uuid::Uuid::parse_str(&id)?),
        profile_id: ConnectionProfileId(uuid::Uuid::parse_str(&profile_id)?),
        device_label: row.try_get("device_label")?,
        platform: row.try_get("platform")?,
        push_token: row.try_get("push_token")?,
        push_provider: row.try_get("push_provider")?,
        public_key: row.try_get("public_key")?,
        app_version: row.try_get("app_version")?,
        created_at: parse_dt(&created_at)?,
        last_seen_at: parse_dt(&last_seen_at)?,
    })
}

pub(super) fn map_merge_queue_entry(row: SqliteRow) -> Option<MergeQueueEntry> {
    let id: String = row.try_get("id").ok()?;
    let workspace_id: String = row.try_get("workspace_id").ok()?;
    let worktree_id: Option<String> = row.try_get("worktree_id").ok()?;
    let session_id: Option<String> = row.try_get("session_id").ok()?;
    let target_branch: String = row.try_get("target_branch").ok()?;
    let patch_source: String = row.try_get("patch_source").ok()?;
    let created_at: String = row.try_get("created_at").ok()?;
    let updated_at: String = row.try_get("updated_at").ok()?;
    let status: String = row.try_get("status").ok()?;
    Some(MergeQueueEntry {
        id: MergeQueueEntryId(uuid::Uuid::parse_str(&id).ok()?),
        workspace_id: WorkspaceId(uuid::Uuid::parse_str(&workspace_id).ok()?),
        worktree_id: worktree_id
            .and_then(|value| uuid::Uuid::parse_str(&value).ok())
            .map(WorktreeId),
        session_id: parse_optional_session_id(session_id),
        target_branch,
        message: row.try_get("message").ok(),
        patch_source: parse_merge_queue_patch_source(&patch_source),
        base_commit_sha: row.try_get("base_commit_sha").ok(),
        head_commit_sha: row.try_get("head_commit_sha").ok(),
        patch_path: row.try_get("patch_path").ok()?,
        patch_size: row.try_get("patch_size").ok()?,
        status: parse_merge_queue_entry_status(&status),
        result_commit_sha: row.try_get("result_commit_sha").ok(),
        error_message: row.try_get("error_message").ok(),
        created_at: parse_dt(&created_at).ok()?,
        updated_at: parse_dt(&updated_at).ok()?,
    })
}

pub(super) fn map_merge_queue_run(row: SqliteRow) -> Option<MergeQueueRun> {
    let id: String = row.try_get("id").ok()?;
    let entry_id: String = row.try_get("entry_id").ok()?;
    let status: String = row.try_get("status").ok()?;
    let started_at: String = row.try_get("started_at").ok()?;
    let finished_at: Option<String> = row.try_get("finished_at").ok()?;
    Some(MergeQueueRun {
        id: MergeQueueRunId(uuid::Uuid::parse_str(&id).ok()?),
        entry_id: MergeQueueEntryId(uuid::Uuid::parse_str(&entry_id).ok()?),
        status: parse_merge_queue_run_status(&status),
        started_at: parse_dt(&started_at).ok()?,
        finished_at: finished_at.as_deref().and_then(|v| parse_dt(v).ok()),
        exit_code: row.try_get("exit_code").ok(),
        log_path: row.try_get("log_path").ok(),
        error_message: row.try_get("error_message").ok(),
        result_commit_sha: row.try_get("result_commit_sha").ok(),
    })
}

pub(super) struct SessionSnapshotRow {
    pub(super) session: Session,
    pub(super) last_message_at: Option<DateTime<Utc>>,
    pub(super) last_message_preview: Option<String>,
    pub(super) last_event_seq: Option<i64>,
    pub(super) activity: SessionActivityState,
}

pub(super) fn session_metadata_from_session(session: &Session) -> SessionMetadata {
    SessionMetadata {
        id: session.id,
        task_id: session.task_id,
        workspace_id: session.workspace_id,
        worktree_id: session.worktree_id,
        parent_session_id: session.parent_session_id,
        relationship: session.relationship.clone(),
        provider_id: session.provider_id.clone(),
        model_id: session.model_id.clone(),
        title: session.title.clone(),
        agent_role: session.agent_role.clone(),
        status: session.status.clone(),
        provider_session_ref: session.provider_session_ref.clone(),
        created_at: session.created_at,
        updated_at: session.updated_at,
    }
}

pub(super) fn session_head_to_snapshot(head: SessionHead) -> SessionHeadSnapshot {
    SessionHeadSnapshot {
        session: session_metadata_from_session(&head.session),
        turns: head.turns,
        tool_summaries: head.tool_summaries,
        events: head.events,
        messages: head.messages,
        last_event_seq: head.last_event_seq,
        state_rev: head.last_event_seq,
        activity: head.activity,
        has_more_turns: head.has_more_turns,
        history_cursor: None,
        has_more_history: false,
        summary_checkpoint: head.summary_checkpoint,
        head_window: head.head_window,
    }
}

pub(super) fn derive_message_preview(content: &str) -> String {
    let trimmed = content.trim();
    let line = trimmed.lines().next().unwrap_or("").trim();
    if line.is_empty() {
        return String::new();
    }
    const MAX_CHARS: usize = 160;
    let mut out: String = line.chars().take(MAX_CHARS).collect();
    if line.chars().count() > MAX_CHARS {
        out.push_str("...");
    }
    out
}

pub(super) fn derive_activity_from_status(
    last_status: Option<SessionTurnStatus>,
    has_running_turn: bool,
) -> SessionActivityState {
    SessionActivityState {
        is_working: has_running_turn,
        last_turn_status: last_status,
    }
}

pub(super) fn parse_dt(value: &str) -> Result<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(value)?.with_timezone(&Utc))
}

pub(super) fn task_status_to_str(status: &TaskStatus) -> &'static str {
    match status {
        TaskStatus::Pending => "pending",
        TaskStatus::Running => "running",
        TaskStatus::Completed => "completed",
        TaskStatus::Failed => "failed",
        TaskStatus::Cancelled => "cancelled",
    }
}

pub(super) fn parse_task_status(value: &str) -> TaskStatus {
    match value {
        "pending" => TaskStatus::Pending,
        "running" => TaskStatus::Running,
        "completed" => TaskStatus::Completed,
        "failed" => TaskStatus::Failed,
        "cancelled" => TaskStatus::Cancelled,
        _ => TaskStatus::Pending,
    }
}

pub(super) fn session_status_to_str(status: &SessionStatus) -> &'static str {
    match status {
        SessionStatus::Active => "active",
        SessionStatus::Completed => "completed",
        SessionStatus::Failed => "failed",
        SessionStatus::Cancelled => "cancelled",
    }
}

pub(super) fn parse_session_status(value: &str) -> SessionStatus {
    match value {
        "active" => SessionStatus::Active,
        "completed" => SessionStatus::Completed,
        "failed" => SessionStatus::Failed,
        "cancelled" => SessionStatus::Cancelled,
        _ => SessionStatus::Active,
    }
}

pub(super) fn merge_queue_entry_status_to_str(status: &MergeQueueEntryStatus) -> &'static str {
    match status {
        MergeQueueEntryStatus::Queued => "queued",
        MergeQueueEntryStatus::Running => "running",
        MergeQueueEntryStatus::Passed => "passed",
        MergeQueueEntryStatus::Failed => "failed",
        MergeQueueEntryStatus::Conflict => "conflict",
        MergeQueueEntryStatus::Cancelled => "cancelled",
    }
}

pub(super) fn parse_merge_queue_entry_status(value: &str) -> MergeQueueEntryStatus {
    match value {
        "queued" => MergeQueueEntryStatus::Queued,
        "running" => MergeQueueEntryStatus::Running,
        "passed" => MergeQueueEntryStatus::Passed,
        "failed" => MergeQueueEntryStatus::Failed,
        "conflict" => MergeQueueEntryStatus::Conflict,
        "cancelled" => MergeQueueEntryStatus::Cancelled,
        _ => MergeQueueEntryStatus::Queued,
    }
}

pub(super) fn merge_queue_run_status_to_str(status: &MergeQueueRunStatus) -> &'static str {
    match status {
        MergeQueueRunStatus::Running => "running",
        MergeQueueRunStatus::Passed => "passed",
        MergeQueueRunStatus::Failed => "failed",
        MergeQueueRunStatus::Conflict => "conflict",
        MergeQueueRunStatus::Cancelled => "cancelled",
    }
}

pub(super) fn parse_merge_queue_run_status(value: &str) -> MergeQueueRunStatus {
    match value {
        "running" => MergeQueueRunStatus::Running,
        "passed" => MergeQueueRunStatus::Passed,
        "failed" => MergeQueueRunStatus::Failed,
        "conflict" => MergeQueueRunStatus::Conflict,
        "cancelled" => MergeQueueRunStatus::Cancelled,
        _ => MergeQueueRunStatus::Running,
    }
}

pub(super) fn merge_queue_patch_source_to_str(source: &MergeQueuePatchSource) -> &'static str {
    match source {
        MergeQueuePatchSource::Generated => "generated",
        MergeQueuePatchSource::Provided => "provided",
    }
}

pub(super) fn parse_merge_queue_patch_source(value: &str) -> MergeQueuePatchSource {
    match value {
        "provided" => MergeQueuePatchSource::Provided,
        "generated" => MergeQueuePatchSource::Generated,
        _ => MergeQueuePatchSource::Generated,
    }
}

pub(super) fn message_role_to_str(role: &MessageRole) -> &'static str {
    match role {
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::System => "system",
    }
}

pub(super) fn parse_message_role(value: &str) -> MessageRole {
    match value {
        "user" => MessageRole::User,
        "assistant" => MessageRole::Assistant,
        "system" => MessageRole::System,
        _ => MessageRole::User,
    }
}

pub(super) fn attachment_kind_to_str(kind: &WorkspaceAttachmentKind) -> &'static str {
    match kind {
        WorkspaceAttachmentKind::ReferenceRepo => "reference_repo",
        WorkspaceAttachmentKind::DocMirror => "doc_mirror",
    }
}

pub(super) fn parse_attachment_kind(value: &str) -> WorkspaceAttachmentKind {
    match value {
        "reference_repo" => WorkspaceAttachmentKind::ReferenceRepo,
        "doc_mirror" => WorkspaceAttachmentKind::DocMirror,
        _ => WorkspaceAttachmentKind::ReferenceRepo,
    }
}

pub(super) fn attachment_mode_to_str(mode: &AttachmentMode) -> &'static str {
    match mode {
        AttachmentMode::Ro => "ro",
        AttachmentMode::Rw => "rw",
    }
}

pub(super) fn parse_attachment_mode(value: &str) -> AttachmentMode {
    match value {
        "rw" => AttachmentMode::Rw,
        "ro" => AttachmentMode::Ro,
        _ => AttachmentMode::Ro,
    }
}

pub(super) fn attachment_update_policy_to_str(policy: &AttachmentUpdatePolicy) -> &'static str {
    match policy {
        AttachmentUpdatePolicy::Manual => "manual",
        AttachmentUpdatePolicy::OnOpen => "on_open",
        AttachmentUpdatePolicy::Scheduled => "scheduled",
    }
}

pub(super) fn parse_attachment_update_policy(value: &str) -> AttachmentUpdatePolicy {
    match value {
        "on_open" => AttachmentUpdatePolicy::OnOpen,
        "scheduled" => AttachmentUpdatePolicy::Scheduled,
        "manual" => AttachmentUpdatePolicy::Manual,
        _ => AttachmentUpdatePolicy::Manual,
    }
}

pub(super) fn workspace_attachment_status_to_str(
    status: &WorkspaceAttachmentStatus,
) -> &'static str {
    match status {
        WorkspaceAttachmentStatus::Pending => "pending",
        WorkspaceAttachmentStatus::Syncing => "syncing",
        WorkspaceAttachmentStatus::Ready => "ready",
        WorkspaceAttachmentStatus::Error => "error",
    }
}

pub(super) fn parse_workspace_attachment_status(value: &str) -> WorkspaceAttachmentStatus {
    match value {
        "pending" => WorkspaceAttachmentStatus::Pending,
        "syncing" => WorkspaceAttachmentStatus::Syncing,
        "ready" => WorkspaceAttachmentStatus::Ready,
        "error" => WorkspaceAttachmentStatus::Error,
        _ => WorkspaceAttachmentStatus::Ready,
    }
}

pub(super) fn worktree_attachment_status_to_str(status: &WorktreeAttachmentStatus) -> &'static str {
    match status {
        WorktreeAttachmentStatus::Ready => "ready",
        WorktreeAttachmentStatus::Stale => "stale",
        WorktreeAttachmentStatus::Error => "error",
    }
}

pub(super) fn parse_worktree_attachment_status(value: &str) -> WorktreeAttachmentStatus {
    match value {
        "ready" => WorktreeAttachmentStatus::Ready,
        "stale" => WorktreeAttachmentStatus::Stale,
        "error" => WorktreeAttachmentStatus::Error,
        _ => WorktreeAttachmentStatus::Error,
    }
}

pub(super) fn message_delivery_to_str(delivery: &MessageDelivery) -> &'static str {
    match delivery {
        MessageDelivery::Immediate => "immediate",
        MessageDelivery::Queued => "queued",
    }
}

pub(super) fn parse_message_delivery(value: &str) -> MessageDelivery {
    match value {
        "immediate" => MessageDelivery::Immediate,
        "queued" => MessageDelivery::Queued,
        _ => MessageDelivery::Queued,
    }
}

pub(super) fn session_turn_status_to_str(status: &SessionTurnStatus) -> &'static str {
    match status {
        SessionTurnStatus::Queued => "queued",
        SessionTurnStatus::Running => "running",
        SessionTurnStatus::Completed => "completed",
        SessionTurnStatus::Interrupted => "interrupted",
        SessionTurnStatus::Failed => "failed",
    }
}

pub(super) fn parse_session_turn_status(value: &str) -> SessionTurnStatus {
    match value {
        "queued" => SessionTurnStatus::Queued,
        "running" => SessionTurnStatus::Running,
        "completed" => SessionTurnStatus::Completed,
        "interrupted" => SessionTurnStatus::Interrupted,
        "failed" => SessionTurnStatus::Failed,
        _ => SessionTurnStatus::Running,
    }
}

pub(super) fn session_event_type_to_str(event_type: &SessionEventType) -> &'static str {
    match event_type {
        SessionEventType::Init => "init",
        SessionEventType::UserMessage => "user_message",
        SessionEventType::InputQueued => "input_queued",
        SessionEventType::TurnQueued => "turn_queued",
        SessionEventType::TurnStarted => "turn_started",
        SessionEventType::TurnFinished => "turn_finished",
        SessionEventType::AuthRequired => "auth_required",
        SessionEventType::Notice => "notice",
        SessionEventType::AssistantChunk => "assistant_chunk",
        SessionEventType::ThoughtChunk => "thought_chunk",
        SessionEventType::AssistantComplete => "assistant_complete",
        SessionEventType::AssistantMessageInserted => "assistant_message_inserted",
        SessionEventType::ToolCall => "tool_call",
        SessionEventType::ToolCallUpdate => "tool_call_update",
        SessionEventType::ToolResult => "tool_result",
        SessionEventType::Plan => "plan",
        SessionEventType::ArtifactsSet => "artifacts_set",
        SessionEventType::Done => "done",
        SessionEventType::InterruptRequested => "interrupt_requested",
        SessionEventType::TurnInterrupted => "turn_interrupted",
        SessionEventType::MessageQueueAdded => "message_queue_added",
        SessionEventType::MessageQueueUpdated => "message_queue_updated",
        SessionEventType::MessageQueueRemoved => "message_queue_removed",
        SessionEventType::MessageQueuePromoted => "message_queue_promoted",
        SessionEventType::Error => "error",
    }
}

pub(super) fn parse_session_event_type(value: &str) -> SessionEventType {
    match value {
        "init" => SessionEventType::Init,
        "user_message" => SessionEventType::UserMessage,
        "input_queued" => SessionEventType::InputQueued,
        "turn_queued" => SessionEventType::TurnQueued,
        "turn_started" => SessionEventType::TurnStarted,
        "turn_finished" => SessionEventType::TurnFinished,
        "auth_required" => SessionEventType::AuthRequired,
        "notice" => SessionEventType::Notice,
        "assistant_chunk" => SessionEventType::AssistantChunk,
        "thought_chunk" => SessionEventType::ThoughtChunk,
        "assistant_complete" => SessionEventType::AssistantComplete,
        "assistant_message_inserted" => SessionEventType::AssistantMessageInserted,
        "tool_call" => SessionEventType::ToolCall,
        "tool_call_update" => SessionEventType::ToolCallUpdate,
        "tool_result" => SessionEventType::ToolResult,
        "plan" => SessionEventType::Plan,
        "artifacts_set" => SessionEventType::ArtifactsSet,
        "done" => SessionEventType::Done,
        "interrupt_requested" => SessionEventType::InterruptRequested,
        "turn_interrupted" => SessionEventType::TurnInterrupted,
        "message_queue_added" => SessionEventType::MessageQueueAdded,
        "message_queue_updated" => SessionEventType::MessageQueueUpdated,
        "message_queue_removed" => SessionEventType::MessageQueueRemoved,
        "message_queue_promoted" => SessionEventType::MessageQueuePromoted,
        "error" => SessionEventType::Error,
        _ => SessionEventType::Error,
    }
}

pub(super) fn is_transient_session_event(
    event_type: &SessionEventType,
    payload_json: &serde_json::Value,
) -> bool {
    if payload_json
        .get("crp_channel")
        .and_then(|v| v.as_str())
        .is_some_and(|v| v == "data")
    {
        return true;
    }
    if payload_json
        .get("crpChannel")
        .and_then(|v| v.as_str())
        .is_some_and(|v| v == "data")
    {
        return true;
    }
    if matches!(event_type, SessionEventType::ToolCallUpdate) {
        return true;
    }
    if matches!(event_type, SessionEventType::AssistantComplete) {
        return true;
    }
    if matches!(event_type, SessionEventType::AuthRequired) {
        return true;
    }
    if !matches!(event_type, SessionEventType::Notice) {
        return false;
    }

    if let Some(kind) = payload_json.get("kind").and_then(|v| v.as_str()) {
        if matches!(
            kind,
            "reasoning_summary"
                | "provider_guard_warning"
                | "provider_guard_kill"
                | "title_generated"
                | "git_status_snapshot"
                | "auth_started"
                | "auth_finished"
                | "auth_failed"
                | "auth_required"
        ) {
            return true;
        }
    }

    false
}

pub(super) fn build_subagent_invocation_from_row(r: SqliteRow) -> Result<SubagentInvocation> {
    let id: String = r.try_get("id")?;
    let tool_call_id: String = r.try_get("tool_call_id")?;
    let parent_session_id: String = r.try_get("parent_session_id")?;
    let parent_turn_id: Option<String> = r.try_get("parent_turn_id")?;
    let created_at: String = r.try_get("created_at")?;
    let updated_at: String = r.try_get("updated_at")?;
    let request_json = r
        .try_get::<Option<String>, _>("request_json")?
        .and_then(|raw| serde_json::from_str(&raw).ok());

    Ok(SubagentInvocation {
        id,
        tool_call_id,
        parent_session_id: SessionId(uuid::Uuid::parse_str(&parent_session_id)?),
        parent_turn_id: parent_turn_id
            .and_then(|value| uuid::Uuid::parse_str(&value).ok())
            .map(TurnId),
        requested_count: r.try_get("requested_count")?,
        request_json,
        status: r.try_get("status")?,
        created_at: parse_dt(&created_at)?,
        updated_at: parse_dt(&updated_at)?,
        children: Vec::new(),
    })
}

pub(super) fn build_subagent_invocation_child_from_row(
    r: SqliteRow,
) -> Result<SubagentInvocationChild> {
    let invocation_id: String = r.try_get("invocation_id")?;
    let child_session_id: String = r.try_get("child_session_id")?;
    let run_id: Option<String> = r.try_get("run_id")?;
    let created_at: String = r.try_get("created_at")?;
    let updated_at: String = r.try_get("updated_at")?;

    Ok(SubagentInvocationChild {
        invocation_id,
        child_session_id: SessionId(uuid::Uuid::parse_str(&child_session_id)?),
        run_id: run_id
            .and_then(|value| uuid::Uuid::parse_str(&value).ok())
            .map(RunId),
        position: r.try_get("position")?,
        status: r.try_get("status")?,
        label: r.try_get("label")?,
        harness: r.try_get("harness")?,
        model: r.try_get("model")?,
        reasoning_effort: r.try_get("reasoning_effort")?,
        prompt_length: r.try_get("prompt_length")?,
        created_at: parse_dt(&created_at)?,
        updated_at: parse_dt(&updated_at)?,
    })
}

pub(super) fn build_artifact_from_row(r: SqliteRow) -> Result<Artifact> {
    let id: String = r.try_get("id")?;
    let session_id: String = r.try_get("session_id")?;
    let task_id: String = r.try_get("task_id")?;
    let workspace_id: String = r.try_get("workspace_id")?;
    let worktree_id: String = r.try_get("worktree_id")?;
    let created_at: String = r.try_get("created_at")?;

    Ok(Artifact {
        id: ArtifactId(uuid::Uuid::parse_str(&id)?),
        session_id: SessionId(uuid::Uuid::parse_str(&session_id)?),
        task_id: TaskId(uuid::Uuid::parse_str(&task_id)?),
        workspace_id: WorkspaceId(uuid::Uuid::parse_str(&workspace_id)?),
        worktree_id: WorktreeId(uuid::Uuid::parse_str(&worktree_id)?),
        name: r.try_get("name")?,
        absolute_path: r.try_get("absolute_path")?,
        mime_type: r.try_get("mime_type")?,
        bytes: r.try_get("bytes")?,
        created_at: parse_dt(&created_at)?,
        missing: None,
    })
}

pub(super) fn build_session_turn_from_row(r: SqliteRow) -> Result<SessionTurn> {
    let turn_id: String = r.try_get("turn_id")?;
    let session_id: String = r.try_get("session_id")?;
    let run_id: Option<String> = r.try_get("run_id")?;
    let user_message_id: Option<String> = r.try_get("user_message_id")?;
    let status: String = r.try_get("status")?;
    let started_at: String = r.try_get("started_at")?;
    let updated_at: String = r.try_get("updated_at")?;
    let metrics_json: Option<String> = r.try_get("metrics_json")?;
    let metrics_json = metrics_json
        .as_deref()
        .and_then(|s| serde_json::from_str::<Value>(s).ok());

    Ok(SessionTurn {
        turn_id: TurnId(uuid::Uuid::parse_str(&turn_id)?),
        session_id: SessionId(uuid::Uuid::parse_str(&session_id)?),
        run_id: run_id
            .as_deref()
            .and_then(|s| uuid::Uuid::parse_str(s).ok())
            .map(RunId),
        user_message_id: user_message_id
            .as_deref()
            .and_then(|s| uuid::Uuid::parse_str(s).ok())
            .map(MessageId),
        status: parse_session_turn_status(status.as_str()),
        start_seq: r.try_get("start_seq")?,
        end_seq: r.try_get("end_seq")?,
        started_at: parse_dt(&started_at)?,
        updated_at: parse_dt(&updated_at)?,
        assistant_partial: r.try_get("assistant_partial")?,
        thought_partial: r.try_get("thought_partial")?,
        metrics_json,
        tool_total: r.try_get("tool_total")?,
        tool_pending: r.try_get("tool_pending")?,
        tool_running: r.try_get("tool_running")?,
        tool_completed: r.try_get("tool_completed")?,
        tool_failed: r.try_get("tool_failed")?,
    })
}

pub(super) fn build_session_turn_tool_from_row(r: SqliteRow) -> Result<SessionTurnTool> {
    let session_id: String = r.try_get("session_id")?;
    let tool_call_id: String = r.try_get("tool_call_id")?;
    let turn_id: String = r.try_get("turn_id")?;
    let created_at: String = r.try_get("created_at")?;
    let updated_at: String = r.try_get("updated_at")?;
    let input_json: Option<String> = r.try_get("input_json")?;
    let input_json = input_json
        .as_deref()
        .and_then(|s| serde_json::from_str::<Value>(s).ok());
    let first_event_seq: Option<i64> = r.try_get("first_event_seq")?;
    let input_truncated: Option<i64> = r.try_get("input_truncated")?;
    let input_original_bytes: Option<i64> = r.try_get("input_original_bytes")?;
    let output_truncated: Option<i64> = r.try_get("output_truncated")?;
    let output_original_bytes: Option<i64> = r.try_get("output_original_bytes")?;

    Ok(SessionTurnTool {
        session_id: SessionId(uuid::Uuid::parse_str(&session_id)?),
        tool_call_id,
        turn_id: TurnId(uuid::Uuid::parse_str(&turn_id)?),
        tool_kind: r.try_get("tool_kind")?,
        title: r.try_get("title")?,
        status: r.try_get("status")?,
        input_json,
        output_text: r.try_get("output_text")?,
        first_event_seq,
        input_truncated: input_truncated.map(|value| value != 0),
        input_original_bytes,
        output_truncated: output_truncated.map(|value| value != 0),
        output_original_bytes,
        created_at: parse_dt(&created_at)?,
        updated_at: parse_dt(&updated_at)?,
    })
}

pub(super) fn build_session_turn_tool_summary_from_row(
    r: SqliteRow,
) -> Result<SessionTurnToolSummary> {
    let session_id: String = r.try_get("session_id")?;
    let tool_call_id: String = r.try_get("tool_call_id")?;
    let turn_id: String = r.try_get("turn_id")?;
    let created_at: String = r.try_get("created_at")?;
    let updated_at: String = r.try_get("updated_at")?;
    let input_json: Option<String> = r.try_get("input_json")?;
    let input_json = input_json
        .as_deref()
        .and_then(|s| serde_json::from_str::<Value>(s).ok());
    let output_text: Option<String> = r.try_get("output_text")?;
    let first_event_seq: Option<i64> = r.try_get("first_event_seq")?;
    let input_truncated: Option<i64> = r.try_get("input_truncated")?;
    let input_original_bytes: Option<i64> = r.try_get("input_original_bytes")?;
    let output_truncated: Option<i64> = r.try_get("output_truncated")?;
    let output_original_bytes: Option<i64> = r.try_get("output_original_bytes")?;
    let input_preview = tool_input_preview_from_value(input_json.as_ref());

    Ok(SessionTurnToolSummary {
        session_id: SessionId(uuid::Uuid::parse_str(&session_id)?),
        tool_call_id,
        turn_id: TurnId(uuid::Uuid::parse_str(&turn_id)?),
        tool_kind: r.try_get("tool_kind")?,
        title: r.try_get("title")?,
        status: r.try_get("status")?,
        input_preview,
        output_preview: output_text,
        first_event_seq,
        input_truncated: input_truncated.map(|value| value != 0),
        input_original_bytes,
        output_truncated: output_truncated.map(|value| value != 0),
        output_original_bytes,
        created_at: parse_dt(&created_at)?,
        updated_at: parse_dt(&updated_at)?,
    })
}

pub(super) fn summarize_session_turn_tool(tool: &SessionTurnTool) -> SessionTurnToolSummary {
    SessionTurnToolSummary {
        session_id: tool.session_id,
        tool_call_id: tool.tool_call_id.clone(),
        turn_id: tool.turn_id,
        tool_kind: tool.tool_kind.clone(),
        title: tool.title.clone(),
        status: tool.status.clone(),
        input_preview: tool_input_preview_from_value(tool.input_json.as_ref()),
        output_preview: tool.output_text.clone(),
        first_event_seq: tool.first_event_seq,
        input_truncated: tool.input_truncated,
        input_original_bytes: tool.input_original_bytes,
        output_truncated: tool.output_truncated,
        output_original_bytes: tool.output_original_bytes,
        created_at: tool.created_at,
        updated_at: tool.updated_at,
    }
}

pub(super) fn tool_seq_sort_key(seq: Option<i64>) -> i64 {
    seq.unwrap_or(i64::MAX)
}

pub(super) fn compare_tool_summary_order(
    a: &SessionTurnToolSummary,
    b: &SessionTurnToolSummary,
) -> std::cmp::Ordering {
    tool_seq_sort_key(a.first_event_seq)
        .cmp(&tool_seq_sort_key(b.first_event_seq))
        .then_with(|| a.created_at.cmp(&b.created_at))
        .then_with(|| a.tool_call_id.cmp(&b.tool_call_id))
}

pub(super) fn compare_tool_order(a: &SessionTurnTool, b: &SessionTurnTool) -> std::cmp::Ordering {
    tool_seq_sort_key(a.first_event_seq)
        .cmp(&tool_seq_sort_key(b.first_event_seq))
        .then_with(|| a.created_at.cmp(&b.created_at))
        .then_with(|| a.tool_call_id.cmp(&b.tool_call_id))
}

pub(super) const TOOL_PREVIEW_MAX_LINES: usize = 5;
pub(super) const TOOL_PREVIEW_MAX_LINE_CHARS: usize = 80;

pub(super) struct ToolTextPreview {
    preview: String,
    truncated: bool,
    original_bytes: usize,
}

pub(super) struct ToolJsonPreview {
    preview: Option<Value>,
    truncated: Option<bool>,
    original_bytes: Option<i64>,
}

pub(super) fn tool_input_preview_from_value(input: Option<&Value>) -> Option<Value> {
    let input = input?;
    let obj = input.as_object()?;
    let mut out = serde_json::Map::new();
    for key in [
        "command",
        "query",
        "pattern",
        "text",
        "path",
        "file",
        "filename",
        "file_path",
        "filePath",
        "filepath",
        "paths",
        "paths_total",
        "files",
        "file_paths",
        "filePaths",
        "target",
        "glob",
        "parsed_cmd",
        "cwd",
        "root",
        "url",
        "uri",
        "href",
        "method",
        "regex",
        "diff_stats",
    ] {
        if let Some(value) = obj.get(key) {
            if value.is_string() || value.is_number() || value.is_array() || value.is_object() {
                out.insert(key.to_string(), value.clone());
            }
        }
    }
    if out.is_empty() {
        None
    } else {
        let value = Value::Object(out);
        let mut truncated = false;
        Some(truncate_preview_value(&value, &mut truncated))
    }
}

pub(super) fn truncate_preview_value(value: &Value, truncated: &mut bool) -> Value {
    match value {
        Value::String(value) => {
            let preview = build_text_preview(value);
            if preview.truncated {
                *truncated = true;
            }
            Value::String(preview.preview)
        }
        Value::Array(values) => Value::Array(
            values
                .iter()
                .map(|value| truncate_preview_value(value, truncated))
                .collect(),
        ),
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (key, value) in map {
                out.insert(key.clone(), truncate_preview_value(value, truncated));
            }
            Value::Object(out)
        }
        _ => value.clone(),
    }
}

pub(super) fn push_text_preview_line(out: &mut Vec<String>, truncated: &mut bool, line: &str) {
    if line.chars().count() > TOOL_PREVIEW_MAX_LINE_CHARS {
        *truncated = true;
        out.push(line.chars().take(TOOL_PREVIEW_MAX_LINE_CHARS).collect());
    } else {
        out.push(line.to_string());
    }
}

pub(super) fn build_text_preview(text: &str) -> ToolTextPreview {
    let original_bytes = text.len();
    let mut truncated = false;
    let lines: Vec<&str> = text.lines().collect();
    let total_lines = lines.len();

    let mut out = Vec::new();

    if total_lines <= TOOL_PREVIEW_MAX_LINES {
        for line in lines {
            push_text_preview_line(&mut out, &mut truncated, line);
        }
    } else {
        truncated = true;
        let head_count = TOOL_PREVIEW_MAX_LINES / 2;
        let tail_count = TOOL_PREVIEW_MAX_LINES.saturating_sub(head_count + 1);
        for line in lines.iter().take(head_count) {
            push_text_preview_line(&mut out, &mut truncated, line);
        }
        let omitted = total_lines.saturating_sub(head_count + tail_count);
        out.push(format!("... +{omitted} lines"));
        for line in lines.iter().skip(total_lines - tail_count) {
            push_text_preview_line(&mut out, &mut truncated, line);
        }
    }

    ToolTextPreview {
        preview: out.join("\n"),
        truncated,
        original_bytes,
    }
}

pub(super) fn build_json_preview(input: Option<&Value>, preview: Option<Value>) -> ToolJsonPreview {
    let original_bytes = input
        .and_then(|value| serde_json::to_string(value).ok())
        .map(|value| value.len() as i64);
    let mut preview_truncated = false;
    let preview = preview.map(|value| truncate_preview_value(&value, &mut preview_truncated));
    let preview_bytes = preview
        .as_ref()
        .and_then(|value| serde_json::to_string(value).ok())
        .map(|value| value.len() as i64);
    let mut truncated = preview_truncated;
    if let (Some(original), Some(preview_bytes)) = (original_bytes, preview_bytes) {
        if original > preview_bytes {
            truncated = true;
        }
    } else if original_bytes.is_some() && preview.is_none() {
        truncated = true;
    }
    let truncated = if original_bytes.is_some() || preview.is_some() {
        Some(truncated)
    } else {
        None
    };
    ToolJsonPreview {
        preview,
        truncated,
        original_bytes,
    }
}

pub(super) fn build_output_preview(text: &str) -> ToolTextPreview {
    build_text_preview(text)
}

pub(super) fn normalize_tool_status(status: &str, event_type: SessionEventType) -> String {
    let s = status.trim().to_lowercase();
    if s == "inprogress" || s == "in_progress" || s == "running" {
        return "in_progress".to_string();
    }
    if s == "pending" || s == "queued" {
        return "pending".to_string();
    }
    if s == "completed" || s == "complete" || s == "ok" || s == "succeeded" {
        return "completed".to_string();
    }
    if s == "failed" || s == "error" {
        return "failed".to_string();
    }
    if matches!(event_type, SessionEventType::ToolResult) {
        return "completed".to_string();
    }
    if s.is_empty() {
        return "pending".to_string();
    }
    s
}

pub(super) fn merge_tool_status(existing: Option<&str>, next: &str) -> String {
    match (existing, next) {
        (Some(current @ ("completed" | "failed")), "pending" | "in_progress") => current.to_string(),
        _ => next.to_string(),
    }
}

pub(super) fn extract_tool_update(payload: &Value) -> &Value { payload }

fn tool_kind_from_update(update: &Value) -> Option<String> {
    update
        .get("kind")
        .and_then(Value::as_str)
        .or_else(|| update.pointer("/toolCall/kind").and_then(Value::as_str))
        .map(str::to_owned)
}

fn tool_title_from_update(update: &Value) -> Option<String> {
    update
        .get("title")
        .and_then(Value::as_str)
        .or_else(|| update.get("tool_label").and_then(Value::as_str))
        .or_else(|| update.pointer("/toolCall/title").and_then(Value::as_str))
        .or_else(|| update.pointer("/toolCall/tool_label").and_then(Value::as_str))
        .map(str::to_owned)
}

fn tool_status_from_update<'a>(update: &'a Value) -> Option<&'a str> {
    update
        .get("status")
        .and_then(Value::as_str)
        .or_else(|| update.pointer("/toolCall/status").and_then(Value::as_str))
}

pub(super) fn sanitize_tool_event_payload(
    event_type: &SessionEventType,
    raw_payload: &Value,
) -> Value {
    let update = extract_tool_update(raw_payload);
    let tool_call_id = tool_call_id_from_payload(raw_payload).unwrap_or_default();

    let tool_kind = tool_kind_from_update(update);
    let title = tool_title_from_update(update);
    let raw_status = tool_status_from_update(update);
    let status = if let Some(raw_status) = raw_status {
        normalize_tool_status(raw_status, event_type.clone())
    } else if matches!(event_type, SessionEventType::ToolResult) {
        "completed".to_string()
    } else {
        "pending".to_string()
    };

    let input = update
        .pointer("/rawInput")
        .or_else(|| update.pointer("/toolCall/rawInput"))
        .or_else(|| update.pointer("/toolCall/input"))
        .or_else(|| update.pointer("/input"))
        .or_else(|| update.pointer("/args"));
    let input_preview = tool_input_preview_from_value(input);
    let input_preview = input_preview.or_else(|| update.get("input_preview").cloned());
    let input_meta = build_json_preview(input, input_preview);

    let output_meta = extract_tool_output_text(update)
        .map(|output| build_output_preview(&output))
        .filter(|preview| !preview.preview.trim().is_empty());
    let output_spool_path = update
        .get("output_spool_path")
        .and_then(|v| v.as_str())
        .or_else(|| {
            raw_payload
                .get("output_spool_path")
                .and_then(|v| v.as_str())
        })
        .map(|v| v.to_string());

    let mut obj = serde_json::Map::new();
    if !tool_call_id.trim().is_empty() {
        obj.insert("tool_call_id".to_string(), Value::String(tool_call_id));
    }
    if let Some(v) = tool_kind {
        obj.insert("kind".to_string(), Value::String(v));
    }
    if let Some(v) = title {
        obj.insert("title".to_string(), Value::String(v));
    }
    obj.insert("status".to_string(), Value::String(status));

    if let Some(v) = input_meta.preview {
        obj.insert("input_preview".to_string(), v);
    }
    let input_truncated = update
        .get("input_truncated")
        .and_then(|v| v.as_bool())
        .or(input_meta.truncated);
    let input_original_bytes = update
        .get("input_original_bytes")
        .and_then(|v| v.as_i64())
        .or(input_meta.original_bytes);
    if let Some(truncated) = input_truncated {
        obj.insert("input_truncated".to_string(), Value::Bool(truncated));
    }
    if let Some(bytes) = input_original_bytes {
        obj.insert(
            "input_original_bytes".to_string(),
            Value::Number(serde_json::Number::from(bytes)),
        );
    }

    if let Some(output_meta) = output_meta {
        obj.insert(
            "output_preview".to_string(),
            Value::String(output_meta.preview),
        );
        let output_truncated = update
            .get("output_truncated")
            .and_then(|v| v.as_bool())
            .unwrap_or(output_meta.truncated);
        let output_original_bytes = update
            .get("output_original_bytes")
            .and_then(|v| v.as_i64())
            .unwrap_or(output_meta.original_bytes as i64);
        obj.insert(
            "output_truncated".to_string(),
            Value::Bool(output_truncated),
        );
        obj.insert(
            "output_original_bytes".to_string(),
            Value::Number(serde_json::Number::from(output_original_bytes)),
        );
    }
    if let Some(path) = output_spool_path {
        obj.insert("output_spool_path".to_string(), Value::String(path));
    }

    if let Some(value) = raw_payload
        .get("crp_seq")
        .or_else(|| raw_payload.get("crpSeq"))
        .or_else(|| update.get("crp_seq"))
        .or_else(|| update.get("crpSeq"))
    {
        obj.insert("crp_seq".to_string(), value.clone());
    }
    if let Some(value) = raw_payload
        .get("order_seq")
        .or_else(|| raw_payload.get("orderSeq"))
        .or_else(|| update.get("order_seq"))
        .or_else(|| update.get("orderSeq"))
    {
        obj.insert("order_seq".to_string(), value.clone());
    }
    if let Some(value) = raw_payload
        .get("crp_channel")
        .or_else(|| raw_payload.get("crpChannel"))
        .or_else(|| update.get("crp_channel"))
        .or_else(|| update.get("crpChannel"))
    {
        obj.insert("crp_channel".to_string(), value.clone());
    }

    Value::Object(obj)
}

pub(super) fn build_turn_tool_from_event(
    event: &SessionEvent,
    turn_id: TurnId,
) -> Option<SessionTurnTool> {
    if !matches!(
        event.event_type,
        SessionEventType::ToolCall
            | SessionEventType::ToolCallUpdate
            | SessionEventType::ToolResult
    ) {
        return None;
    }
    let tool_call_id = tool_call_id_from_payload(&event.payload_json)?;
    let update = extract_tool_update(&event.payload_json);

    let tool_kind = tool_kind_from_update(update);
    let title = tool_title_from_update(update);
    let raw_status = tool_status_from_update(update);
    let status = if let Some(raw_status) = raw_status {
        Some(normalize_tool_status(raw_status, event.event_type.clone()))
    } else if matches!(event.event_type, SessionEventType::ToolResult) {
        Some("completed".to_string())
    } else if matches!(event.event_type, SessionEventType::ToolCall) {
        Some("pending".to_string())
    } else {
        None
    };

    let input = update
        .pointer("/rawInput")
        .or_else(|| update.pointer("/toolCall/rawInput"))
        .or_else(|| update.pointer("/toolCall/input"))
        .or_else(|| update.pointer("/input"))
        .or_else(|| update.pointer("/args"));
    let input_preview = tool_input_preview_from_value(input);
    let input_preview = input_preview.or_else(|| update.get("input_preview").cloned());
    let input_meta = build_json_preview(input, input_preview);
    let input_truncated = update
        .get("input_truncated")
        .and_then(|v| v.as_bool())
        .or(input_meta.truncated);
    let input_original_bytes = update
        .get("input_original_bytes")
        .and_then(|v| v.as_i64())
        .or(input_meta.original_bytes);

    let output_meta = extract_tool_output_text(update)
        .map(|output| build_output_preview(&output))
        .filter(|preview| !preview.preview.trim().is_empty());
    let output_truncated = update
        .get("output_truncated")
        .and_then(|v| v.as_bool())
        .or(output_meta.as_ref().map(|preview| preview.truncated));
    let output_original_bytes = update
        .get("output_original_bytes")
        .and_then(|v| v.as_i64())
        .or(output_meta
            .as_ref()
            .map(|preview| preview.original_bytes as i64));

    Some(SessionTurnTool {
        session_id: event.session_id,
        tool_call_id,
        turn_id,
        tool_kind,
        title,
        status,
        input_json: input_meta.preview,
        output_text: output_meta.as_ref().map(|preview| preview.preview.clone()),
        first_event_seq: Some(event.seq),
        input_truncated,
        input_original_bytes,
        output_truncated,
        output_original_bytes,
        created_at: event.created_at,
        updated_at: event.created_at,
    })
}

pub(super) fn tool_call_id_from_payload(payload: &Value) -> Option<String> {
    let direct = payload.get("tool_call_id").and_then(|v| v.as_str());
    if let Some(v) = direct {
        return Some(v.to_string());
    }
    let update = extract_tool_update(payload);
    let direct = update
        .get("toolCallId")
        .and_then(|v| v.as_str())
        .or_else(|| update.get("tool_call_id").and_then(|v| v.as_str()));
    if let Some(v) = direct {
        return Some(v.to_string());
    }
    let from_raw = update
        .pointer("/rawInput/call_id")
        .and_then(|v| v.as_str())
        .or_else(|| {
            update
                .pointer("/raw_input/call_id")
                .and_then(|v| v.as_str())
        });
    from_raw.map(|v| v.to_string())
}

pub(super) fn extract_tool_output_text(update: &Value) -> Option<String> {
    let direct = update
        .get("outputText")
        .and_then(|v| v.as_str())
        .or_else(|| update.get("output_text").and_then(|v| v.as_str()))
        .or_else(|| update.get("output_preview").and_then(|v| v.as_str()))
        .or_else(|| {
            update
                .pointer("/toolCall/outputText")
                .and_then(|v| v.as_str())
        })
        .or_else(|| {
            update
                .pointer("/toolCall/output_text")
                .and_then(|v| v.as_str())
        })
        .or_else(|| update.get("result").and_then(|v| v.as_str()))
        .or_else(|| {
            update
                .pointer("/rawOutput/aggregated_output")
                .and_then(|v| v.as_str())
        })
        .or_else(|| update.pointer("/rawOutput/output").and_then(|v| v.as_str()));
    if let Some(v) = direct {
        let trimmed = v.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    let blocks = update.get("content").and_then(|v| v.as_array())?;
    let mut out = String::new();
    for b in blocks {
        if let Some(t) = b
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|v| v.as_str())
        {
            out.push_str(t);
        } else if let Some(t) = b.get("text").and_then(|v| v.as_str()) {
            out.push_str(t);
        }
    }
    if out.trim().is_empty() {
        None
    } else {
        Some(out.trim().to_string())
    }
}

pub(super) fn merge_streaming_text(prev: Option<&str>, next: &str) -> String {
    let prev = prev.unwrap_or("");
    if prev.is_empty() {
        return next.to_string();
    }
    if next.is_empty() {
        return prev.to_string();
    }
    if next.starts_with(prev) {
        return next.to_string();
    }
    if prev.starts_with(next) {
        return prev.to_string();
    }
    if next.len() >= prev.len() {
        next.to_string()
    } else {
        prev.to_string()
    }
}

pub(super) fn build_turn_tools_from_events(
    session_id: SessionId,
    turn_id: TurnId,
    events: &[SessionEvent],
) -> Vec<SessionTurnTool> {
    #[derive(Default)]
    struct ToolAgg {
        tool_kind: Option<String>,
        title: Option<String>,
        status: Option<String>,
        input_json: Option<Value>,
        output_text: Option<String>,
        input_truncated: Option<bool>,
        input_original_bytes: Option<i64>,
        output_truncated: Option<bool>,
        output_original_bytes: Option<i64>,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
        first_event_seq: Option<i64>,
        initialized: bool,
    }

    let mut map: HashMap<String, ToolAgg> = HashMap::new();

    for ev in events {
        if !matches!(
            ev.event_type,
            SessionEventType::ToolCall
                | SessionEventType::ToolCallUpdate
                | SessionEventType::ToolResult
        ) {
            continue;
        }
        let tool_call_id = match tool_call_id_from_payload(&ev.payload_json) {
            Some(v) => v,
            None => continue,
        };
        let update = extract_tool_update(&ev.payload_json);
        let entry = map.entry(tool_call_id.clone()).or_default();
        if !entry.initialized {
            entry.created_at = ev.created_at;
            entry.updated_at = ev.created_at;
            entry.first_event_seq = Some(ev.seq);
            entry.initialized = true;
        }
        entry.updated_at = ev.created_at;
        entry.first_event_seq = Some(match entry.first_event_seq {
            Some(prev) => prev.min(ev.seq),
            None => ev.seq,
        });

        if let Some(kind) = tool_kind_from_update(update) {
            entry.tool_kind = Some(kind.to_string());
        }
        if let Some(title) = tool_title_from_update(update) {
            entry.title = Some(title);
        }
        let raw_status = tool_status_from_update(update);
        if let Some(raw_status) = raw_status {
            let next_status = normalize_tool_status(raw_status, ev.event_type.clone());
            entry.status = Some(merge_tool_status(entry.status.as_deref(), &next_status));
        } else if matches!(ev.event_type, SessionEventType::ToolResult) {
            entry.status = Some(merge_tool_status(entry.status.as_deref(), "completed"));
        } else if matches!(ev.event_type, SessionEventType::ToolCall) {
            entry.status = Some(merge_tool_status(entry.status.as_deref(), "pending"));
        }

        let input = update
            .pointer("/rawInput")
            .or_else(|| update.pointer("/toolCall/rawInput"))
            .or_else(|| update.pointer("/toolCall/input"))
            .or_else(|| update.pointer("/input"))
            .or_else(|| update.pointer("/args"));
        if let Some(value) = input {
            let input_preview = tool_input_preview_from_value(Some(value));
            let input_meta = build_json_preview(Some(value), input_preview);
            entry.input_json = input_meta.preview.or_else(|| entry.input_json.clone());
            let input_truncated = update
                .get("input_truncated")
                .and_then(|v| v.as_bool())
                .or(input_meta.truncated);
            let input_original_bytes = update
                .get("input_original_bytes")
                .and_then(|v| v.as_i64())
                .or(input_meta.original_bytes);
            if let Some(next) = input_truncated {
                entry.input_truncated = Some(entry.input_truncated.unwrap_or(false) || next);
            }
            if let Some(next) = input_original_bytes {
                entry.input_original_bytes =
                    Some(entry.input_original_bytes.unwrap_or(0).max(next));
            }
        } else if let Some(preview) = update.get("input_preview") {
            let input_meta = build_json_preview(None, Some(preview.clone()));
            entry.input_json = input_meta.preview.or_else(|| entry.input_json.clone());
            let input_truncated = update
                .get("input_truncated")
                .and_then(|v| v.as_bool())
                .or(input_meta.truncated);
            let input_original_bytes = update
                .get("input_original_bytes")
                .and_then(|v| v.as_i64())
                .or(input_meta.original_bytes);
            if let Some(next) = input_truncated {
                entry.input_truncated = Some(entry.input_truncated.unwrap_or(false) || next);
            }
            if let Some(next) = input_original_bytes {
                entry.input_original_bytes =
                    Some(entry.input_original_bytes.unwrap_or(0).max(next));
            }
        }

        if let Some(output) = extract_tool_output_text(update) {
            let preview = build_output_preview(&output);
            if !preview.preview.trim().is_empty() {
                let merged = merge_streaming_text(entry.output_text.as_deref(), &preview.preview);
                entry.output_text = Some(merged);
                let output_truncated = update
                    .get("output_truncated")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(preview.truncated);
                let output_original_bytes = update
                    .get("output_original_bytes")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(preview.original_bytes as i64);
                entry.output_truncated =
                    Some(entry.output_truncated.unwrap_or(false) || output_truncated);
                entry.output_original_bytes = Some(
                    entry
                        .output_original_bytes
                        .unwrap_or(0)
                        .max(output_original_bytes),
                );
            }
        }
    }

    let mut out: Vec<SessionTurnTool> = map
        .into_iter()
        .map(|(tool_call_id, agg)| SessionTurnTool {
            session_id,
            tool_call_id,
            turn_id,
            tool_kind: agg.tool_kind,
            title: agg.title,
            status: agg.status,
            input_json: agg.input_json,
            output_text: agg.output_text,
            input_truncated: agg.input_truncated,
            input_original_bytes: agg.input_original_bytes,
            output_truncated: agg.output_truncated,
            output_original_bytes: agg.output_original_bytes,
            created_at: agg.created_at,
            updated_at: agg.updated_at,
            first_event_seq: agg.first_event_seq,
        })
        .collect();

    out.sort_by(compare_tool_order);
    out
}
