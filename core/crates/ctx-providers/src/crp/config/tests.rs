use super::*;
use base64::Engine as _;
use std::fs;
use std::path::PathBuf;

#[test]
fn probe_timeout_for_env_defaults_to_host_timeout() {
    let env = HashMap::<String, String>::new();
    assert_eq!(
        probe_timeout_for_env(&env, Duration::from_secs(10), Duration::from_secs(45)),
        Duration::from_secs(10)
    );
}

#[test]
fn probe_timeout_for_env_uses_container_timeout_when_container_exec_is_present() {
    let mut env = HashMap::<String, String>::new();
    env.insert(
        "CTX_HARNESS_CONTAINER_ID".to_string(),
        "ctx-workspace-123".to_string(),
    );
    assert_eq!(
        probe_timeout_for_env(&env, Duration::from_secs(10), Duration::from_secs(45)),
        Duration::from_secs(45)
    );
}

#[test]
fn build_crp_session_config_sets_pragmatic_personality_for_codex() {
    let mut env = HashMap::new();
    env.insert("CTX_PROVIDER_ID".to_string(), "codex-crp".to_string());
    let workdir = PathBuf::from("/tmp/workdir");

    let cfg = build_crp_session_config(&env, &workdir).expect("build session config");
    assert_eq!(
        cfg.approval_policy.as_deref(),
        Some(FULL_YOLO_APPROVAL_POLICY)
    );
    assert_eq!(cfg.sandbox_mode.as_deref(), Some(FULL_YOLO_SANDBOX_MODE));
    assert_eq!(cfg.reasoning_trace_enabled, Some(true));
    assert_eq!(cfg.personality.as_deref(), Some("pragmatic"));
}

#[test]
fn build_crp_session_config_omits_personality_for_non_codex() {
    let mut env = HashMap::new();
    env.insert("CTX_PROVIDER_ID".to_string(), "claude-crp".to_string());
    let workdir = PathBuf::from("/tmp/workdir");

    let cfg = build_crp_session_config(&env, &workdir).expect("build session config");
    assert_eq!(
        cfg.approval_policy.as_deref(),
        Some(FULL_YOLO_APPROVAL_POLICY)
    );
    assert_eq!(cfg.sandbox_mode.as_deref(), Some(FULL_YOLO_SANDBOX_MODE));
    assert_eq!(cfg.personality, None);
}

#[test]
fn build_crp_session_config_can_disable_model_override() {
    let mut env = HashMap::new();
    env.insert("CTX_MODEL_ID".to_string(), "openai/gpt-4.1-mini".to_string());
    env.insert(
        "CTX_CRP_DISABLE_MODEL_OVERRIDE".to_string(),
        "1".to_string(),
    );
    let workdir = PathBuf::from("/tmp/workdir");

    let cfg = build_crp_session_config(&env, &workdir).expect("build session config");
    assert_eq!(cfg.model, None);
    assert_eq!(cfg.reasoning_effort, None);
}

#[test]
fn build_crp_session_config_uses_default_ctx_mcp_for_container_when_override_is_host_only() {
    let mut env = HashMap::new();
    env.insert(
        "CTX_HARNESS_CONTAINER_ID".to_string(),
        "ctx-harness-123".to_string(),
    );
    env.insert(
        "CTX_MCP_COMMAND".to_string(),
        "/Users/example-user/.cache/cargo/ctx-monorepo/debug/ctx-mcp".to_string(),
    );

    let cfg = build_crp_session_config(&env, Path::new("/ctx/ws")).expect("build session config");
    let command = cfg
        .mcp_servers
        .as_ref()
        .and_then(|servers| servers.get("ctx"))
        .and_then(|server| server.command.as_deref());
    assert_eq!(command, Some("ctx-mcp"));
}

#[test]
fn build_crp_session_config_preserves_existing_absolute_ctx_mcp_for_container() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let mcp_path = tempdir.path().join("ctx-mcp");
    fs::write(&mcp_path, b"#!/bin/sh\n").expect("write mcp");

    let mut env = HashMap::new();
    env.insert(
        "CTX_HARNESS_CONTAINER_ID".to_string(),
        "ctx-harness-123".to_string(),
    );
    env.insert(
        "CTX_MCP_COMMAND".to_string(),
        mcp_path.to_string_lossy().to_string(),
    );

    let cfg = build_crp_session_config(&env, Path::new("/ctx/ws")).expect("build session config");
    let command = cfg
        .mcp_servers
        .as_ref()
        .and_then(|servers| servers.get("ctx"))
        .and_then(|server| server.command.as_deref());
    assert_eq!(command, Some(mcp_path.to_string_lossy().as_ref()));
}

#[test]
fn build_crp_model_probe_config_forces_full_yolo_policy() {
    let workdir = PathBuf::from("/tmp/workdir");

    let cfg =
        build_crp_model_probe_config(&HashMap::new(), &workdir).expect("build model probe config");
    assert_eq!(
        cfg.approval_policy.as_deref(),
        Some(FULL_YOLO_APPROVAL_POLICY)
    );
    assert_eq!(cfg.sandbox_mode.as_deref(), Some(FULL_YOLO_SANDBOX_MODE));
}

