import type { MessageAttachment, SessionEvent, SessionTurn } from "../api/client";

export type ThreadItem =
  | {
      kind: "message";
      id: string;
      role: "user" | "assistant" | "system";
      content: string;
      attachments: MessageAttachment[];
      created_at: string;
    }
  | {
      kind: "spacer";
      id: string;
      created_at: string;
    }
  | {
      kind: "assistant";
      id: string;
      turn_id: string;
      created_at: string;
      content: string;
      thought: string;
      is_complete: boolean;
      thought_seconds?: number;
    }
  | {
      kind: "thought";
      id: string;
      turn_id: string;
      created_at: string;
      content: string;
    }
  | {
      kind: "turn_status";
      id: string;
      turn_id: string;
      created_at: string;
      status: SessionTurn["status"];
      started_at: string;
      updated_at: string;
      custom_status?: string | null;
      assistant_messages_content?: string;
    }
  | {
      kind: "tool_group";
      id: string;
      turn_id: string;
      created_at: string;
      updated_at: string;
      tool_total: number;
      tool_pending: number;
      tool_running: number;
      tool_completed: number;
      tool_failed: number;
      tools: Array<Extract<ThreadItem, { kind: "tool" }>>;
      thought: string;
    }
  | {
      kind: "tool";
      id: string;
      created_at: string;
      updated_at: string;
      tool_call_id: string;
      tool_kind: string;
      title: string;
      status: string;
      locations: Array<{ path?: string; range?: any }>;
      input: any;
      output_text: string;
      raw: any;
      updates_seen: number;
      has_details?: boolean;
    }
  | {
      kind: "ask_user_question";
      id: string;
      turn_id: string;
      created_at: string;
      tool_call_id: string;
      input: any;
      answers?: Record<string, string>;
      outcome?: "submitted" | "cancelled";
      answered: boolean;
    };

export type AskUserQuestionAnswerState = {
  outcome: "submitted" | "cancelled";
  answers: Record<string, string>;
};

export type WorkbenchTurnHeader = {
  id: string;
  content: string;
  plain_text?: string;
  attachments: MessageAttachment[];
  created_at: string;
};

export type WorkbenchListItem =
  | ThreadItem
  | {
      kind: "turn_header";
      id: string;
      header: WorkbenchTurnHeader;
    };

export type ScrollbarDragState = {
  pointerId: number;
  startY: number;
  startScrollTop: number;
  trackHeight: number;
  thumbHeight: number;
  scrollHeight: number;
  clientHeight: number;
};

export type WorkbenchThreadView = {
  groups: Array<{
    key: string;
    header: WorkbenchTurnHeader | null;
    items: ThreadItem[];
  }>;
  debugEvents: SessionEvent[];
};
