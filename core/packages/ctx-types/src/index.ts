export type Workspace = {
  id: string;
  name: string;
  root_path: string;
  created_at: string;
  vcs_kind?: VcsKind | null;
};

export type VcsKind = "git" | "jj" | "hg" | "svn" | "p4" | "other";

export type WorkspaceAttachmentKind = "reference_repo" | "doc_mirror";

export type AttachmentMode = "ro" | "rw";

export type AttachmentUpdatePolicy = "manual" | "on_open" | "scheduled";

export type WorkspaceAttachmentStatus = "pending" | "syncing" | "ready" | "error";

export type WorkspaceAttachment = {
  id: string;
  workspace_id: string;
  kind: WorkspaceAttachmentKind;
  name: string;
  source: string;
  revision?: string | null;
  subpath?: string | null;
  mount_relpath: string;
  mode: AttachmentMode;
  update_policy: AttachmentUpdatePolicy;
  status: WorkspaceAttachmentStatus;
  last_sync_at?: string | null;
  error_message?: string | null;
  created_at: string;
  updated_at: string;
};

export type Task = {
  id: string;
  workspace_id: string;
  title: string;
  description?: string | null;
  status: string;
  primary_session_id?: string | null;
  primary_worktree_id?: string | null;
  created_at: string;
  updated_at: string;
  archived_at?: string | null;
  assistant_seen_at?: string | null;
  last_activity_at?: string | null;
  last_assistant_message_at?: string | null;
  has_active_session?: boolean;
};

export type Worktree = {
  id: string;
  workspace_id: string;
  root_path: string;
  base_commit_sha: string;
  git_branch?: string | null;
  vcs_kind?: VcsKind | null;
  base_revision?: string | null;
  vcs_ref?: string | null;
  created_at: string;
  bootstrap_status?: "success" | "failed" | "timeout";
  bootstrap_started_at?: string | null;
  bootstrap_finished_at?: string | null;
  bootstrap_exit_code?: number | null;
  bootstrap_timeout_sec?: number | null;
  bootstrap_error?: string | null;
  bootstrap_log_path?: string | null;
  bootstrap_log_truncated?: boolean | null;
  bootstrap_config_path?: string | null;
  bootstrap_config_key?: string | null;
  bootstrap_command?: string | null;
  bootstrap_script_path?: string | null;
};

export type MergeQueueEntryStatus =
  | "queued"
  | "running"
  | "passed"
  | "failed"
  | "conflict"
  | "cancelled";

export type MergeQueuePatchSource = "generated" | "provided";

export type MergeQueueEntry = {
  id: string;
  workspace_id: string;
  worktree_id?: string | null;
  session_id?: string | null;
  target_branch: string;
  message?: string | null;
  patch_source: MergeQueuePatchSource;
  base_commit_sha?: string | null;
  head_commit_sha?: string | null;
  patch_path: string;
  patch_size: number;
  status: MergeQueueEntryStatus;
  result_commit_sha?: string | null;
  error_message?: string | null;
  created_at: string;
  updated_at: string;
};

export type MergeQueueRunStatus =
  | "running"
  | "passed"
  | "failed"
  | "conflict"
  | "cancelled";

export type MergeQueueRun = {
  id: string;
  entry_id: string;
  status: MergeQueueRunStatus;
  started_at: string;
  finished_at?: string | null;
  exit_code?: number | null;
  log_path?: string | null;
  error_message?: string | null;
  result_commit_sha?: string | null;
};

export type ExecutionEnvironment = "host" | "container_host_mounted" | "container_disk_isolated";

export type Session = {
  id: string;
  task_id: string;
  workspace_id: string;
  worktree_id: string;
  parent_session_id?: string | null;
  relationship?: string | null;
  provider_id: string;
  model_id: string;
  reasoning_effort?: string | null;
  title: string;
  agent_role: string;
  status: string;
  provider_session_ref?: string | null;
  execution_environment?: ExecutionEnvironment | null;
  created_at?: string;
  updated_at?: string;
};

