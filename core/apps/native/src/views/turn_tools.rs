use std::collections::HashSet;

use gpui::{div, prelude::*, px};

use ctx_core::ids::TurnId;
use serde_json::Value;

use crate::theme::{ThemeColors, ThemeMetrics};

use super::super::state::turn_tools::{
    human_tool_kind, ToolStatusTone, TurnToolGroup, TurnToolItem,
};

pub(super) struct TurnToolsView<'a> {
    pub(super) colors: ThemeColors,
    pub(super) groups: &'a [TurnToolGroup],
}

impl<'a> TurnToolsView<'a> {
    pub(super) fn render(&self) -> impl IntoElement {
        let metrics = ThemeMetrics::default();
        let total_tools: usize = self.groups.iter().map(|group| group.tools.len()).sum();
        let list = if self.groups.is_empty() {
            div()
                .text_sm()
                .text_color(self.colors.muted)
                .child("No tools yet.")
        } else {
            self.groups
                .iter()
                .fold(div().flex().flex_col().gap(px(metrics.spacing.xl)), |list, group| {
                    list.child(self.render_group(group))
                })
        };

        div()
            .id("turn-tools")
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_sm()
                    .text_color(self.colors.muted)
                    .child("Tools")
                    .child(format!("{}", total_tools)),
            )
            .child(div().h(px(metrics.spacing.md)))
            .child(list)
    }

    fn render_group(&self, group: &TurnToolGroup) -> impl IntoElement {
        let metrics = ThemeMetrics::default();
        // Match web copy and separators for summary: use " · " instead of " | "
        let label = group.summary.label().replace(" | ", " · ");
        let turn_label = format_turn_label(group.turn_id);
        let mut time_line = format!("updated {}", group.updated_at.to_rfc3339());
        if group.updated_at != group.created_at {
            time_line = format!(
                "started {} · {}",
                group.created_at.to_rfc3339(),
                time_line
            );
        }
        let tools_list = if group.tools.is_empty() {
            div()
                .text_sm()
                .text_color(self.colors.muted)
                .child("No tool activity.")
        } else {
            group
                .tools
                .iter()
                .fold(div().flex().flex_col().gap(px(metrics.spacing.md)), |list, tool| list.child(self.render_tool(tool)))
        };

        // Header row (match web wb-event-row styling: subtle, muted, tight padding)
        let header_row = div()
            .flex()
            .items_center()
            .justify_between()
            .px(px(metrics.spacing.xs))
            .py(px(0.0))
            .text_sm()
            .text_color(self.colors.muted)
            .child(turn_label)
            .child(label);

        // Body with bordered container similar to wb-tool-group-body
        let body = div()
            .flex()
            .flex_col()
            .gap(px(metrics.spacing.xl))
            .border_1()
            .border_color(self.colors.border)
            .rounded_sm()
            .bg(self.colors.panel_2)
            .p(px(metrics.spacing.md))
            .child(div().text_sm().text_color(self.colors.muted).child(time_line))
            .child(tools_list);

        div()
            .flex()
            .flex_col()
            .gap(px(metrics.spacing.md))
            .child(header_row)
            .child(body)
    }

    fn render_tool(&self, tool: &TurnToolItem) -> impl IntoElement {
        let metrics = ThemeMetrics::default();
        let label_parts = tool_label_parts(tool);

        let mut time_line = format!("updated {}", tool.updated_at.to_rfc3339());
        if tool.updated_at != tool.created_at {
            time_line = format!(
                "started {} · {}",
                tool.created_at.to_rfc3339(),
                time_line
            );
        }

        let input_preview = tool_input_preview(tool);
        let output_preview = tool_output_preview(tool);
        let mut details = div().flex().flex_col().gap_1();
        let mut has_details = false;
        if let Some(preview) = input_preview {
            has_details = true;
            details = details.child(
                div()
                    .text_sm()
                    .text_color(self.colors.muted)
                    .child(format!("Input: {}", preview)),
            );
        }
        if let Some(preview) = output_preview {
            has_details = true;
            details = details.child(
                div()
                    .text_sm()
                    .text_color(self.colors.muted)
                    .child(format!("Output: {}", preview)),
            );
        }

        // Compact event row with verb · rest like wb-tool-row
        let event_row = div()
            .flex()
            .items_center()
            .justify_between()
            .px(px(metrics.spacing.xs))
            .py(px(0.0))
            .text_sm()
            .text_color(self.colors.muted)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(metrics.spacing.md))
                    .child(div().text_color(self.colors.text).child(label_parts.verb.clone()))
                    .child(if label_parts.rest.is_empty() {
                        div().into_any_element()
                    } else {
                        div()
                            .text_color(self.colors.muted)
                            .child(format!(" · {}", label_parts.rest))
                            .into_any_element()
                    }),
            );

        // Optional details in a bordered container
        let details_block = if has_details {
            div()
                .flex()
                .flex_col()
                .gap(px(metrics.spacing.xl))
                .border_1()
                .border_color(self.colors.border)
                .rounded_sm()
                .bg(self.colors.panel_2)
                .p(px(metrics.spacing.md))
                .child(details)
                .into_any_element()
        } else {
            div().into_any_element()
        };

        div()
            .flex()
            .flex_col()
            .gap(px(metrics.spacing.md))
            .child(event_row)
            .child(div().text_sm().text_color(self.colors.muted).child(time_line))
            .child(details_block)
    }
}

