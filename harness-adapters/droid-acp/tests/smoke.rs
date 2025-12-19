use droid_acp::events::{parse_event, DroidEvent};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

#[tokio::test]
#[ignore]
async fn droid_exec_stream_smoke() {
    if std::env::var("FACTORY_API_KEY").is_err() {
        return;
    }

    let Ok(droid_path) = which::which("droid") else {
        return;
    };

    let mut child = Command::new(droid_path)
        .arg("exec")
        .arg("--output-format")
        .arg("stream-json")
        .arg("hello")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn droid exec");

    let stdout = child.stdout.take().expect("stdout");
    let mut lines = BufReader::new(stdout).lines();

    let mut saw_completion = false;
    while let Ok(Some(line)) = lines.next_line().await {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let parsed: serde_json::Value = serde_json::from_str(trimmed)
            .expect("valid json");
        if let Some(envelope) = parse_event(parsed) {
            if let Some(DroidEvent::Completion { .. }) = envelope.event {
                saw_completion = true;
                break;
            }
        }
    }

    let _ = child.wait().await;
    assert!(saw_completion, "expected completion event");
}
