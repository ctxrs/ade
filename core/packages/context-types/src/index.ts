export type Workspace = {
  id: { 0: string } | string;
  name: string;
  root_path: string;
  created_at: string;
};

export type Task = {
  id: { 0: string } | string;
  workspace_id: { 0: string } | string;
  title: string;
  description?: string | null;
  status: string;
  created_at: string;
  updated_at: string;
  archived_at?: string | null;
  assistant_seen_at?: string | null;
  last_activity_at?: string | null;
  last_assistant_message_at?: string | null;
  has_active_session?: boolean;
};

export type Track = {
  id: { 0: string } | string;
  task_id: { 0: string } | string;
  workspace_id: { 0: string } | string;
  worktree_id: { 0: string } | string;
  label: string;
  status: string;
  created_at?: string;
  updated_at?: string;
};

export type Worktree = {
  id: { 0: string } | string;
  workspace_id: { 0: string } | string;
  root_path: string;
  base_commit_sha: string;
  git_branch?: string | null;
  created_at: string;
};

export type Session = {
  id: { 0: string } | string;
  track_id: { 0: string } | string;
  task_id: { 0: string } | string;
  workspace_id: { 0: string } | string;
  worktree_id: { 0: string } | string;
  provider_id: string;
  model_id: string;
  agent_role: string;
  status: string;
  env_target?: "worktree" | "local" | string;
  created_at?: string;
  updated_at?: string;
};

export type SessionSummary = {
  id: { 0: string } | string;
  track_id: { 0: string } | string;
  task_id: { 0: string } | string;
  workspace_id: { 0: string } | string;
  provider_id: string;
  model_id: string;
  status: string;
  created_at: string;
  updated_at: string;
};

export type TrackSummary = {
  track: Track;
  sessions?: SessionSummary[];
};

export type WorkspaceTaskSummary = {
  task: Task;
  provider_ids?: string[];
  tracks: TrackSummary[];
  sort_at: string;
};

export type WorkspaceIndexCursor = {
  sort_at: string;
  task_id: { 0: string } | string;
};

export type WorkspaceIndexPage = {
  workspace_id: { 0: string } | string;
  snapshot_rev: number;
  tasks: WorkspaceTaskSummary[];
  next_cursor?: WorkspaceIndexCursor | null;
  total_active: number;
  total_archived: number;
};

export type WorkspaceIndexEvent =
  | {
      type: "ready";
      workspace_id: { 0: string } | string;
      snapshot_rev: number;
    }
  | {
      type: "task_upsert";
      workspace_id: { 0: string } | string;
      snapshot_rev: number;
      task: WorkspaceTaskSummary;
    }
  | {
      type: "task_delete";
      workspace_id: { 0: string } | string;
      snapshot_rev: number;
      task_id: { 0: string } | string;
    };

export type WorkspaceCatchupCursor = {
  sort_at: string;
  task_id: { 0: string } | string;
};

export type SessionCatchupSummary = {
  session: Session;
  last_message_at?: string | null;
  last_message_preview?: string | null;
  last_event_seq?: number | null;
  unread?: boolean;
};

export type TrackDiffSummary = {
  file_count: number;
  line_additions: number;
  line_deletions: number;
  updated_at: string;
};

export type TrackDiffSummaryResponse = {
  summary?: TrackDiffSummary | null;
  too_large: boolean;
};

export type WorkspaceCatchupTrackSummary = {
  track: Track;
  primary_session_id?: { 0: string } | string | null;
  sessions: SessionCatchupSummary[];
  diff_summary?: TrackDiffSummary | null;
};

export type WorkspaceCatchupTaskSummary = {
  task: Task;
  tracks: WorkspaceCatchupTrackSummary[];
  sort_at: string;
};

export type WorkspaceCatchupPage = {
  tasks: WorkspaceCatchupTaskSummary[];
  next_cursor?: WorkspaceCatchupCursor | null;
  total_count: number;
};