export type SessionMetadata = Session;

export type TerminalStatus = "running" | "exited";

export type TerminalSession = {
  id: string;
  workspace_id: string;
  task_id?: string | null;
  session_id?: string | null;
  worktree_id?: string | null;
  cwd: string;
  shell: string;
  title: string;
  status: TerminalStatus;
  exit_code?: number | null;
  created_at: string;
  updated_at: string;
};

export type SessionSummary = {
  id: string;
  task_id: string;
  workspace_id: string;
  parent_session_id?: string | null;
  relationship?: string | null;
  provider_id: string;
  model_id: string;
  reasoning_effort?: string | null;
  title: string;
  status: string;
  execution_environment?: ExecutionEnvironment | null;
  created_at: string;
  updated_at: string;
};

export type SubagentInvocationChild = {
  invocation_id: string;
  child_session_id: string;
  run_id?: string | null;
  position: number;
  status: string;
  label?: string | null;
  harness?: string | null;
  model?: string | null;
  reasoning_effort?: string | null;
  prompt_length: number;
  created_at: string;
  updated_at: string;
};

export type SubagentInvocation = {
  id: string;
  tool_call_id: string;
  parent_session_id: string;
  parent_turn_id?: string | null;
  requested_count: number;
  request_json?: Record<string, unknown> | null;
  status: string;
  created_at: string;
  updated_at: string;
  children: SubagentInvocationChild[];
};

export type WorkspaceTaskSummary = {
  task: Task;
  provider_ids?: string[];
  sessions?: SessionSummary[];
  sort_at: string;
};

export type WorkspaceIndexCursor = {
  sort_at: string;
  task_id: string;
};

export type WorkspaceIndexPage = {
  workspace_id: string;
  snapshot_rev: number;
  tasks: WorkspaceTaskSummary[];
  next_cursor?: WorkspaceIndexCursor | null;
  total_active: number;
  total_archived: number;
};

export type WorkspaceArchivedPage = {
  workspace_id: string;
  archived_rev?: number;
  tasks: WorkspaceTaskSummary[];
  next_cursor?: WorkspaceIndexCursor | null;
  total_archived: number;
};

export type WorkspaceIndexEvent =
  | {
      type: "ready";
      workspace_id: string;
      snapshot_rev: number;
      archived_rev: number;
    }
  | {
      type: "task_upsert";
      workspace_id: string;
      snapshot_rev: number;
      task: WorkspaceTaskSummary;
    }
  | {
      type: "task_delete";
      workspace_id: string;
      snapshot_rev: number;
      task_id: string;
    };

export type SessionSnapshotSummary = {
  session: SessionMetadata;
  last_message_at?: string | null;
  last_message_preview?: string | null;
  last_event_seq?: number | null;
  state_rev?: number;
  activity?: SessionActivityState;
  unread?: boolean;
};

export type SessionHeadSnapshot = {
  session: SessionMetadata;
  turns: SessionTurn[];
  tool_summaries?: SessionTurnToolSummary[];
  events?: SessionEvent[];
  messages: Message[];
  last_event_seq: number;
  state_rev?: number;
  activity?: SessionActivityState;
  has_more_turns: boolean;
  history_cursor?: number | null;
  has_more_history: boolean;
  summary_checkpoint?: SessionSummaryCheckpoint | null;
  head_window?: SessionHeadWindow;
};

export type SessionSnapshot = {
  summary: SessionSnapshotSummary;
  head?: SessionHeadSnapshot | null;
  state?: SessionState | null;
};

export type SessionGitStatusSummary = {
  summary_line: string;
  branch?: string | null;
  upstream?: string | null;
  ahead: number;
  behind: number;
  detached: boolean;
  staged: number;
  unstaged: number;
  untracked: number;
};

export type SessionState = {
  artifacts: Artifact[];
  git_status?: SessionGitStatusSummary | null;
};

