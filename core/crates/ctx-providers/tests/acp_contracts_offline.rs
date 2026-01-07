use std::collections::HashSet;
use std::path::PathBuf;

use ctx_core::models::SessionEventType;
use ctx_providers::acp::transcript::{load_transcript, replay_transcript};
use ctx_providers::events::NormalizedEvent;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("acp_transcripts")
}

fn tool_call_id(ev: &NormalizedEvent) -> Option<String> {
    ev.payload_json
        .get("tool_call_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .filter(|s| !s.trim().is_empty())
}

fn done_status(events: &[NormalizedEvent]) -> Option<String> {
    events.iter().find_map(|ev| {
        if matches!(ev.event_type, SessionEventType::Done) {
            ev.payload_json
                .get("status")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        } else {
            None
        }
    })
}

fn assistant_fragments_concat(events: &[NormalizedEvent]) -> String {
    let mut buf = String::new();
    for ev in events {
        if matches!(ev.event_type, SessionEventType::AssistantChunk) {
            if let Some(frag) = ev
                .payload_json
                .get("content_fragment")
                .and_then(|v| v.as_str())
            {
                buf.push_str(frag);
            }
        }
    }
    buf
}

fn assistant_complete_content(events: &[NormalizedEvent]) -> Option<String> {
    events.iter().find_map(|ev| {
        if matches!(ev.event_type, SessionEventType::AssistantComplete) {
            ev.payload_json
                .get("full_content")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        } else {
            None
        }
    })
}

fn assert_success_invariants(events: &[NormalizedEvent]) {
    let done: Vec<_> = events
        .iter()
        .filter(|e| matches!(e.event_type, SessionEventType::Done))
        .collect();
    assert_eq!(done.len(), 1, "expected exactly one Done event");
    assert_eq!(
        done_status(events).as_deref(),
        Some("success"),
        "expected Done status=success"
    );

    let tool_calls: HashSet<String> = events
        .iter()
        .filter(|e| matches!(e.event_type, SessionEventType::ToolCall))
        .filter_map(tool_call_id)
        .collect();
    let tool_results: HashSet<String> = events
        .iter()
        .filter(|e| matches!(e.event_type, SessionEventType::ToolResult))
        .filter_map(tool_call_id)
        .collect();
    assert!(
        !tool_calls.is_empty(),
        "expected at least one ToolCall, got {events:#?}"
    );
    assert!(
        !tool_results.is_empty(),
        "expected at least one ToolResult, got {events:#?}"
    );
    let correlated = tool_calls.intersection(&tool_results).next().cloned();
    assert!(
        correlated.is_some(),
        "expected at least one correlated ToolCall/ToolResult pair; calls={tool_calls:?} results={tool_results:?}\nfull events={events:#?}"
    );

    let concat = assistant_fragments_concat(events);
    if !concat.trim().is_empty() {
        let complete = assistant_complete_content(events);
        assert_eq!(
            complete.as_deref(),
            Some(concat.as_str()),
            "AssistantComplete.full_content must match concatenated AssistantChunk fragments"
        );
    }
}

fn assert_error_invariants(events: &[NormalizedEvent]) {
    let done: Vec<_> = events
        .iter()
        .filter(|e| matches!(e.event_type, SessionEventType::Done))
        .collect();
    assert_eq!(done.len(), 1, "expected exactly one Done event");
    assert_eq!(
        done_status(events).as_deref(),
        Some("error"),
        "expected Done status=error"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e.event_type, SessionEventType::Error)),
        "expected at least one Error event, got {events:#?}"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e.event_type, SessionEventType::AuthRequired)),
        "expected at least one AuthRequired event, got {events:#?}"
    );

    // Error fixtures still include a tool call; ensure ToolResult is emitted for failed updates.
    let tool_calls: HashSet<String> = events
        .iter()
        .filter(|e| matches!(e.event_type, SessionEventType::ToolCall))
        .filter_map(tool_call_id)
        .collect();
    let tool_results: HashSet<String> = events
        .iter()
        .filter(|e| matches!(e.event_type, SessionEventType::ToolResult))
        .filter_map(tool_call_id)
        .collect();
    let correlated = tool_calls.intersection(&tool_results).next().cloned();
    assert!(
        correlated.is_some(),
        "expected correlated ToolCall/ToolResult even in error case; calls={tool_calls:?} results={tool_results:?}\nfull events={events:#?}"
    );
}

#[test]
fn offline_acp_contracts_success_transcripts() {
    let fixtures = [
        "codex_success.json",
        "claude_success.json",
        "gemini_success.json",
        "cursor_success.json",
    ];
    for name in fixtures {
        let transcript = load_transcript(fixtures_dir().join(name)).unwrap();
        let events = replay_transcript(&transcript).unwrap();
        assert_success_invariants(&events);
    }
}

#[test]
fn offline_acp_contracts_error_transcripts() {
    let fixtures = [
        "codex_error.json",
        "claude_error.json",
        "gemini_error.json",
        "cursor_error.json",
    ];
    for name in fixtures {
        let transcript = load_transcript(fixtures_dir().join(name)).unwrap();
        let events = replay_transcript(&transcript).unwrap();
        assert_error_invariants(&events);
    }
}