export type WorkspaceCatchupSnapshot = {
  workspace_id: { 0: string } | string;
  snapshot_rev: number;
  active: WorkspaceCatchupPage;
  archived?: WorkspaceCatchupPage | null;
};

export type SessionHead = {
  session: Session;
  turns: SessionTurn[];
  tool_summaries?: SessionTurnToolSummary[];
  events?: SessionEvent[];
  messages: Message[];
  last_event_seq: number;
  has_more_turns: boolean;
};

export type SessionHeadDelta = {
  session_id: { 0: string } | string;
  last_event_seq: number;
  event?: SessionEvent | null;
  turn?: SessionTurn | null;
  message?: Message | null;
};

export type SessionHistoryPage = {
  session_id: { 0: string } | string;
  turns: SessionTurn[];
  messages: Message[];
  next_cursor?: number | null;
  has_more: boolean;
};

export type WorkspaceCatchupEvent =
  | {
      type: "ready";
      workspace_id: { 0: string } | string;
      snapshot_rev: number;
    }
  | {
      type: "task_upsert";
      workspace_id: { 0: string } | string;
      snapshot_rev: number;
      task: WorkspaceCatchupTaskSummary;
    }
  | {
      type: "task_delete";
      workspace_id: { 0: string } | string;
      snapshot_rev: number;
      task_id: { 0: string } | string;
    }
  | {
      type: "track_upsert";
      workspace_id: { 0: string } | string;
      snapshot_rev: number;
      track: WorkspaceCatchupTrackSummary;
    }
  | {
      type: "session_summary";
      workspace_id: { 0: string } | string;
      snapshot_rev: number;
      summary: SessionCatchupSummary;
    }
  | {
      type: "session_head_delta";
      workspace_id: { 0: string } | string;
      snapshot_rev: number;
      delta: SessionHeadDelta;
    };

export type WorkspaceCatchupClientMessage =
  | {
      type: "subscribe";
      session_ids: ({ 0: string } | string)[];
    };

export type Message = {
  id: { 0: string } | string;
  session_id: { 0: string } | string;
  turn_id?: { 0: string } | string | null;
  turn_sequence?: number | null;
  role: "user" | "assistant" | "system";
  content: string;
  attachments?: MessageAttachment[];
  delivery: "immediate" | "queued";
  created_at: string;
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

export type SessionEvent = {
  seq: number;
  id: { 0: string } | string;
  session_id: { 0: string } | string;
  run_id?: { 0: string } | string | null;
  turn_id?: { 0: string } | string | null;
  event_type: string;
  payload_json: any;
  created_at: string;
};

export type SessionTurnStatus = "queued" | "running" | "completed" | "interrupted" | "failed";

export type SessionTurn = {
  turn_id: { 0: string } | string;
  session_id: { 0: string } | string;
  run_id?: { 0: string } | string | null;
  user_message_id?: { 0: string } | string | null;
  status: SessionTurnStatus;
  start_seq?: number | null;
  end_seq?: number | null;
  started_at: string;
  updated_at: string;
  assistant_partial?: string | null;
  thought_partial?: string | null;
  metrics_json?: any;
  tool_total: number;
  tool_pending: number;
  tool_running: number;
  tool_completed: number;
  tool_failed: number;
};

export type SessionTurnTool = {
  session_id: { 0: string } | string;
  tool_call_id: string;
  turn_id: { 0: string } | string;
  tool_kind?: string | null;
  title?: string | null;
  status?: string | null;
  input_json?: any;
  output_text?: string | null;
  created_at: string;
  updated_at: string;
};

export type SessionTurnToolSummary = {
  session_id: { 0: string } | string;
  tool_call_id: string;
  turn_id: { 0: string } | string;
  tool_kind?: string | null;
  title?: string | null;
  status?: string | null;
  input_preview?: any;
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
    pid: number;
    data_root: string;
    daemon_url: string;
    auth_required: boolean;
  };
  platform: { os: string; arch: string };
  logs: { dir: string; files: { name: string; bytes: number; modified_utc?: string | null }[] };
  providers: ProviderStatus[];
  managed_installs: any;
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