fn format_turn_label(turn_id: Option<TurnId>) -> String {
    match turn_id {
        Some(id) => format!("Turn {}", short_id(id)),
        None => "Turn (unknown)".to_string(),
    }
}

fn short_id(turn_id: TurnId) -> String {
    let raw = turn_id.0.to_string();
    raw.split('-')
        .next()
        .map(|chunk| chunk.to_string())
        .unwrap_or(raw)
}

fn status_color(colors: ThemeColors, tone: ToolStatusTone) -> gpui::Rgba {
    match tone {
        ToolStatusTone::Pending => colors.muted,
        ToolStatusTone::Running => colors.accent,
        ToolStatusTone::Success => colors.success,
        ToolStatusTone::Failed => colors.error,
        ToolStatusTone::Unknown => colors.muted,
    }
}

fn format_status_label(status: &str) -> String {
    let normalized = status.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "in_progress" | "running" => "Running".to_string(),
        "pending" | "queued" => "Pending".to_string(),
        "completed" | "complete" | "ok" | "succeeded" => "Completed".to_string(),
        "failed" | "error" => "Failed".to_string(),
        "" => "Pending".to_string(),
        other => {
            let mut out = String::new();
            let mut chars = other.chars();
            if let Some(first) = chars.next() {
                out.push(first.to_ascii_uppercase());
                out.push_str(chars.as_str());
            }
            out
        }
    }
}

#[derive(Clone)]
struct ToolLabelParts {
    verb: String,
    rest: String,
}

impl ToolLabelParts {
    fn new(verb: impl Into<String>, rest: impl Into<String>) -> Self {
        let verb = verb.into();
        let rest = rest.into();
        let rest = rest.trim();
        let rest = if rest.is_empty() {
            String::new()
        } else {
            truncate_middle(rest, 140)
        };
        Self { verb, rest }
    }
}

fn tool_label_parts(tool: &TurnToolItem) -> ToolLabelParts {
    let kind = tool.tool_kind.trim().to_ascii_lowercase();
    let title = normalize_label_text(tool.title.as_str());
    if !title.is_empty() && title != "Tool" {
        if let Some(parts) = parse_prefixed(&title, tool_prefixed_verbs()) {
            return parts;
        }
    }

    if let Some(parts) = label_from_parsed_cmd(tool.input.as_ref()) {
        return parts;
    }

    let summary = tool_summary_line(&kind, tool.input.as_ref());
    let verb = tool_verb(&kind);
    if !summary.is_empty() {
        return ToolLabelParts::new(verb, summary);
    }

    if !title.is_empty() && title != "Tool" {
        return ToolLabelParts::new(verb, title);
    }

    ToolLabelParts::new(verb, "")
}