export type WorktreeVcsComputeState = "computing" | "ready" | "error";
export type DiffUnavailableReason = "no_repo" | "no_target_branch";

export type WorktreeVcsBaseResolutionKind = "explicit_base" | "merge_base" | "worktree_base";

export type WorktreeVcsTargetSource = "explicit" | "primary_branch_config";

export type WorktreeVcsBaseResolution = {
  kind: WorktreeVcsBaseResolutionKind;
  target_source?: WorktreeVcsTargetSource | null;
  error?: string | null;
};

export type WorktreeVcsSummary = {
  file_count?: number | null;
  line_additions?: number | null;
  line_deletions?: number | null;
  line_count?: number | null;
};

export type WorktreeVcsTouchedFile = {
  path: string;
  orig_path?: string | null;
  index_status?: string | null;
  worktree_status?: string | null;
};

export type WorktreeVcsTouchedFiles = {
  items?: WorktreeVcsTouchedFile[];
  truncated?: boolean;
  total_count?: number | null;
};

export type WorktreeVcsGitStatusSummary = {
  raw?: string;
  summary_line?: string;
  branch?: string | null;
  upstream?: string | null;
  ahead: number;
  behind: number;
  detached: boolean;
  staged: number;
  unstaged: number;
  untracked: number;
  entries?: WorktreeVcsTouchedFile[];
};

export type WorktreeVcsSnapshot = {
  worktree_id: string;
  rev: number;
  emitted_at_ms: number;
  base_commit_sha: string;
  head_commit_sha: string;
  target_branch?: string | null;
  target_branch_commit_sha?: string | null;
  base_resolution: WorktreeVcsBaseResolution;
  compute_state: WorktreeVcsComputeState;
  summary: WorktreeVcsSummary;
  git_status: WorktreeVcsGitStatusSummary;
  touched_files: WorktreeVcsTouchedFiles;
  available?: boolean;
  unavailable_reason?: DiffUnavailableReason | null;
  schema_version: number;
};

export type WorkspaceActiveTaskSummary = {
  task: Task;
  primary_session: SessionSnapshotSummary;
  primary_session_head?: SessionHeadSnapshot | null;
  sessions: SessionSnapshotSummary[];
  sort_at: string;
};

export type WorkspaceActivePage = {
  tasks: WorkspaceActiveTaskSummary[];
  total_count: number;
};

export type WorkspaceActiveSnapshot = {
  workspace_id: string;
  snapshot_rev: number;
  archived_rev?: number;
  active: WorkspaceActivePage;
  worktree_vcs_snapshots?: WorktreeVcsSnapshot[];
};

export type WorkspaceActiveHeadBatch = {
  workspace_id: string;
  snapshot_rev: number;
  heads: SessionHeadSnapshot[];
};

export type SessionSummaryCheckpoint = {
  session_id: string;
  checkpoint_id: string;
  summary: string;
  last_turn_id?: string | null;
  last_event_seq?: number | null;
  created_at: string;
  updated_at: string;
};

export type SessionHeadWindow = {
  turn_limit: number;
  message_limit: number;
  event_limit: number;
  byte_limit: number;
  turn_count: number;
  message_count: number;
  event_count: number;
  bytes: number;
  truncated?: boolean;
};

export type SessionHead = {
  session: Session;
  turns: SessionTurn[];
  tool_summaries?: SessionTurnToolSummary[];
  events?: SessionEvent[];
  messages: Message[];
  last_event_seq: number;
  activity?: SessionActivityState;
  has_more_turns: boolean;
  summary_checkpoint?: SessionSummaryCheckpoint | null;
  head_window?: SessionHeadWindow;
};

export type SessionHeadDelta = {
  session_id: string;
  last_event_seq: number;
  state_rev?: number;
  event?: SessionEvent | null;
  turn?: SessionTurn | null;
  message?: Message | null;
  tool_summaries?: SessionTurnToolSummary[];
};

