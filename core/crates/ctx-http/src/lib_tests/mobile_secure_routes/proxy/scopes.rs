use super::*;

#[tokio::test]
async fn mobile_secure_proxy_grants_mobile_auth_for_proxied_api_routes() {
    let _serial = home_env_test_lock().lock().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let (app, _daemon, device_id, key, _data_dir) = build_mobile_secure_proxy_app(true).await;
    let res = post_mobile_secure_request(
        &app,
        &device_id,
        &key,
        1,
        json!({
            "method": "GET",
            "path": "/api/workspaces",
            "headers": []
        }),
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);

    let payload = decode_mobile_secure_response(res, &device_id, &key).await;
    assert_eq!(payload["status"], 200);
    let body_bytes = base64::engine::general_purpose::STANDARD
        .decode(payload["body_b64"].as_str().unwrap())
        .unwrap();
    let body_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(body_json, json!([]));
}

#[tokio::test]
async fn mobile_secure_proxy_rejects_profiles_without_workspace_read_scope() {
    let _serial = home_env_test_lock().lock().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let (app, _daemon, device_id, key, _data_dir) =
        build_mobile_secure_proxy_app_with_scopes(true, &["device_registration"]).await;
    let res = post_mobile_secure_request(
        &app,
        &device_id,
        &key,
        1,
        json!({
            "method": "GET",
            "path": "/api/workspaces",
            "headers": []
        }),
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);

    let payload = decode_mobile_secure_response(res, &device_id, &key).await;
    assert_eq!(payload["status"], 401);
    let body_bytes = base64::engine::general_purpose::STANDARD
        .decode(payload["body_b64"].as_str().unwrap())
        .unwrap();
    let body_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(
        body_json["error"],
        "mobile profile lacks workspace_read scope"
    );
}

#[tokio::test]
async fn mobile_secure_proxy_migrates_legacy_empty_scope_profiles() {
    let _serial = home_env_test_lock().lock().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let (app, daemon, device_id, key, _data_dir) =
        build_mobile_secure_proxy_app_with_scopes(true, &[]).await;
    let res = post_mobile_secure_request(
        &app,
        &device_id,
        &key,
        1,
        json!({
            "method": "GET",
            "path": "/api/workspaces",
            "headers": []
        }),
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);

    let payload = decode_mobile_secure_response(res, &device_id, &key).await;
    assert_eq!(payload["status"], 200);

    let cfg = daemon
        .mobile_access_for_test()
        .mobile_access_config_for_test()
        .await
        .unwrap()
        .expect("mobile access config should exist");
    let profile = daemon
        .mobile_access_for_test()
        .mobile_profile_for_test(cfg.profile_id)
        .await
        .unwrap()
        .expect("mobile profile should still exist");
    assert_eq!(
        profile.scopes,
        vec![
            "device_registration".to_string(),
            "workspace_read".to_string(),
            "workspace_stream".to_string(),
        ]
    );
}