fn tool_prefixed_verbs() -> &'static [&'static str] {
    &[
        "Read",
        "Explored",
        "Searched",
        "Wrote",
        "Edited",
        "Run",
        "Fetch",
        "Search",
        "List",
        "Write",
        "Edit",
    ]
}

fn parse_prefixed(value: &str, verbs: &[&str]) -> Option<ToolLabelParts> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    for verb in verbs {
        if trimmed == *verb {
            return Some(ToolLabelParts::new(*verb, ""));
        }
        if let Some(rest) = trimmed.strip_prefix(&format!("{} ", verb)) {
            return Some(ToolLabelParts::new(*verb, rest));
        }
    }
    None
}

fn label_from_parsed_cmd(input: Option<&Value>) -> Option<ToolLabelParts> {
    let input = input?;
    let cmds = input.get("parsed_cmd")?.as_array()?;
    for cmd in cmds {
        let cmd_type = cmd.get("type").and_then(|value| value.as_str()).unwrap_or("");
        if cmd_type == "list_files" {
            if let Some(path) = cmd.get("path").and_then(|value| value.as_str()) {
                return Some(ToolLabelParts::new("Explored", short_path(path)));
            }
        }
        if cmd_type == "read_file" {
            if let Some(path) = cmd.get("path").and_then(|value| value.as_str()) {
                return Some(ToolLabelParts::new("Read", short_path(path)));
            }
        }
        if cmd_type == "search" {
            let query = cmd
                .get("query")
                .or_else(|| cmd.get("pattern"))
                .or_else(|| cmd.get("regex"))
                .or_else(|| cmd.get("text"))
                .and_then(|value| value.as_str())
                .unwrap_or("");
            if !query.trim().is_empty() {
                return Some(ToolLabelParts::new(
                    "Searched",
                    truncate_middle(query, 120),
                ));
            }
            if let Some(path) = cmd.get("path").and_then(|value| value.as_str()) {
                return Some(ToolLabelParts::new("Searched", short_path(path)));
            }
            return Some(ToolLabelParts::new("Searched", ""));
        }
    }
    None
}

fn tool_verb(kind: &str) -> String {
    match kind {
        "execute" => "Run".to_string(),
        "search" => "Searched".to_string(),
        "read" | "read_file" => "Read".to_string(),
        "list" | "list_files" => "Explored".to_string(),
        "write" => "Wrote".to_string(),
        "edit" | "apply_patch" => "Edited".to_string(),
        "fetch" | "http" | "curl" => "Fetch".to_string(),
        "error" => "Error".to_string(),
        _ => human_tool_kind(kind),
    }
}

