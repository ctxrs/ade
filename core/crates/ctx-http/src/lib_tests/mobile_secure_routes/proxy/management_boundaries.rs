use super::*;

#[tokio::test]
async fn mobile_secure_proxy_rejects_mobile_management_paths_after_trimming() {
    let _serial = home_env_test_lock().lock().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let (app, state, device_id, key, _data_dir) = build_mobile_secure_proxy_app(true).await;
    let target_device_id = "55555555-5555-5555-5555-555555555555";
    let pairing_token = "pairing-token-through-secure-proxy";
    let mut hasher = sha2::Sha256::new();
    hasher.update(pairing_token.as_bytes());
    let token_hash = hex::encode(hasher.finalize());
    state
        .global_store()
        .insert_mobile_pairing_token(
            "pair-smuggle",
            &token_hash,
            chrono::Utc::now() + chrono::Duration::minutes(5),
        )
        .await
        .unwrap();

    let res = post_mobile_secure_request(
        &app,
        &device_id,
        &key,
        1,
        json!({
            "method": "POST",
            "path": " /api/mobile/pair",
            "headers": [["content-type", "application/json"]],
            "body_b64": base64::engine::general_purpose::STANDARD.encode(
                json!({
                    "pairing_token": pairing_token,
                    "device_id": target_device_id,
                    "device_label": "smuggled-device",
                    "platform": "ios",
                    "public_key": "smuggled-public-key"
                })
                .to_string()
            )
        }),
    )
    .await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        payload["error"],
        "secure proxy cannot target mobile management endpoints"
    );
    assert!(
        state
            .global_store()
            .get_mobile_device(MobileDeviceId(
                uuid::Uuid::parse_str(target_device_id).unwrap()
            ))
            .await
            .unwrap()
            .is_none(),
        "mobile secure proxy unexpectedly registered a device through a trimmed management path"
    );

    let res = post_mobile_secure_request(
        &app,
        &device_id,
        &key,
        2,
        json!({
            "method": "POST",
            "path": "/api/%6dobile/pair",
            "headers": [["content-type", "application/json"]],
            "body_b64": base64::engine::general_purpose::STANDARD.encode(
                json!({
                    "pairing_token": pairing_token,
                    "device_id": target_device_id,
                    "device_label": "encoded-smuggled-device",
                    "platform": "ios",
                    "public_key": "smuggled-public-key"
                })
                .to_string()
            )
        }),
    )
    .await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["error"], "secure proxy path must be normalized");
    assert!(
        state
            .global_store()
            .get_mobile_device(MobileDeviceId(
                uuid::Uuid::parse_str(target_device_id).unwrap()
            ))
            .await
            .unwrap()
            .is_none(),
        "mobile secure proxy unexpectedly registered a device through an encoded management path"
    );
}

#[tokio::test]
async fn mobile_secure_proxy_rejects_repo_path_management_routes() {
    let _serial = home_env_test_lock().lock().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let (app, state, device_id, key, _data_dir) = build_mobile_secure_proxy_app(true).await;
    let sandbox = tempfile::tempdir().unwrap();
    let clone_parent = sandbox.path().join("mobile-clone-parent");
    let init_path = sandbox.path().join("mobile-init-target");
    let existing_repo = setup_git_repo().await;
    let staging_root = state.core.data_root.join("workspaces").join("staging");

    let cases = [
        (
            "POST",
            "/api/repo/clone",
            Some(json!({
                "repo_url": "https://example.com/org/repo.git",
                "dest_parent": clone_parent.to_string_lossy(),
                "dest_name": "repo"
            })),
        ),
        (
            "POST",
            "/api/repo/init",
            Some(json!({
                "path": init_path.to_string_lossy()
            })),
        ),
        (
            "POST",
            "/api/repo/status",
            Some(json!({
                "path": existing_repo.path().to_string_lossy()
            })),
        ),
        (
            "POST",
            "/api/repo/validate_destination",
            Some(json!({
                "path": existing_repo.path().to_string_lossy()
            })),
        ),
        ("GET", "/api/repo/staging_path", None),
    ];

    for (index, (method, path, body)) in cases.into_iter().enumerate() {
        let mut payload = json!({
            "method": method,
            "path": path,
            "headers": [],
        });
        if let Some(body) = body {
            payload["headers"] = json!([["content-type", "application/json"]]);
            payload["body_b64"] =
                json!(base64::engine::general_purpose::STANDARD.encode(body.to_string()));
        }
        let res =
            post_mobile_secure_request(&app, &device_id, &key, index as i64 + 1, payload).await;
        assert_eq!(
            res.status(),
            StatusCode::OK,
            "{method} {path} outer secure response"
        );

        let payload = decode_mobile_secure_response(res, &device_id, &key).await;
        assert_eq!(payload["status"], 401, "{method} {path} proxied status");

        let body_bytes = base64::engine::general_purpose::STANDARD
            .decode(payload["body_b64"].as_str().unwrap())
            .unwrap();
        let body_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(body_json["error"], "desktop auth required");
    }

    assert!(
        !clone_parent.exists(),
        "mobile secure proxy unexpectedly created clone parent path"
    );
    assert!(
        !init_path.exists(),
        "mobile secure proxy unexpectedly created init path"
    );
    assert!(
        !staging_root.exists(),
        "mobile secure proxy unexpectedly created repo staging path"
    );
}

#[tokio::test]
async fn mobile_secure_proxy_rejects_daemon_maintenance_routes() {
    let _serial = home_env_test_lock().lock().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let (app, _state, device_id, key, _data_dir) = build_mobile_secure_proxy_app(true).await;

    let cases = [
        ("POST", "/api/execution/linux_sandbox_runtime/prepare"),
        ("POST", "/api/logs/open"),
        ("POST", "/api/updates/appimage/apply"),
    ];

    for (index, (method, path)) in cases.into_iter().enumerate() {
        let res = post_mobile_secure_request(
            &app,
            &device_id,
            &key,
            index as i64 + 1,
            json!({
                "method": method,
                "path": path,
                "headers": []
            }),
        )
        .await;
        if res.status() != StatusCode::OK {
            let status = res.status();
            let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
            panic!(
                "{method} {path} outer secure response was {status}: {}",
                String::from_utf8_lossy(&body)
            );
        }

        let payload = decode_mobile_secure_response(res, &device_id, &key).await;
        assert_eq!(payload["status"], 401, "{method} {path} proxied status");
        let body_bytes = base64::engine::general_purpose::STANDARD
            .decode(payload["body_b64"].as_str().unwrap())
            .unwrap();
        let body_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(body_json["error"], "desktop auth required");
    }
}
