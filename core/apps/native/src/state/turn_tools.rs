use std::collections::HashMap;

use chrono::{DateTime, Utc};
use ctx_core::ids::TurnId;
use ctx_core::models::{SessionEvent, SessionEventType, SessionTurnTool};
use serde_json::Value;

#[derive(Debug, Clone)]
pub(crate) struct TurnToolItem {
    pub(crate) tool_call_id: String,
    pub(crate) tool_kind: String,
    pub(crate) title: String,
    pub(crate) status: String,
    pub(crate) input: Option<Value>,
    pub(crate) output_text: Option<String>,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
    pub(crate) order_seq: i64,
    pub(crate) last_seq: i64,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct TurnToolSummary {
    pub(crate) total: usize,
    pub(crate) pending: usize,
    pub(crate) running: usize,
    pub(crate) completed: usize,
    pub(crate) failed: usize,
}

impl TurnToolSummary {
    pub(crate) fn label(&self) -> String {
        if self.total == 0 {
            return "No tools".to_string();
        }
        let mut parts = vec![format!(
            "{} tool{}",
            self.total,
            if self.total == 1 { "" } else { "s" }
        )];
        if self.running > 0 {
            parts.push(format!("{} running", self.running));
        }
        if self.failed > 0 {
            parts.push(format!("{} failed", self.failed));
        }
        parts.join(" | ")
    }
}

#[derive(Debug, Clone)]
pub(crate) struct TurnToolGroup {
    pub(crate) turn_id: Option<TurnId>,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
    pub(crate) tools: Vec<TurnToolItem>,
    pub(crate) summary: TurnToolSummary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolStatusTone {
    Pending,
    Running,
    Success,
    Failed,
    Unknown,
}

pub(crate) fn build_turn_tool_groups(
    events: &[SessionEvent],
    turn_tools: Option<&[SessionTurnTool]>,
) -> Vec<TurnToolGroup> {
    let mut groups = build_turn_tool_groups_from_events(events);
    if let Some(turn_tools) = turn_tools {
        merge_turn_tool_groups(&mut groups, turn_tools);
    }
    groups
}

pub(crate) fn build_turn_tool_groups_from_events(events: &[SessionEvent]) -> Vec<TurnToolGroup> {
    let mut groups: HashMap<Option<TurnId>, TurnToolGroupBuilder> = HashMap::new();

    for event in events {
        if !is_tool_event(&event.event_type) {
            continue;
        }
        let update = event_update(event);
        let Some(tool_call_id) = extract_tool_call_id(event, update) else {
            continue;
        };
        let group = groups
            .entry(event.turn_id)
            .or_insert_with(|| TurnToolGroupBuilder::new(event.turn_id, event.created_at));
        group.touch(event.created_at);
        let tool = group
            .tools
            .entry(tool_call_id.clone())
            .or_insert_with(|| {
                TurnToolItem::new(
                    tool_call_id.clone(),
                    event.created_at,
                    event.seq,
                )
            });
        apply_tool_update(tool, event, update);
    }

    let mut out: Vec<TurnToolGroup> = groups.into_values().map(|builder| builder.finish()).collect();
    out.sort_by_key(|group| group.created_at);
    out
}

pub(crate) fn merge_turn_tool_groups(groups: &mut Vec<TurnToolGroup>, turn_tools: &[SessionTurnTool]) {
    let mut group_index: HashMap<TurnId, usize> = HashMap::new();
    for (idx, group) in groups.iter().enumerate() {
        if let Some(turn_id) = group.turn_id {
            group_index.insert(turn_id, idx);
        }
    }

    for tool in turn_tools {
        let idx = match group_index.get(&tool.turn_id) {
            Some(idx) => *idx,
            None => {
                let next = groups.len();
                groups.push(TurnToolGroup {
                    turn_id: Some(tool.turn_id),
                    created_at: tool.created_at,
                    updated_at: tool.updated_at,
                    tools: Vec::new(),
                    summary: TurnToolSummary::default(),
                });
                group_index.insert(tool.turn_id, next);
                next
            }
        };

        let group = &mut groups[idx];
        if tool.created_at < group.created_at {
            group.created_at = tool.created_at;
        }
        if tool.updated_at > group.updated_at {
            group.updated_at = tool.updated_at;
        }

        if let Some(existing) = group
            .tools
            .iter_mut()
            .find(|item| item.tool_call_id == tool.tool_call_id)
        {
            apply_tool_snapshot(existing, tool);
        } else {
            group.tools.push(tool_from_snapshot(tool));
        }
        refresh_group_summary(group);
        sort_group_tools(group);
    }

    groups.sort_by_key(|group| group.created_at);
}

pub(crate) fn tool_status_tone(status: &str) -> ToolStatusTone {
    let normalized = normalize_tool_status(status, None);
    match normalized.as_str() {
        "pending" => ToolStatusTone::Pending,
        "in_progress" => ToolStatusTone::Running,
        "completed" => ToolStatusTone::Success,
        "failed" => ToolStatusTone::Failed,
        _ => ToolStatusTone::Unknown,
    }
}

pub(crate) fn human_tool_kind(kind: &str) -> String {
    let k = kind.trim().to_ascii_lowercase();
    match k.as_str() {
        "execute" => "Run Command",
        "search" => "Search",
        "read" | "read_file" => "Read File",
        "edit" | "write" | "apply_patch" => "Edit File",
        "list" | "list_files" => "List Files",
        "fetch" | "http" | "curl" => "Fetch",
        "think" => "Think",
        "error" => "Error",
        "" => "Tool",
        _ => kind,
    }
    .to_string()
}

struct TurnToolGroupBuilder {
    turn_id: Option<TurnId>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    tools: HashMap<String, TurnToolItem>,
}

impl TurnToolGroupBuilder {
    fn new(turn_id: Option<TurnId>, created_at: DateTime<Utc>) -> Self {
        Self {
            turn_id,
            created_at,
            updated_at: created_at,
            tools: HashMap::new(),
        }
    }