fn tool_summary_line(kind: &str, input: Option<&Value>) -> String {
    let Some(input) = input else {
        return String::new();
    };
    match kind {
        "execute" => {
            let cmd = input
                .get("command")
                .and_then(|value| {
                    if let Some(raw) = value.as_str() {
                        return Some(raw.to_string());
                    }
                    let arr = value.as_array()?;
                    let mut parts = Vec::new();
                    for item in arr {
                        if let Some(text) = item.as_str() {
                            parts.push(text);
                        }
                    }
                    if parts.is_empty() {
                        None
                    } else {
                        Some(parts.join(" "))
                    }
                })
                .unwrap_or_default();
            if cmd.trim().is_empty() {
                String::new()
            } else {
                truncate_middle(&short_command(&cmd), 120)
            }
        }
        "search" => {
            let query = input
                .get("query")
                .or_else(|| input.get("pattern"))
                .or_else(|| input.get("regex"))
                .or_else(|| input.get("text"))
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let query = truncate_middle(query, 120);
            let path = format_tool_path_summary(input);
            if !query.is_empty() && !path.is_empty() {
                truncate_middle(&format!("{query} in {path}"), 120)
            } else if !query.is_empty() {
                query
            } else {
                path
            }
        }
        "list" | "list_files" | "read" | "read_file" => format_tool_path_summary(input),
        "edit" | "write" | "apply_patch" => {
            let path = format_tool_path_summary(input);
            let stats = format_tool_diff_stats(input);
            if !path.is_empty() && !stats.is_empty() {
                format!("{path} {stats}")
            } else if !path.is_empty() {
                path
            } else {
                stats
            }
        }
        "fetch" | "http" | "curl" => {
            let method = input
                .get("method")
                .and_then(|value| value.as_str())
                .unwrap_or("GET")
                .trim()
                .to_ascii_uppercase();
            let url = input
                .get("url")
                .or_else(|| input.get("uri"))
                .or_else(|| input.get("href"))
                .and_then(|value| value.as_str())
                .unwrap_or("");
            if url.trim().is_empty() {
                method
            } else {
                truncate_middle(&format!("{method} {url}"), 120)
            }
        }
        _ => String::new(),
    }
}

fn format_tool_diff_stats(input: &Value) -> String {
    let (added, removed, files) = extract_tool_diff_stats(input).unwrap_or((None, None, None));
    let mut parts = Vec::new();
    if let Some(value) = added.filter(|value| *value > 0) {
        parts.push(format!("+{value}"));
    }
    if let Some(value) = removed.filter(|value| *value > 0) {
        parts.push(format!("-{value}"));
    }
    if parts.is_empty() {
        if let Some(value) = files.filter(|value| *value > 0) {
            parts.push(format!("{value} files"));
        }
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("({})", parts.join(" "))
    }
}

fn extract_tool_diff_stats(input: &Value) -> Option<(Option<i64>, Option<i64>, Option<i64>)> {
    let diff = input.get("diff_stats").or_else(|| input.get("diffStats"));
    let added = diff
        .and_then(|value| value.get("added").or_else(|| value.get("add")))
        .and_then(value_to_i64)
        .or_else(|| input.get("added").and_then(value_to_i64));
    let removed = diff
        .and_then(|value| value.get("removed").or_else(|| value.get("delete")))
        .and_then(value_to_i64)
        .or_else(|| input.get("removed").and_then(value_to_i64));
    let files = diff
        .and_then(|value| value.get("files").or_else(|| value.get("file_count")))
        .and_then(value_to_i64)
        .or_else(|| input.get("files").and_then(value_to_i64));
    if added.is_none() && removed.is_none() && files.is_none() {
        None
    } else {
        Some((added, removed, files))
    }
}

fn format_tool_path_summary(input: &Value) -> String {
    let paths = extract_tool_paths(input);
    if paths.is_empty() {
        return String::new();
    }
    let total = input
        .get("paths_total")
        .and_then(value_to_i64)
        .or_else(|| input.get("pathsTotal").and_then(value_to_i64))
        .map(|value| value as usize)
        .unwrap_or(paths.len());
    let more = total.saturating_sub(1);
    let head = truncate_middle(&paths[0], 120);
    if more > 0 {
        format!("{head} +{more} more")
    } else {
        head
    }
}

fn extract_tool_paths(input: &Value) -> Vec<String> {
    let mut paths = Vec::new();
    let mut seen = HashSet::new();
    let mut push = |value: &str| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return;
        }
        let normalized = normalize_path_text(trimmed);
        if seen.insert(normalized.clone()) {
            paths.push(normalized);
        }
    };

    for key in [
        "path",
        "file",
        "filename",
        "file_path",
        "filePath",
        "filepath",
        "target",
    ] {
        if let Some(text) = input.get(key).and_then(value_to_string) {
            push(&text);
        }
    }

    for key in ["paths", "files", "file_paths", "filePaths"] {
        if let Some(values) = input.get(key).and_then(|value| value.as_array()) {
            for value in values {
                if let Some(text) = value_to_string(value) {
                    push(&text);
                }
            }
        }
    }

    if let Some(cmds) = input.get("parsed_cmd").and_then(|value| value.as_array()) {
        for cmd in cmds {
            if let Some(text) = cmd.get("path").and_then(value_to_string) {
                push(&text);
            }
        }
    }

    paths
}

