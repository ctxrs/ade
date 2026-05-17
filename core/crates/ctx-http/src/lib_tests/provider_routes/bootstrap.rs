use super::*;

#[tokio::test]
async fn provider_bootstrap_preserves_invalid_workspace_error() {
    let fixture = ProviderRouteFixture::new().await;
    let app = fixture.app();

    let req = Request::builder()
        .method("GET")
        .uri("/api/workspaces/not-a-uuid/providers/bootstrap")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["error"].as_str(), Some("invalid workspace id"));
}

#[tokio::test]
async fn provider_bootstrap_preserves_missing_workspace_error() {
    let fixture = ProviderRouteFixture::new().await;
    let app = fixture.app();

    let req = Request::builder()
        .method("GET")
        .uri("/api/workspaces/11111111-1111-4111-8111-111111111111/providers/bootstrap")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["error"].as_str(), Some("workspace not found"));
}
