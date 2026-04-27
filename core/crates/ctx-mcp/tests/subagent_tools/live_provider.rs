use super::*;

#[tokio::test]
#[ignore = "Requires live provider credentials and a built ctx-mcp binary on the product path."]
async fn live_provider_parent_can_invoke_real_agent_via_ctx_mcp() {
    let provider_id = std::env::var("CTX_LIVE_PROVIDER_ID").ok();
    let model_id = std::env::var("CTX_LIVE_MODEL_ID").ok();
    if provider_id.is_none() || model_id.is_none() {
        eprintln!(
            "skipping: set CTX_LIVE_PROVIDER_ID and CTX_LIVE_MODEL_ID to run live subagent canary"
        );
        return;
    }
    let provider_id = provider_id.unwrap();
    let model_id = model_id.unwrap();
    if !matches!(provider_id.as_str(), "codex" | "claude" | "claude-crp") {
        eprintln!("skipping: live subagent canary only supports codex/claude providers");
        return;
    }

    std::env::set_var("CTX_MCP_COMMAND", mcp_bin());

    let repo = init_git_repo().await;
    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");

    let adapter: Arc<dyn ProviderAdapter> = match provider_id.as_str() {
        "codex" => Arc::new(ctx_providers::crp::Tier1CrpAdapter::codex()),
        "claude" | "claude-crp" => Arc::new(ctx_providers::crp::Tier1CrpAdapter::claude()),
        _ => unreachable!(),
    };
    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    providers.insert(provider_id.clone(), adapter);

    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores.clone(),
        providers,
        base_url.clone(),
        None,
    ));
    let app = ctx_http::api::router(state.clone());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = reqwest::Client::new();
    let workspace: Workspace = client
        .post(format!("{base_url}/api/workspaces"))
        .json(&json!({
            "root_path": repo.path().to_string_lossy(),
            "name": "ws"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let task: Task = client
        .post(format!(
            "{base_url}/api/workspaces/{}/tasks",
            workspace.id.0
        ))
        .json(&json!({ "title": "t1" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let session: Session = client
        .post(format!("{base_url}/api/tasks/{}/sessions", task.id.0))
        .json(&json!({ "provider_id": provider_id, "model_id": model_id }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let token = format!("CTX_SUBAGENT_LIVE_OK_{}", Uuid::new_v4());
    client
        .post(format!("{base_url}/api/sessions/{}/messages", session.id.0))
        .json(&json!({
            "content": format!(
                "Use ctx.spawn_agent to launch exactly one agent labeled ping. Ask it to reply with exactly {token}. Read the returned agent.agent.agent_id, then use ctx.wait_agent with that agent_id. After the child agent completes, reply with exactly {token} and nothing else."
            )
        }))
        .send()
        .await
        .unwrap();

    let store = state.store_for_session(session.id).await.unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(240);
    loop {
        let events = store.list_session_events(session.id).await.unwrap();
        if events
            .iter()
            .any(|event| matches!(event.event_type, ctx_core::models::SessionEventType::Done))
        {
            break;
        }
        if events.iter().any(|event| {
            matches!(
                event.event_type,
                ctx_core::models::SessionEventType::Error
                    | ctx_core::models::SessionEventType::AuthRequired
            )
        }) {
            panic!("live subagent canary saw terminal error/auth-required events: {events:#?}");
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("timed out waiting for live subagent canary completion");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    let subagents = store.list_subagent_sessions(session.id).await.unwrap();
    assert!(
        !subagents.is_empty(),
        "expected live provider to create at least one subagent session"
    );

    let events = store.list_session_events(session.id).await.unwrap();
    let assistant_messages = events
        .iter()
        .filter(|event| {
            matches!(
                event.event_type,
                ctx_core::models::SessionEventType::AssistantMessageInserted
            )
        })
        .filter_map(|event| {
            event
                .payload_json
                .get("content")
                .and_then(|value| value.as_str())
        })
        .collect::<Vec<_>>();
    assert!(
        assistant_messages
            .iter()
            .any(|message| message.contains(&token)),
        "expected final assistant message containing {token}; saw {assistant_messages:#?}"
    );
}