fn normalize_label_text(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.contains(' ') {
        return trimmed.to_string();
    }
    strip_ctx_worktree_prefix(trimmed)
}

fn normalize_path_text(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let stripped = strip_ctx_worktree_prefix(trimmed);
    short_path(&stripped)
}

fn strip_ctx_worktree_prefix(value: &str) -> String {
    let marker = "/.ctx/worktrees/";
    if let Some(idx) = value.find(marker) {
        let after = &value[idx + marker.len()..];
        let mut parts = after.split('/');
        let _workspace = parts.next();
        let _worktree = parts.next();
        let rest: Vec<&str> = parts.collect();
        if !rest.is_empty() {
            return rest.join("/");
        }
    }
    value.to_string()
}

fn short_path(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let parts: Vec<&str> = trimmed
        .split(|c| c == '/' || c == '\\')
        .filter(|part| !part.is_empty())
        .collect();
    if parts.len() <= 2 {
        trimmed.to_string()
    } else {
        format!("{}/{}", parts[parts.len() - 2], parts[parts.len() - 1])
    }
}

fn short_command(command: &str) -> String {
    let mut cmd = command.trim().to_string();
    if let Some(stripped) = cmd.strip_prefix("/bin/bash -lc ") {
        cmd = stripped.trim().to_string();
    }
    if let Some(stripped) = cmd.strip_prefix("bash -lc ") {
        cmd = stripped.trim().to_string();
    }
    if let Some(stripped) = cmd.strip_prefix("set -euo pipefail") {
        cmd = stripped.trim_start_matches([';', ' ']).to_string();
    }
    cmd = cmd.trim_matches('"').to_string();
    if cmd.contains(' ') {
        cmd
    } else {
        strip_ctx_worktree_prefix(&cmd)
    }
}

fn tool_input_preview(tool: &TurnToolItem) -> Option<String> {
    let input = tool.input.as_ref()?;
    let kind = tool.tool_kind.trim().to_ascii_lowercase();
    let summary = tool_summary_line(&kind, Some(input));
    if !summary.is_empty() {
        return Some(summary);
    }
    let raw = serde_json::to_string(input).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(truncate_middle(trimmed, 160))
    }
}

fn tool_output_preview(tool: &TurnToolItem) -> Option<String> {
    let output = tool.output_text.as_ref()?;
    let trimmed = output.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(truncate_middle(&trimmed.replace(['\n', '\r'], " "), 180))
    }
}

fn truncate_middle(text: &str, max_len: usize) -> String {
    let s = text.trim();
    let len = s.chars().count();
    if len <= max_len {
        return s.to_string();
    }
    let head = std::cmp::max(10, (max_len as f32 * 0.6) as usize);
    let tail = std::cmp::max(10, max_len.saturating_sub(head + 3));
    let head_text: String = s.chars().take(head).collect();
    let tail_text: String = s.chars().rev().take(tail).collect::<Vec<_>>().into_iter().rev().collect();
    format!("{head_text}...{tail_text}")
}

fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn value_to_i64(value: &Value) -> Option<i64> {
    if let Some(value) = value.as_i64() {
        return Some(value);
    }
    if let Some(value) = value.as_u64() {
        return Some(value as i64);
    }
    if let Some(value) = value.as_f64() {
        if value.is_finite() {
            return Some(value.round() as i64);
        }
    }
    if let Some(text) = value.as_str() {
        return text.trim().parse::<i64>().ok();
    }
    None
}