    fn touch(&mut self, created_at: DateTime<Utc>) {
        if created_at < self.created_at {
            self.created_at = created_at;
        }
        if created_at > self.updated_at {
            self.updated_at = created_at;
        }
    }

    fn finish(self) -> TurnToolGroup {
        let mut tools: Vec<TurnToolItem> = self.tools.into_values().collect();
        sort_tools(&mut tools);
        let summary = compute_summary(&tools);
        TurnToolGroup {
            turn_id: self.turn_id,
            created_at: self.created_at,
            updated_at: self.updated_at,
            tools,
            summary,
        }
    }
}

impl TurnToolItem {
    fn new(tool_call_id: String, created_at: DateTime<Utc>, seq: i64) -> Self {
        Self {
            tool_call_id,
            tool_kind: "tool".to_string(),
            title: "Tool".to_string(),
            status: "pending".to_string(),
            input: None,
            output_text: None,
            created_at,
            updated_at: created_at,
            order_seq: seq,
            last_seq: seq,
        }
    }
}

fn tool_from_snapshot(tool: &SessionTurnTool) -> TurnToolItem {
    let tool_kind = tool
        .tool_kind
        .as_deref()
        .unwrap_or("tool")
        .trim()
        .to_string();
    let mut title = tool
        .title
        .as_deref()
        .unwrap_or("")
        .trim()
        .to_string();
    if title.is_empty() {
        title = human_tool_kind(&tool_kind);
    }
    let status = tool
        .status
        .as_deref()
        .map(|status| normalize_tool_status(status, None))
        .unwrap_or_else(|| "pending".to_string());
    TurnToolItem {
        tool_call_id: tool.tool_call_id.clone(),
        tool_kind,
        title,
        status,
        input: tool.input_json.clone(),
        output_text: tool
            .output_text
            .as_deref()
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty()),
        created_at: tool.created_at,
        updated_at: tool.updated_at,
        order_seq: 0,
        last_seq: 0,
    }
}

fn apply_tool_snapshot(item: &mut TurnToolItem, tool: &SessionTurnTool) {
    if let Some(kind) = tool.tool_kind.as_deref().map(|k| k.trim()).filter(|k| !k.is_empty()) {
        item.tool_kind = kind.to_string();
    }
    if let Some(title) = tool.title.as_deref().map(|t| t.trim()).filter(|t| !t.is_empty()) {
        item.title = title.to_string();
    } else if item.title == "Tool" && !item.tool_kind.is_empty() {
        item.title = human_tool_kind(&item.tool_kind);
    }
    if let Some(status) = tool.status.as_deref() {
        item.status = normalize_tool_status(status, None);
    }
    if let Some(input) = tool.input_json.as_ref() {
        item.input = Some(input.clone());
    }
    if let Some(output) = tool.output_text.as_deref().map(|t| t.trim()).filter(|t| !t.is_empty()) {
        item.output_text = Some(output.to_string());
    }
    if tool.created_at < item.created_at {
        item.created_at = tool.created_at;
    }
    if tool.updated_at > item.updated_at {
        item.updated_at = tool.updated_at;
    }
}

fn refresh_group_summary(group: &mut TurnToolGroup) {
    group.summary = compute_summary(&group.tools);
}

fn sort_group_tools(group: &mut TurnToolGroup) {
    sort_tools(&mut group.tools);
}

fn sort_tools(tools: &mut Vec<TurnToolItem>) {
    tools.sort_by(|a, b| {
        let a_seq = if a.order_seq > 0 || a.last_seq > 0 {
            Some((a.order_seq, a.last_seq))
        } else {
            None
        };
        let b_seq = if b.order_seq > 0 || b.last_seq > 0 {
            Some((b.order_seq, b.last_seq))
        } else {
            None
        };

        match (a_seq, b_seq) {
            (Some(a_key), Some(b_key)) => a_key
                .cmp(&b_key)
                .then_with(|| a.created_at.cmp(&b.created_at))
                .then_with(|| a.tool_call_id.cmp(&b.tool_call_id)),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a
                .created_at
                .cmp(&b.created_at)
                .then_with(|| a.tool_call_id.cmp(&b.tool_call_id)),
        }
    });
}

fn compute_summary(tools: &[TurnToolItem]) -> TurnToolSummary {
    let mut summary = TurnToolSummary::default();
    summary.total = tools.len();
    for tool in tools {
        match tool_status_tone(&tool.status) {
            ToolStatusTone::Pending => summary.pending += 1,
            ToolStatusTone::Running => summary.running += 1,
            ToolStatusTone::Success => summary.completed += 1,
            ToolStatusTone::Failed => summary.failed += 1,
            ToolStatusTone::Unknown => {}
        }
    }
    summary
}

fn is_tool_event(event_type: &SessionEventType) -> bool {
    matches!(
        event_type,
        SessionEventType::ToolCall | SessionEventType::ToolCallUpdate | SessionEventType::ToolResult
    )
}

fn event_update(event: &SessionEvent) -> &Value {
    event
        .payload_json
        .get("acp_update")
        .unwrap_or(&event.payload_json)
}

fn extract_tool_call_id(event: &SessionEvent, update: &Value) -> Option<String> {
    first_non_empty_string([
        event.payload_json.get("tool_call_id"),
        update.get("toolCallId"),
        update.get("tool_call_id"),
        update.pointer("/toolCall/rawInput/call_id"),
        update.pointer("/rawInput/call_id"),
    ])
}

fn apply_tool_update(item: &mut TurnToolItem, event: &SessionEvent, update: &Value) {
    item.updated_at = event.created_at;
    item.last_seq = event.seq;

    if let Some(kind) = extract_tool_kind(update) {
        item.tool_kind = kind;
    }

    if let Some(title) = extract_tool_title(update) {
        item.title = title;
    } else if item.title == "Tool" && !item.tool_kind.is_empty() {
        item.title = human_tool_kind(&item.tool_kind);
    }

    if let Some(status) = extract_tool_status(update) {
        item.status = normalize_tool_status(&status, Some(&event.event_type));
    } else if matches!(event.event_type, SessionEventType::ToolResult) {
        item.status = "completed".to_string();
    }

    if let Some(input) = extract_tool_input(update) {
        item.input = Some(input);
    }

    if let Some(output) = extract_tool_output_text(update) {
        let next = if let Some(prev) = item.output_text.as_deref() {
            merge_streaming_text(prev, &output)
        } else {
            output
        };
        if !next.trim().is_empty() {
            item.output_text = Some(next);
        }
    }
}

fn extract_tool_kind(update: &Value) -> Option<String> {
    first_non_empty_string([
        update.get("kind"),
        update.get("tool_kind"),
        update.get("toolKind"),
        update.pointer("/toolCall/kind"),
    ])
}

fn extract_tool_title(update: &Value) -> Option<String> {
    first_non_empty_string([
        update.get("title"),
        update.pointer("/toolCall/title"),
        update.pointer("/toolCall/name"),
    ])
}

fn extract_tool_status(update: &Value) -> Option<String> {
    first_non_empty_string([
        update.get("status"),
        update.get("tool_status"),
        update.get("toolStatus"),
        update.pointer("/toolCall/status"),
    ])
}

fn extract_tool_input(update: &Value) -> Option<Value> {
    for candidate in [
        update.get("rawInput"),
        update.pointer("/toolCall/rawInput"),
        update.pointer("/toolCall/input"),
        update.get("input"),
        update.get("input_preview"),
    ] {
        if let Some(value) = candidate {
            if !value.is_null() {
                return Some(value.clone());
            }
        }
    }
    None
}

fn extract_tool_output_text(update: &Value) -> Option<String> {
    let direct = first_non_empty_string([
        update.get("outputText"),
        update.get("output_text"),
        update.get("output_preview"),
        update.get("result"),
        update.pointer("/rawOutput/aggregated_output"),
        update.pointer("/rawOutput/output"),
    ]);
    if direct.is_some() {
        return direct;
    }

    let mut parts = Vec::new();
    if let Some(blocks) = update.get("content").and_then(|v| v.as_array()) {
        for block in blocks {
            let content = block.get("content").unwrap_or(block);
            if let Some(text) = content.get("text").and_then(|v| v.as_str()) {
                parts.push(text);
            }
        }
    }
    let joined = parts.join("");
    if joined.trim().is_empty() {
        None
    } else {
        Some(joined.trim().to_string())
    }
}

fn normalize_tool_status(status: &str, event_type: Option<&SessionEventType>) -> String {
    let lowercased = status.trim().to_ascii_lowercase();
    let normalized = match lowercased.as_str() {
        "inprogress" | "in_progress" | "running" => "in_progress",
        "pending" | "queued" => "pending",
        "completed" | "complete" | "ok" | "succeeded" => "completed",
        "failed" | "error" => "failed",
        other => other,
    };
    if normalized.is_empty() {
        if matches!(event_type, Some(SessionEventType::ToolResult)) {
            "completed".to_string()
        } else {
            "pending".to_string()
        }
    } else {
        normalized.to_string()
    }
}

fn merge_streaming_text(prev: &str, next: &str) -> String {
    let p = prev;
    let n = next;
    if p.is_empty() {
        return n.to_string();
    }
    if n.is_empty() {
        return p.to_string();
    }
    if n.starts_with(p) {
        return n.to_string();
    }
    if p.ends_with(n) {
        return p.to_string();
    }
    format!("{p}{n}")
}

fn first_non_empty_string<'a, const N: usize>(values: [Option<&'a Value>; N]) -> Option<String> {
    for value in values {
        if let Some(text) = value.and_then(|v| v.as_str()) {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}