#[test]
fn build_crp_session_config_maps_container_thread_cwd_to_guest_worktree() {
    let mut env = HashMap::new();
    env.insert(
        "CTX_HARNESS_CONTAINER_ID".to_string(),
        "ctx-harness-123".to_string(),
    );
    env.insert(
        "CTX_HARNESS_HOST_WORKTREE_ROOT".to_string(),
        "/Users/example-user/code/repo".to_string(),
    );
    env.insert(
        "CTX_HARNESS_GUEST_WORKTREE_ROOT".to_string(),
        "/ctx/ws/worktrees/wt-123".to_string(),
    );
    env.insert(
        "CTX_HARNESS_GUEST_WORKSPACE_ROOT".to_string(),
        "/ctx/ws".to_string(),
    );
    let workdir = PathBuf::from("/Users/example-user/code/repo/src");

    let cfg = build_crp_session_config(&env, &workdir).expect("build session config");
    assert_eq!(cfg.cwd, Some(PathBuf::from("/ctx/ws/worktrees/wt-123/src")));
    assert_eq!(cfg.spawn_cwd, Some(workdir));
}

#[test]
fn synthetic_cline_models_probe_uses_openai_model() {
    let mut env = HashMap::new();
    env.insert(
        "OPENAI_MODEL".to_string(),
        "openai/gpt-5.2-codex".to_string(),
    );
    let probe = synthetic_models_probe_for_provider("cline", &env).expect("cline synthetic probe");
    assert_eq!(
        probe.current_model_id.as_deref(),
        Some("openai/gpt-5.2-codex")
    );
    assert_eq!(probe.models.len(), 1);
    assert_eq!(probe.models[0].id, "openai/gpt-5.2-codex");
}

#[test]
fn synthetic_models_probe_is_provider_scoped() {
    let env = HashMap::new();
    assert!(synthetic_models_probe_for_provider("qwen", &env).is_none());
}

#[test]
fn synthetic_cline_models_probe_requires_openai_model() {
    let env = HashMap::new();
    assert!(synthetic_models_probe_for_provider("cline", &env).is_none());
}

#[test]
fn flatten_prompt_items_as_text_joins_text_blocks_in_order() {
    let items = vec![
        json!({"type":"text","text":"system"}),
        json!({"type":"text","text":"user"}),
    ];
    assert_eq!(
        flatten_prompt_items_as_text(&items).expect("flattened prompt"),
        "system\n\nuser"
    );
}

#[test]
fn flatten_prompt_items_as_text_rejects_non_text_items() {
    let items = vec![json!({"type":"image","image_url":"data:image/png;base64,AAAA"})];
    let err = flatten_prompt_items_as_text(&items).expect_err("image item should fail");
    assert!(err
        .to_string()
        .contains("provider requires text-only ACP prompt items"));
}

#[tokio::test]
async fn build_prompt_items_emits_inline_images_as_bytes_items() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut env = HashMap::new();
    env.insert(
        "CTX_DATA_ROOT".to_string(),
        temp.path().to_string_lossy().to_string(),
    );
    let bytes = vec![1u8, 2, 3, 4];
    let input = TurnInput {
        content: "describe image".to_string(),
        context_blocks: Vec::new(),
        attachments: vec![ctx_core::models::MessageAttachment::Image {
            mime_type: "image/png".to_string(),
            data_base64: base64::engine::general_purpose::STANDARD.encode(&bytes),
            name: Some("inline.png".to_string()),
        }],
        model_id: None,
    };

    let items = build_prompt_items(&input, &PathBuf::from("."), &env)
        .await
        .expect("inline image should be emitted as CRP image item");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].get("type").and_then(Value::as_str), Some("image"));
    assert_eq!(
        items[0].get("mime_type").and_then(Value::as_str),
        Some("image/png")
    );
    assert_eq!(
        items[0].get("data").and_then(Value::as_str),
        Some(
            base64::engine::general_purpose::STANDARD
                .encode(&bytes)
                .as_str()
        )
    );
}

#[tokio::test]
async fn build_prompt_items_emits_blob_refs_as_image_refs() {
    let host_root = tempfile::tempdir().expect("host tempdir");
    let runtime_root = tempfile::tempdir().expect("runtime tempdir");
    let blob_dir = host_root.path().join("blobs");
    tokio::fs::create_dir_all(&blob_dir)
        .await
        .expect("create blob dir");
    let blob_id = "blob-123";
    let blob_path = blob_dir.join(blob_id);
    let bytes = vec![9u8, 8, 7, 6];
    tokio::fs::write(&blob_path, &bytes)
        .await
        .expect("write blob");

    let mut env = HashMap::new();
    env.insert(
        "CTX_DATA_ROOT_HOST".to_string(),
        host_root.path().to_string_lossy().to_string(),
    );
    env.insert(
        "CTX_DATA_ROOT".to_string(),
        runtime_root.path().to_string_lossy().to_string(),
    );

    let input = TurnInput {
        content: "describe image".to_string(),
        context_blocks: Vec::new(),
        attachments: vec![ctx_core::models::MessageAttachment::ImageRef {
            blob_id: blob_id.to_string(),
            mime_type: "image/png".to_string(),
            name: Some("blob.png".to_string()),
        }],
        model_id: None,
    };

    let items = build_prompt_items(&input, &PathBuf::from("."), &env)
        .await
        .expect("blob image should be emitted as image_ref");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].get("type").and_then(Value::as_str), Some("image_ref"));
    assert_eq!(items[0].get("blob_id").and_then(Value::as_str), Some(blob_id));
    assert_eq!(
        items[0].get("mime_type").and_then(Value::as_str),
        Some("image/png")
    );
}