export type SessionHistoryPage = {
  session_id: string;
  turns: SessionTurn[];
  messages: Message[];
  next_cursor?: number | null;
  has_more: boolean;
};

export type SessionEventsPage = {
  session_id: string;
  events: SessionEvent[];
  next_cursor?: number | null;
  has_more: boolean;
};

export type WorktreeBootstrapNotice = {
  worktree_id: string;
  worktree_root: string;
  status: "success" | "failed" | "timeout";
  started_at: string;
  finished_at: string;
  exit_code?: number | null;
  timeout_sec?: number | null;
  config_path?: string | null;
  config_key?: string | null;
  command?: string | null;
  script_path?: string | null;
  log_path?: string | null;
  log_truncated?: boolean | null;
  error?: string | null;
};

export type WorkspaceActiveSnapshotTaskDeltaKind = "updated" | "archived" | "unarchived";

export type WorkspaceActiveSnapshotTaskDelta = {
  kind: WorkspaceActiveSnapshotTaskDeltaKind;
  task: Task;
};

export type WorkspaceActiveSnapshotTaskDeltaEvent = {
  type: "task_delta";
  workspace_id: string;
  snapshot_rev: number;
  delta: WorkspaceActiveSnapshotTaskDelta;
};

export type WorkspaceActiveSnapshotSessionSummaryDelta = {
  session_id: string;
  task_id: string;
  activity?: SessionActivityState | null;
  last_message_at?: string | null;
  last_message_preview?: string | null;
  last_event_seq?: number | null;
  state_rev?: number | null;
};

export type WorkspaceActiveSnapshotSessionSummaryDeltaEvent = {
  type: "session_summary_delta";
  workspace_id: string;
  snapshot_rev: number;
  delta: WorkspaceActiveSnapshotSessionSummaryDelta;
};

export type WorkspaceActiveSnapshotEvent =
  | {
      type: "ready";
      workspace_id: string;
      snapshot_rev: number;
      archived_rev?: number;
    }
  | WorkspaceActiveSnapshotTaskDeltaEvent
  | {
      type: "active_task_upsert";
      workspace_id: string;
      snapshot_rev: number;
      task: WorkspaceActiveTaskSummary;
    }
  | {
      type: "active_task_delete";
      workspace_id: string;
      snapshot_rev: number;
      task_id: string;
    }
  | WorkspaceActiveSnapshotSessionSummaryDeltaEvent
  | {
      type: "session_summary";
      workspace_id: string;
      snapshot_rev: number;
      summary: SessionSnapshotSummary;
    }
  | {
      type: "session_head_delta";
      workspace_id: string;
      snapshot_rev: number;
      delta: SessionHeadDelta;
    }
  | {
      type: "session_head_seed";
      workspace_id: string;
      snapshot_rev: number;
      head: SessionHeadSnapshot;
    }
  | {
      type: "session_gap";
      workspace_id: string;
      snapshot_rev: number;
      session_id: string;
      after_seq: number;
      reason?: string | null;
    }
  | {
      type: "worktree_bootstrap";
      workspace_id: string;
      snapshot_rev: number;
      notice: WorktreeBootstrapNotice;
    }
  | {
      type: "worktree_vcs_snapshot";
      workspace_id: string;
      snapshot_rev: number;
      snapshot: WorktreeVcsSnapshot;
    }
  | {
      type: "archived_task_upsert";
      workspace_id: string;
      snapshot_rev?: number;
      archived_rev: number;
      task: WorkspaceTaskSummary;
    }
  | {
      type: "archived_task_delete";
      workspace_id: string;
      snapshot_rev?: number;
      archived_rev: number;
      task_id: string;
    };

export type WorkspaceActiveSnapshotSessionSubscription = {
  session_id: string;
  after_seq?: number | null;
};

export type WorkspaceActiveSnapshotSubscribeScope = "active";

