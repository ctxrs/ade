use super::*;

#[tokio::test]
async fn mcp_subagent_tools_work_end_to_end_against_real_daemon_router() {
    let (_repo, _data_dir, _state, base_url, parent_id) =
        setup_daemon_backed_parent_session().await;

    let mut child = Command::new(mcp_bin())
        .arg("--stdio")
        .env("CTX_DAEMON_URL", &base_url)
        .env("CTX_SESSION_ID", &parent_id)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();

    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout).lines();

    for msg in [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25"}}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
        json!({
            "jsonrpc":"2.0",
            "id":3,
            "method":"tools/call",
            "params":{
                "name":"ctx.subagent_init",
                "arguments":{
                    "worktree":"inherit",
                    "agents":[
                        {
                            "prompt":"reply with exactly OK",
                            "label":"Audit FooAPI",
                            "harness":"fake",
                            "model":"fake-model"
                        }
                    ]
                }
            }
        }),
    ] {
        stdin.write_all(msg.to_string().as_bytes()).await.unwrap();
        stdin.write_all(b"\n").await.unwrap();
    }
    stdin.flush().await.unwrap();

    let init_response = wait_for_response(&mut reader, 3).await;
    let init_text = init_response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or("");
    let init_payload: serde_json::Value = serde_json::from_str(init_text).unwrap();
    assert_eq!(init_payload["status"], "running");
    assert_eq!(init_payload["results"][0]["label"], "Audit FooAPI");
    assert_eq!(init_payload["results"][0]["status"], "running");

    stdin
        .write_all(
            json!({
                "jsonrpc":"2.0",
                "id":4,
                "method":"tools/call",
                "params":{
                    "name":"ctx.subagent_wait",
                    "arguments":{
                        "label":"Audit FooAPI"
                    }
                }
            })
            .to_string()
            .as_bytes(),
        )
        .await
        .unwrap();
    stdin.write_all(b"\n").await.unwrap();
    stdin.flush().await.unwrap();

    let wait_response = wait_for_response(&mut reader, 4).await;
    let wait_text = wait_response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or("");
    let wait_payload: serde_json::Value = serde_json::from_str(wait_text).unwrap();
    assert_eq!(wait_payload["status"], "completed");
    assert_eq!(wait_payload["results"][0]["label"], "Audit FooAPI");
    assert_eq!(wait_payload["results"][0]["status"], "completed");
    assert!(wait_payload["results"][0]["content"]
        .as_str()
        .unwrap_or("")
        .contains("done: reply with exactly OK"));

    let _ = child.kill().await;
}
