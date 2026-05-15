use super::*;

#[tokio::test]
async fn provider_verify_surfaces_agent_server_config_errors() {
    let git_repo = setup_git_repo().await;
    let fixture = ProviderRouteFixture::new().await;
    write_invalid_agent_server_config(fixture.data_root());
    let app = fixture.app();
    let workspace = create_workspace_via_api(&app, &git_repo.path().to_string_lossy()).await;

    let req = Request::builder()
        .method("POST")
        .uri(format!(
            "/api/workspaces/{}/providers/qwen/verify",
            workspace.id.0
        ))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["status"].as_str(), Some("error"));
    assert!(payload["message"]
        .as_str()
        .is_some_and(|value| value.contains("parsing agent server config")));
}

#[tokio::test]
async fn provider_get_surfaces_agent_server_config_errors_in_status() {
    let fixture = ProviderRouteFixture::new().await;
    write_invalid_agent_server_config(fixture.data_root());
    let app = fixture.app();

    let req = Request::builder()
        .method("GET")
        .uri("/api/providers/qwen")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["health"].as_str(), Some("error"));
    assert_eq!(
        payload
            .pointer("/usability/reason_code")
            .and_then(|v| v.as_str()),
        Some("managed_config_error")
    );
    assert!(payload["diagnostics"]
        .as_array()
        .is_some_and(|values| values.iter().any(|value| value
            .as_str()
            .is_some_and(|message| message.contains("parsing agent server config")))));
}

#[tokio::test]
async fn provider_bootstrap_marks_statuses_with_agent_server_config_errors() {
    let git_repo = setup_git_repo().await;
    let fixture = ProviderRouteFixture::new().await;
    write_invalid_agent_server_config(fixture.data_root());
    let app = fixture.app();
    let workspace = create_workspace_via_api(&app, &git_repo.path().to_string_lossy()).await;

    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/workspaces/{}/providers/bootstrap",
            workspace.id.0
        ))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let qwen = payload["providers"]
        .as_array()
        .and_then(|providers| {
            providers
                .iter()
                .find(|provider| provider["provider_id"].as_str() == Some("qwen"))
        })
        .expect("qwen bootstrap status");
    assert_eq!(qwen["health"].as_str(), Some("error"));
    assert_eq!(
        qwen.pointer("/usability/reason_code")
            .and_then(|v| v.as_str()),
        Some("managed_config_error")
    );
}

#[tokio::test]
async fn provider_authenticate_surfaces_agent_server_config_errors() {
    let git_repo = setup_git_repo().await;
    let fixture = ProviderRouteFixture::new().await;
    write_invalid_agent_server_config(fixture.data_root());
    let app = fixture.app();
    let workspace = create_workspace_via_api(&app, &git_repo.path().to_string_lossy()).await;

    let req = Request::builder()
        .method("POST")
        .uri(format!(
            "/api/workspaces/{}/providers/qwen/authenticate",
            workspace.id.0
        ))
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["status"].as_str(), Some("error"));
    assert_eq!(payload["auth_required"].as_bool(), Some(false));
    assert!(payload["message"]
        .as_str()
        .is_some_and(|value| value.contains("parsing agent server config")));
}