export type WorkspaceActiveSnapshotClientMessage =
  | {
      type: "subscribe";
      session_ids?: (string)[];
      sessions?: WorkspaceActiveSnapshotSessionSubscription[];
      task_ids?: (string)[];
      foreground_task_id?: string;
      scope?: WorkspaceActiveSnapshotSubscribeScope | null;
      include_active_heads?: boolean;
    };

export type Message = {
  id: string;
  session_id: string;
  task_id: string;
  turn_id?: string | null;
  turn_sequence?: number | null;
  role: "user" | "assistant" | "system";
  content: string;
  attachments?: MessageAttachment[];
  delivery: "immediate" | "queued";
  created_at: string;
};

export type Artifact = {
  id: string;
  session_id: string;
  task_id: string;
  workspace_id: string;
  worktree_id: string;
  name?: string | null;
  absolute_path: string;
  mime_type: string;
  bytes: number;
  created_at: string;
  missing?: boolean | null;
};

export type MessageAttachment =
  | {
      kind: "image";
      mime_type: string;
      data_base64: string;
      name?: string | null;
    }
  | {
      kind: "image_ref";
      blob_id: string;
      mime_type: string;
      name?: string | null;
    };

export type SessionEventType =
  | "init"
  | "user_message"
  | "input_queued"
  | "turn_queued"
  | "turn_started"
  | "turn_finished"
  | "message_queue_added"
  | "message_queue_updated"
  | "message_queue_removed"
  | "message_queue_promoted"
  | "auth_required"
  | "notice"
  | "assistant_chunk"
  | "thought_chunk"
  | "assistant_complete"
  | "assistant_message_inserted"
  | "tool_call"
  | "tool_call_update"
  | "tool_result"
  | "plan"
  | "artifacts_set"
  | "done"
  | "interrupt_requested"
  | "turn_interrupted"
  | "error";

export type TurnLifecycleEventPayload = {
  message_id?: string;
  queue_position?: number | null;
  status?: SessionTurnStatus;
  reason?: string;
  provider_cancelled?: boolean;
};

export type MessageQueueEventPayload = {
  message_id: string;
  queue_position?: number | null;
  previous_position?: number | null;
  reason?: string;
};

export type SessionEvent = {
  seq: number;
  id: string;
  session_id: string;
  run_id?: string | null;
  turn_id?: string | null;
  event_type: SessionEventType;
  payload_json: Record<string, unknown> | null;
  transient?: boolean;
  created_at: string;
};

export type SessionTurnStatus = "queued" | "running" | "completed" | "interrupted" | "failed";

export type SessionActivityState = {
  is_working: boolean;
  last_turn_status?: SessionTurnStatus | null;
};

export type SessionTurn = {
  turn_id: string;
  session_id: string;
  run_id?: string | null;
  user_message_id?: string | null;
  status: SessionTurnStatus;
  start_seq?: number | null;
  end_seq?: number | null;
  started_at: string;
  updated_at: string;
  assistant_partial?: string | null;
  thought_partial?: string | null;
  metrics_json?: Record<string, unknown> | null;
  tool_total: number;
  tool_pending: number;
  tool_running: number;
  tool_completed: number;
  tool_failed: number;
};

export type SessionTurnTool = {
  session_id: string;
  tool_call_id: string;
  turn_id: string;
  tool_kind?: string | null;
  title?: string | null;
  status?: string | null;
  input_json?: Record<string, unknown> | null;
  output_text?: string | null;
  input_truncated?: boolean | null;
  input_original_bytes?: number | null;
  output_truncated?: boolean | null;
  output_original_bytes?: number | null;
  created_at: string;
  updated_at: string;
};

export type SessionTurnToolSummary = {
  session_id: string;
  tool_call_id: string;
  turn_id: string;
  tool_kind?: string | null;
  title?: string | null;
  status?: string | null;
  input_preview?: Record<string, unknown> | null;
  output_preview?: string | null;
  input_truncated?: boolean | null;
  input_original_bytes?: number | null;
  output_truncated?: boolean | null;
  output_original_bytes?: number | null;
  created_at: string;
  updated_at: string;
};

export type ProviderStatus = {
  provider_id: string;
  installed: boolean;
  detected_path?: string | null;
  version?: string | null;
  health: string;
  diagnostics: string[];
  details?: Record<string, string>;
};

export type InstallEventLevel = "info" | "warning" | "error" | "success";

export type InstallProgressEvent = {
  install_id: string;
  provider_id: string;
  at: string;
  stage: string;
  message: string;
  level: InstallEventLevel;
  bytes?: number;
  total_bytes?: number;
  attempt?: number;
};

export type Diagnostics = {
  daemon: {
    version: string;
    daemon_version: string;
    pid: number;
    data_root: string;
    daemon_url: string;
    auth_required: boolean;
    compatibility: {
      desktop_exact_version: string;
      mobile_api_min: number;
      mobile_api_max: number;
    };
  };
  platform: { os: string; arch: string };
  logs: { dir: string; files: { name: string; bytes: number; modified_utc?: string | null }[] };
  providers: ProviderStatus[];
  managed_installs: Record<string, unknown>;
};

export type ResourceProcess = {
  label: string;
  pid: number;
  cpu_pct: number;
  memory_bytes: number;
  virtual_memory_bytes: number;
  child_count: number;
  children: ResourceChildProcess[];
  children_truncated: boolean;
};

export type ResourceChildProcess = {
  pid: number;
  parent_pid?: number | null;
  name: string;
  cmdline?: string | null;
  cpu_pct: number;
  memory_bytes: number;
  virtual_memory_bytes: number;
};

export type ResourceDisk = {
  name: string;
  mount_point: string;
  total_bytes: number;
  available_bytes: number;
  file_system: string;
};

export type ResourceWorktreeDisk = {
  worktree_id: string;
  root_path: string;
  size_bytes: number;
};

export type ResourceWorkspaceDisk = {
  workspace_id: string;
  root_path: string;
  size_bytes: number;
  size_collected_at: string;
  size_cache_age_ms: number;
  disk?: ResourceDisk | null;
  worktrees: ResourceWorktreeDisk[];
};

export type ResourceUtilization = {
  collected_at: string;
  cache_age_ms: number;
  system: {
    cpu_pct: number;
    memory_total_bytes: number;
    memory_used_bytes: number;
    swap_total_bytes: number;
    swap_used_bytes: number;
  };
  processes: {
    daemon?: ResourceProcess | null;
    providers: ResourceProcess[];
  };
  workspace: ResourceWorkspaceDisk;
};

export type TelemetryMetricKind = "histogram" | "counter" | "gauge";

export type TelemetryMetricSummary = {
  name: string;
  kind: TelemetryMetricKind;
  unit: string;
  labels: Record<string, string>;
  run_id?: string | null;
  count: number;
  sum: number;
  min?: number | null;
  max?: number | null;
  p50?: number | null;
  p95?: number | null;
  p99?: number | null;
};

export type TelemetrySummaryResponse = {
  generated_at: string;
  window_ms?: number | null;
  metrics: TelemetryMetricSummary[];
};

export type ClientTelemetryMetric = {
  name: string;
  kind: TelemetryMetricKind;
  unit: string;
  value: number;
  labels?: Record<string, string>;
  run_id?: string | null;
};

export type ClientTelemetryBatch = {
  events: ClientTelemetryMetric[];
};

export type MobileConnectionProfile = {
  id: string;
  label: string;
  base_url: string;
  token_prefix: string;
  scopes: string[];
  created_at: string;
  last_used_at?: string | null;
};

export type MobileDeviceRegistration = {
  id: string;
  profile_id: string;
  device_label?: string | null;
  platform?: string | null;
  push_token?: string | null;
  push_provider?: string | null;
  public_key?: string | null;
  app_version?: string | null;
  created_at: string;
  last_seen_at: string;
};
