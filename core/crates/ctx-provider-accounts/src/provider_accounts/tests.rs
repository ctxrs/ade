use super::*;

static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
const CLAUDE_TEST_SETUP_TOKEN: &str =
    "sk-ant-oat01-abcDEF1234567890_abcdefghijklmnopqrstuvwxyz_0123456789";

async fn lock_env() -> tokio::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().await
}

struct EnvGuard {
    key: &'static str,
    prev: Option<String>,
}

impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let prev = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, prev }
    }

    fn without(key: &'static str) -> Self {
        let prev = std::env::var(key).ok();
        std::env::remove_var(key);
        Self { key, prev }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        if let Some(value) = self.prev.as_deref() {
            std::env::set_var(self.key, value);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

#[tokio::test]
async fn codex_env_mirrors_active_account_auth_into_runtime_home() {
    let _env_lock = lock_env().await;
    let _guard = EnvGuard::without("CTX_CODEX_HOME");
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let registry = CodexAccountRegistry {
        active_account_id: Some("acct-123".to_string()),
        accounts: vec![CodexAccountEntry {
            id: "acct-123".to_string(),
            label: "Account".to_string(),
            kind: CODEX_CREDENTIAL_KIND_API_KEY.to_string(),
            email: None,
            plan_type: None,
            created_at: Utc::now(),
            last_used_at: None,
            secret_ref: None,
            endpoint_profile: CodexEndpointProfile::default(),
        }],
    };
    save_codex_registry(root, &registry).await.unwrap();
    let account_dir = ensure_codex_account_dir(root, "acct-123").await.unwrap();
    tokio::fs::write(
        account_dir.join("auth.json"),
        br#"{"OPENAI_API_KEY":"test-key"}"#,
    )
    .await
    .unwrap();

    let env = codex_env_for_active_account(root).await.unwrap();
    let home = env.get("CODEX_HOME").unwrap();
    assert_eq!(home, &codex_runtime_home(root).to_string_lossy());
    assert!(codex_runtime_home(root).exists());
    let mirrored = tokio::fs::read_to_string(codex_runtime_home(root).join("auth.json"))
        .await
        .unwrap();
    assert!(mirrored.contains("OPENAI_API_KEY"));
}

#[tokio::test]
async fn codex_env_projects_active_account_auth_into_runtime_root() {
    let _env_lock = lock_env().await;
    let _guard = EnvGuard::without("CTX_CODEX_HOME");
    let dir = tempfile::tempdir().unwrap();
    let runtime_root = tempfile::tempdir().unwrap();
    let root = dir.path();
    let registry = CodexAccountRegistry {
        active_account_id: Some("acct-123".to_string()),
        accounts: vec![CodexAccountEntry {
            id: "acct-123".to_string(),
            label: "Account".to_string(),
            kind: CODEX_CREDENTIAL_KIND_API_KEY.to_string(),
            email: None,
            plan_type: None,
            created_at: Utc::now(),
            last_used_at: None,
            secret_ref: None,
            endpoint_profile: CodexEndpointProfile::default(),
        }],
    };
    save_codex_registry(root, &registry).await.unwrap();
    let account_dir = ensure_codex_account_dir(root, "acct-123").await.unwrap();
    tokio::fs::write(
        account_dir.join("auth.json"),
        br#"{"OPENAI_API_KEY":"test-key"}"#,
    )
    .await
    .unwrap();

    let env = codex_env_for_active_account_with_runtime_root(root, runtime_root.path())
        .await
        .unwrap();
    let home = env.get("CODEX_HOME").unwrap();
    assert_eq!(
        home,
        &codex_runtime_home(runtime_root.path()).to_string_lossy()
    );
    assert!(codex_runtime_home(runtime_root.path()).exists());
    let mirrored =
        tokio::fs::read_to_string(codex_runtime_home(runtime_root.path()).join("auth.json"))
            .await
            .unwrap();
    assert!(mirrored.contains("OPENAI_API_KEY"));
}

#[tokio::test]
async fn codex_env_defaults_to_runtime_home() {
    let _env_lock = lock_env().await;
    let _guard = EnvGuard::without("CTX_CODEX_HOME");
    let _seed_guard = EnvGuard::without(CTX_SEED_CODEX_AUTH_FROM_HOST_ENV);
    let _path_guard = EnvGuard::without(CTX_CODEX_HOST_AUTH_PATH_ENV);
    let dir = tempfile::tempdir().unwrap();

    let env = codex_env_for_active_account(dir.path()).await.unwrap();
    let home = env.get("CODEX_HOME").unwrap();
    assert_eq!(home, &codex_runtime_home(dir.path()).to_string_lossy());
    assert!(codex_runtime_home(dir.path()).exists());
}

#[tokio::test]
async fn ensure_codex_endpoint_runtime_home_from_env_sets_container_accessible_codex_home() {
    let dir = tempfile::tempdir().unwrap();
    let mut env = HashMap::new();
    env.insert("OPENAI_API_KEY".to_string(), "endpoint-key".to_string());
    env.insert(
        "CODEX_HOME".to_string(),
        "/tmp/host/endpoint-homes/abc123".to_string(),
    );

    ensure_codex_endpoint_runtime_home_from_env(dir.path(), &mut env)
        .await
        .unwrap();

    let codex_home = env.get("CODEX_HOME").cloned().unwrap_or_default();
    assert_eq!(codex_home, codex_runtime_home(dir.path()).to_string_lossy());
    let auth_path = Path::new(&codex_home).join("auth.json");
    let auth = tokio::fs::read_to_string(auth_path).await.unwrap();
    assert!(auth.contains("OPENAI_API_KEY"));
    assert!(auth.contains("endpoint-key"));
}

#[tokio::test]
async fn ensure_codex_endpoint_runtime_home_from_env_uses_endpoint_home_auth_when_env_key_missing()
{
    let dir = tempfile::tempdir().unwrap();
    let endpoint_home = dir
        .path()
        .join("providers")
        .join("codex")
        .join("endpoint-homes")
        .join("ep-1");
    tokio::fs::create_dir_all(&endpoint_home).await.unwrap();
    tokio::fs::write(
        endpoint_home.join("auth.json"),
        br#"{"OPENAI_API_KEY":"endpoint-home-key"}"#,
    )
    .await
    .unwrap();
    let mut env = HashMap::new();
    env.insert(
        "CODEX_HOME".to_string(),
        endpoint_home.to_string_lossy().to_string(),
    );

    ensure_codex_endpoint_runtime_home_from_env(dir.path(), &mut env)
        .await
        .unwrap();

    let codex_home = env.get("CODEX_HOME").cloned().unwrap_or_default();
    assert_eq!(codex_home, codex_runtime_home(dir.path()).to_string_lossy());
    assert_eq!(
        env.get("OPENAI_API_KEY").map(String::as_str),
        Some("endpoint-home-key")
    );
    let auth = tokio::fs::read_to_string(Path::new(&codex_home).join("auth.json"))
        .await
        .unwrap();
    assert!(auth.contains("endpoint-home-key"));
}

#[tokio::test]
async fn ensure_codex_endpoint_runtime_home_from_env_errors_without_env_key_or_endpoint_auth() {
    let dir = tempfile::tempdir().unwrap();
    let mut env = HashMap::new();
    let err = ensure_codex_endpoint_runtime_home_from_env(dir.path(), &mut env)
        .await
        .unwrap_err();
    assert!(err.to_string().contains(
        "missing OPENAI_API_KEY and CODEX_HOME while preparing codex endpoint runtime home"
    ));
}

#[tokio::test]
async fn ensure_provider_runtime_home_env_sets_home_and_xdg_dirs_when_missing() {
    let dir = tempfile::tempdir().unwrap();
    let mut env = HashMap::new();

    ensure_provider_runtime_home_env(dir.path(), "opencode", &mut env)
        .await
        .unwrap();

    let home = env.get("HOME").cloned().unwrap_or_default();
    let expected_config = Path::new(&home)
        .join(".config")
        .to_string_lossy()
        .to_string();
    let expected_cache = Path::new(&home)
        .join(".cache")
        .to_string_lossy()
        .to_string();
    let expected_data = Path::new(&home)
        .join(".local/share")
        .to_string_lossy()
        .to_string();
    let expected_state = Path::new(&home)
        .join(".local/state")
        .to_string_lossy()
        .to_string();
    assert_eq!(
        home,
        dir.path()
            .join("providers")
            .join("opencode")
            .join("home")
            .to_string_lossy()
    );
    assert_eq!(
        env.get("XDG_CONFIG_HOME").map(String::as_str),
        Some(expected_config.as_str())
    );
    assert_eq!(
        env.get("XDG_CACHE_HOME").map(String::as_str),
        Some(expected_cache.as_str())
    );
    assert_eq!(
        env.get("XDG_DATA_HOME").map(String::as_str),
        Some(expected_data.as_str())
    );
    assert_eq!(
        env.get("XDG_STATE_HOME").map(String::as_str),
        Some(expected_state.as_str())
    );
}

#[tokio::test]
async fn codex_env_seeds_runtime_home_from_host_when_enabled_without_active_account() {
    let _env_lock = lock_env().await;
    let _guard = EnvGuard::without("CTX_CODEX_HOME");
    let _seed_guard = EnvGuard::set(CTX_SEED_CODEX_AUTH_FROM_HOST_ENV, "1");
    let host = tempfile::tempdir().unwrap();
    let host_auth = host.path().join("auth.json");
    tokio::fs::write(&host_auth, br#"{"OPENAI_API_KEY":"seeded-key"}"#)
        .await
        .unwrap();
    let _path_guard = EnvGuard::set(
        CTX_CODEX_HOST_AUTH_PATH_ENV,
        host_auth.to_string_lossy().as_ref(),
    );
    let dir = tempfile::tempdir().unwrap();

    let env = codex_env_for_active_account(dir.path()).await.unwrap();
    let home = env.get("CODEX_HOME").unwrap();
    assert_eq!(home, &codex_runtime_home(dir.path()).to_string_lossy());
    ensure_codex_auth_ready(Path::new(home)).await.unwrap();
}

#[tokio::test]
async fn codex_env_uses_host_auth_candidate_without_active_account() {
    let _env_lock = lock_env().await;
    let _guard = EnvGuard::without("CTX_CODEX_HOME");
    let _seed_guard = EnvGuard::without(CTX_SEED_CODEX_AUTH_FROM_HOST_ENV);
    let host = tempfile::tempdir().unwrap();
    let host_auth = host.path().join("auth.json");
    tokio::fs::write(
        &host_auth,
        br#"{"tokens":{"access_token":"seeded-access","refresh_token":"seeded-refresh"}}"#,
    )
    .await
    .unwrap();
    let _path_guard = EnvGuard::set(
        CTX_CODEX_HOST_AUTH_PATH_ENV,
        host_auth.to_string_lossy().as_ref(),
    );
    let dir = tempfile::tempdir().unwrap();

    let env = codex_env_for_active_account(dir.path()).await.unwrap();
    let home = env.get("CODEX_HOME").unwrap();
    assert_eq!(home, &codex_runtime_home(dir.path()).to_string_lossy());
    ensure_codex_auth_ready(Path::new(home)).await.unwrap();
}

#[tokio::test]
async fn codex_env_seed_enabled_fails_when_host_auth_missing() {
    let _env_lock = lock_env().await;
    let _guard = EnvGuard::without("CTX_CODEX_HOME");
    let _seed_guard = EnvGuard::set(CTX_SEED_CODEX_AUTH_FROM_HOST_ENV, "1");
    let missing = tempfile::tempdir()
        .unwrap()
        .path()
        .join("missing-auth.json");
    let _path_guard = EnvGuard::set(
        CTX_CODEX_HOST_AUTH_PATH_ENV,
        missing.to_string_lossy().as_ref(),
    );
    let dir = tempfile::tempdir().unwrap();

    let err = codex_env_for_active_account(dir.path()).await.unwrap_err();
    assert!(err.to_string().contains("host auth file is missing"));
}

#[tokio::test]
async fn codex_env_clears_stale_runtime_auth_when_no_active_account() {
    let _env_lock = lock_env().await;
    let _guard = EnvGuard::without("CTX_CODEX_HOME");
    let _seed_guard = EnvGuard::without(CTX_SEED_CODEX_AUTH_FROM_HOST_ENV);
    let missing_host_auth = tempfile::tempdir()
        .unwrap()
        .path()
        .join("missing-auth.json");
    let _host_guard = EnvGuard::set(
        CTX_CODEX_HOST_AUTH_PATH_ENV,
        missing_host_auth.to_string_lossy().as_ref(),
    );
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    tokio::fs::create_dir_all(codex_runtime_home(root))
        .await
        .unwrap();
    tokio::fs::write(
        codex_runtime_home(root).join("auth.json"),
        br#"{"OPENAI_API_KEY":"stale-key"}"#,
    )
    .await
    .unwrap();
    write_runtime_owner_marker(root, "acct-stale")
        .await
        .unwrap();

    let env = codex_env_for_active_account(root).await.unwrap();
    let home = env.get("CODEX_HOME").unwrap();
    assert_eq!(home, &codex_runtime_home(root).to_string_lossy());
    assert!(!codex_runtime_home(root).join("auth.json").exists());
    assert!(!codex_runtime_owner_path(root).exists());
}

#[tokio::test]
async fn codex_auth_preflight_accepts_api_key_shape() {
    let dir = tempfile::tempdir().unwrap();
    tokio::fs::write(
        dir.path().join("auth.json"),
        br#"{"OPENAI_API_KEY":"test-key"}"#,
    )
    .await
    .unwrap();
    ensure_codex_auth_ready(dir.path()).await.unwrap();
}

#[tokio::test]
async fn codex_auth_preflight_rejects_missing_supported_fields() {
    let dir = tempfile::tempdir().unwrap();
    tokio::fs::write(
        dir.path().join("auth.json"),
        br#"{"tokens":{"access_token":"a"}}"#,
    )
    .await
    .unwrap();
    let err = ensure_codex_auth_ready(dir.path()).await.unwrap_err();
    assert!(err.to_string().contains("OPENAI_API_KEY"));
}

#[tokio::test]
async fn ingested_secret_projects_even_without_account_dir_auth() {
    let _env_lock = lock_env().await;
    let _guard = EnvGuard::without("CTX_CODEX_HOME");
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let account_id = "acct-123";
    let registry = CodexAccountRegistry {
        active_account_id: Some(account_id.to_string()),
        accounts: vec![CodexAccountEntry {
            id: account_id.to_string(),
            label: "acct".to_string(),
            kind: CODEX_CREDENTIAL_KIND_OAUTH.to_string(),
            email: None,
            plan_type: None,
            created_at: Utc::now(),
            last_used_at: None,
            secret_ref: None,
            endpoint_profile: CodexEndpointProfile::default(),
        }],
    };
    save_codex_registry(root, &registry).await.unwrap();
    let account_dir = ensure_codex_account_dir(root, account_id).await.unwrap();
    tokio::fs::write(
        account_dir.join("auth.json"),
        br#"{"OPENAI_API_KEY":"test-key"}"#,
    )
    .await
    .unwrap();

    ingest_codex_account_auth_to_secret_store(root, account_id)
        .await
        .unwrap();
    tokio::fs::remove_file(account_dir.join("auth.json"))
        .await
        .unwrap();

    let env = codex_env_for_active_account(root).await.unwrap();
    let home = env.get("CODEX_HOME").unwrap();
    assert_eq!(home, &codex_runtime_home(root).to_string_lossy());
    ensure_codex_auth_ready(Path::new(home)).await.unwrap();
}

#[tokio::test]
async fn runtime_home_refresh_reconciles_back_to_active_secret() {
    let _env_lock = lock_env().await;
    let _guard = EnvGuard::without("CTX_CODEX_HOME");
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let account_id = "acct-123";
    let secret_ref = format!("{account_id}.json");
    let registry = CodexAccountRegistry {
        active_account_id: Some(account_id.to_string()),
        accounts: vec![CodexAccountEntry {
            id: account_id.to_string(),
            label: "acct".to_string(),
            kind: CODEX_CREDENTIAL_KIND_OAUTH.to_string(),
            email: None,
            plan_type: None,
            created_at: Utc::now(),
            last_used_at: None,
            secret_ref: Some(secret_ref.clone()),
            endpoint_profile: CodexEndpointProfile::default(),
        }],
    };
    save_codex_registry(root, &registry).await.unwrap();
    tokio::fs::create_dir_all(codex_secrets_root(root))
        .await
        .unwrap();
    tokio::fs::write(
        codex_secret_path(root, &secret_ref),
        br#"{"version":1,"auth":{"tokens":{"access_token":"old-access","refresh_token":"old-refresh"}}}"#,
    )
    .await
    .unwrap();
    tokio::fs::create_dir_all(codex_runtime_home(root))
        .await
        .unwrap();
    tokio::fs::write(
        codex_runtime_home(root).join("auth.json"),
        br#"{"tokens":{"access_token":"new-access","refresh_token":"new-refresh"}}"#,
    )
    .await
    .unwrap();
    write_runtime_owner_marker(root, account_id).await.unwrap();

    let _ = codex_env_for_active_account(root).await.unwrap();

    let secret_payload = tokio::fs::read_to_string(codex_secret_path(root, &secret_ref))
        .await
        .unwrap();
    assert!(secret_payload.contains("new-access"));
    assert!(secret_payload.contains("new-refresh"));
}

#[tokio::test]
async fn removing_account_cleans_secret_file() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let account_id = "acct-123";
    let secret_ref = format!("{account_id}.json");
    let registry = CodexAccountRegistry {
        active_account_id: Some(account_id.to_string()),
        accounts: vec![CodexAccountEntry {
            id: account_id.to_string(),
            label: "acct".to_string(),
            kind: CODEX_CREDENTIAL_KIND_OAUTH.to_string(),
            email: None,
            plan_type: None,
            created_at: Utc::now(),
            last_used_at: None,
            secret_ref: Some(secret_ref.clone()),
            endpoint_profile: CodexEndpointProfile::default(),
        }],
    };
    save_codex_registry(root, &registry).await.unwrap();
    tokio::fs::create_dir_all(codex_secrets_root(root))
        .await
        .unwrap();
    tokio::fs::write(
        codex_secret_path(root, &secret_ref),
        br#"{"version":1,"auth":{"OPENAI_API_KEY":"test-key"}}"#,
    )
    .await
    .unwrap();

    remove_codex_account(root, account_id).await.unwrap();
    assert!(!codex_secret_path(root, &secret_ref).exists());
}

#[tokio::test]
async fn probe_host_auth_candidate_reports_api_key_shape() {
    let _env_lock = lock_env().await;
    let auth_dir = tempfile::tempdir().unwrap();
    let auth_path = auth_dir.path().join("auth.json");
    tokio::fs::write(&auth_path, br#"{"OPENAI_API_KEY":"test-key"}"#)
        .await
        .unwrap();
    let _path_guard = EnvGuard::set(
        CTX_CODEX_HOST_AUTH_PATH_ENV,
        auth_path.to_string_lossy().as_ref(),
    );

    let probe = probe_host_codex_auth_candidate().await;
    assert!(probe.available);
    assert_eq!(
        probe.auth_kind.as_deref(),
        Some(CODEX_CREDENTIAL_KIND_API_KEY)
    );
    assert_eq!(
        probe.path.as_deref(),
        Some(auth_path.to_string_lossy().as_ref())
    );
}

#[tokio::test]
async fn import_host_auth_persists_secret_and_sets_active() {
    let _env_lock = lock_env().await;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let host_dir = tempfile::tempdir().unwrap();
    let auth_path = host_dir.path().join("auth.json");
    tokio::fs::write(
        &auth_path,
        br#"{"tokens":{"access_token":"a","refresh_token":"b"}}"#,
    )
    .await
    .unwrap();
    let _path_guard = EnvGuard::set(
        CTX_CODEX_HOST_AUTH_PATH_ENV,
        auth_path.to_string_lossy().as_ref(),
    );

    let registry = import_host_codex_auth_to_secret_store(root, Some("Imported".to_string()))
        .await
        .unwrap();
    let active = registry.active_account_id.clone().expect("active account");
    let entry = registry
        .accounts
        .iter()
        .find(|account| account.id == active)
        .expect("imported account");
    assert_eq!(entry.label, "Imported");
    assert_eq!(entry.kind, CODEX_CREDENTIAL_KIND_OAUTH);
    assert!(entry.secret_ref.is_some());
    assert_eq!(
        entry.endpoint_profile.api_shape,
        CODEX_API_SHAPE_OPENAI_RESPONSES
    );
    assert_eq!(entry.endpoint_profile.auth_type, CODEX_AUTH_TYPE_BEARER);

    let env = codex_env_for_active_account(root).await.unwrap();
    let home = env.get("CODEX_HOME").unwrap();
    ensure_codex_auth_ready(Path::new(home)).await.unwrap();
}

#[tokio::test]
async fn import_host_auth_dedupes_existing_account() {
    let _env_lock = lock_env().await;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let host_dir = tempfile::tempdir().unwrap();
    let auth_path = host_dir.path().join("auth.json");
    tokio::fs::write(
        &auth_path,
        br#"{"tokens":{"access_token":"a","refresh_token":"b"}}"#,
    )
    .await
    .unwrap();
    let _path_guard = EnvGuard::set(
        CTX_CODEX_HOST_AUTH_PATH_ENV,
        auth_path.to_string_lossy().as_ref(),
    );

    let first = import_host_codex_auth_to_secret_store(root, Some("First".to_string()))
        .await
        .unwrap();
    let first_active = first.active_account_id.clone().expect("active account");
    assert_eq!(first.accounts.len(), 1);

    let second = import_host_codex_auth_to_secret_store(root, Some("Second".to_string()))
        .await
        .unwrap();
    let second_active = second.active_account_id.clone().expect("active account");
    assert_eq!(second.accounts.len(), 1);
    assert_eq!(second_active, first_active);
}

#[tokio::test]
async fn upsert_rejects_incompatible_endpoint_profile() {
    let dir = tempfile::tempdir().unwrap();
    let entry = CodexAccountEntry {
        id: "acct-incompatible".to_string(),
        label: "Bad Profile".to_string(),
        kind: CODEX_CREDENTIAL_KIND_API_KEY.to_string(),
        email: None,
        plan_type: None,
        created_at: Utc::now(),
        last_used_at: None,
        secret_ref: None,
        endpoint_profile: CodexEndpointProfile {
            api_shape: "anthropic_messages".to_string(),
            auth_type: CODEX_AUTH_TYPE_BEARER.to_string(),
            base_url: Some("https://example.com/v1".to_string()),
        },
    };

    let err = upsert_codex_account(dir.path(), entry).await.unwrap_err();
    assert!(err.to_string().contains("api_shape=openai_responses"));
}

#[tokio::test]
async fn set_active_rejects_incompatible_auth_type() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let registry = CodexAccountRegistry {
        active_account_id: None,
        accounts: vec![CodexAccountEntry {
            id: "acct-bad".to_string(),
            label: "Bad Profile".to_string(),
            kind: CODEX_CREDENTIAL_KIND_API_KEY.to_string(),
            email: None,
            plan_type: None,
            created_at: Utc::now(),
            last_used_at: None,
            secret_ref: None,
            endpoint_profile: CodexEndpointProfile {
                api_shape: CODEX_API_SHAPE_OPENAI_RESPONSES.to_string(),
                auth_type: "basic".to_string(),
                base_url: Some("https://example.com/v1".to_string()),
            },
        }],
    };
    save_codex_registry(root, &registry).await.unwrap();

    let err = set_active_codex_account(root, Some("acct-bad".to_string()))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("auth_type=bearer"));
}

#[tokio::test]
async fn clearing_active_account_clears_runtime_projection() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let registry = CodexAccountRegistry {
        active_account_id: Some("acct-1".to_string()),
        accounts: vec![CodexAccountEntry {
            id: "acct-1".to_string(),
            label: "Account".to_string(),
            kind: CODEX_CREDENTIAL_KIND_API_KEY.to_string(),
            email: None,
            plan_type: None,
            created_at: Utc::now(),
            last_used_at: None,
            secret_ref: None,
            endpoint_profile: CodexEndpointProfile::default(),
        }],
    };
    save_codex_registry(root, &registry).await.unwrap();
    tokio::fs::create_dir_all(codex_runtime_home(root))
        .await
        .unwrap();
    tokio::fs::write(
        codex_runtime_home(root).join("auth.json"),
        br#"{"OPENAI_API_KEY":"stale"}"#,
    )
    .await
    .unwrap();
    write_runtime_owner_marker(root, "acct-1").await.unwrap();

    let _ = set_active_codex_account(root, None).await.unwrap();
    assert!(!codex_runtime_home(root).join("auth.json").exists());
    assert!(!codex_runtime_owner_path(root).exists());
}

#[tokio::test]
async fn removing_active_account_clears_runtime_projection() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let registry = CodexAccountRegistry {
        active_account_id: Some("acct-remove".to_string()),
        accounts: vec![CodexAccountEntry {
            id: "acct-remove".to_string(),
            label: "Account".to_string(),
            kind: CODEX_CREDENTIAL_KIND_API_KEY.to_string(),
            email: None,
            plan_type: None,
            created_at: Utc::now(),
            last_used_at: None,
            secret_ref: None,
            endpoint_profile: CodexEndpointProfile::default(),
        }],
    };
    save_codex_registry(root, &registry).await.unwrap();
    tokio::fs::create_dir_all(codex_runtime_home(root))
        .await
        .unwrap();
    tokio::fs::write(
        codex_runtime_home(root).join("auth.json"),
        br#"{"OPENAI_API_KEY":"stale"}"#,
    )
    .await
    .unwrap();
    write_runtime_owner_marker(root, "acct-remove")
        .await
        .unwrap();

    let _ = remove_codex_account(root, "acct-remove").await.unwrap();
    assert!(!codex_runtime_home(root).join("auth.json").exists());
    assert!(!codex_runtime_owner_path(root).exists());
}

#[tokio::test]
async fn subscription_env_dispatches_to_supported_providers() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let _ = add_claude_account(
        root,
        Some("Claude".to_string()),
        CLAUDE_TEST_SETUP_TOKEN.to_string(),
    )
    .await
    .unwrap();
    let codex_registry = CodexAccountRegistry {
        active_account_id: Some("acct-codex".to_string()),
        accounts: vec![CodexAccountEntry {
            id: "acct-codex".to_string(),
            label: "Codex".to_string(),
            kind: CODEX_CREDENTIAL_KIND_API_KEY.to_string(),
            email: None,
            plan_type: None,
            created_at: Utc::now(),
            last_used_at: None,
            secret_ref: None,
            endpoint_profile: CodexEndpointProfile::default(),
        }],
    };
    save_codex_registry(root, &codex_registry).await.unwrap();
    let codex_account_dir = ensure_codex_account_dir(root, "acct-codex").await.unwrap();
    tokio::fs::write(
        codex_account_dir.join("auth.json"),
        br#"{"OPENAI_API_KEY":"codex-test-key"}"#,
    )
    .await
    .unwrap();
    let _ = add_gemini_account(
        root,
        Some("Gemini".to_string()),
        r#"{"access_token":"token-a","refresh_token":"token-r"}"#.to_string(),
        None,
        None,
    )
    .await
    .unwrap();
    let _ = add_qwen_account(
        root,
        Some("Qwen".to_string()),
        r#"{"access_token":"token-a","refresh_token":"token-r","token_type":"Bearer","expiry_date":4102444800000}"#.to_string(),
        None,
    )
    .await
    .unwrap();
    let _ = add_kimi_account(
        root,
        Some("Kimi".to_string()),
        None,
        r#"{"access_token":"token-a"}"#.to_string(),
        None,
        None,
    )
    .await
    .unwrap();
    let _ = add_copilot_account(
        root,
        Some("Copilot".to_string()),
        "ghp_abc".to_string(),
        None,
    )
    .await
    .unwrap();
    let _ = add_cursor_account(
        root,
        Some("Cursor".to_string()),
        "cursor-key".to_string(),
        None,
    )
    .await
    .unwrap();
    let _ = upsert_amp_account(
        root,
        Some("Amp".to_string()),
        Some("amp@example.com".to_string()),
    )
    .await
    .unwrap();
    let _ = upsert_mistral_account(
        root,
        Some("Mistral".to_string()),
        Some("mistral@example.com".to_string()),
    )
    .await
    .unwrap();

    let claude_env = subscription_env_for_active_account(root, "claude-crp")
        .await
        .unwrap();
    assert!(claude_env.contains_key("CLAUDE_CODE_OAUTH_TOKEN"));
    let gemini_env = subscription_env_for_active_account(root, "gemini")
        .await
        .unwrap();
    assert!(gemini_env.contains_key("GEMINI_CLI_HOME"));
    let qwen_env = subscription_env_for_active_account(root, "qwen")
        .await
        .unwrap();
    assert!(qwen_env.contains_key("HOME"));
    let kimi_env = subscription_env_for_active_account(root, "kimi")
        .await
        .unwrap();
    assert!(kimi_env.contains_key(KIMI_SHARE_DIR_ENV));
    let mistral_env = subscription_env_for_active_account(root, "mistral")
        .await
        .unwrap();
    assert!(mistral_env.contains_key("HOME"));
    let copilot_env = subscription_env_for_active_account(root, "copilot")
        .await
        .unwrap();
    assert!(copilot_env.contains_key("GH_TOKEN"));
    let cursor_env = subscription_env_for_active_account(root, "cursor")
        .await
        .unwrap();
    assert!(cursor_env.contains_key("CURSOR_CONFIG_DIR"));
    let amp_env = subscription_env_for_active_account(root, "amp")
        .await
        .unwrap();
    assert!(amp_env.contains_key("HOME"));
    let unknown_env = subscription_env_for_active_account(root, "unknown")
        .await
        .unwrap();
    assert!(unknown_env.is_empty());
}

#[tokio::test]
async fn subscription_env_runtime_root_projects_path_based_providers() {
    let dir = tempfile::tempdir().unwrap();
    let runtime_dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let runtime_root = runtime_dir.path();

    let _ = add_claude_account(
        root,
        Some("Claude".to_string()),
        CLAUDE_TEST_SETUP_TOKEN.to_string(),
    )
    .await
    .unwrap();
    let codex_registry = CodexAccountRegistry {
        active_account_id: Some("acct-codex".to_string()),
        accounts: vec![CodexAccountEntry {
            id: "acct-codex".to_string(),
            label: "Codex".to_string(),
            kind: CODEX_CREDENTIAL_KIND_API_KEY.to_string(),
            email: None,
            plan_type: None,
            created_at: Utc::now(),
            last_used_at: None,
            secret_ref: None,
            endpoint_profile: CodexEndpointProfile::default(),
        }],
    };
    save_codex_registry(root, &codex_registry).await.unwrap();
    let codex_account_dir = ensure_codex_account_dir(root, "acct-codex").await.unwrap();
    tokio::fs::write(
        codex_account_dir.join("auth.json"),
        br#"{"OPENAI_API_KEY":"codex-test-key"}"#,
    )
    .await
    .unwrap();
    let _ = add_gemini_account(
        root,
        Some("Gemini".to_string()),
        r#"{"access_token":"token-a","refresh_token":"token-r"}"#.to_string(),
        None,
        None,
    )
    .await
    .unwrap();
    let _ = add_qwen_account(
        root,
        Some("Qwen".to_string()),
        r#"{"access_token":"token-a","refresh_token":"token-r","token_type":"Bearer","expiry_date":4102444800000}"#.to_string(),
        None,
    )
    .await
    .unwrap();
    let _ = add_kimi_account(
        root,
        Some("Kimi".to_string()),
        None,
        r#"{"access_token":"token-a"}"#.to_string(),
        None,
        None,
    )
    .await
    .unwrap();
    let _ = add_copilot_account(
        root,
        Some("Copilot".to_string()),
        "gho_runtime_token".to_string(),
        Some("copilot@example.com".to_string()),
    )
    .await
    .unwrap();
    let _ = add_cursor_account(
        root,
        Some("Cursor".to_string()),
        "cursor-key".to_string(),
        None,
    )
    .await
    .unwrap();
    let _ = upsert_amp_account(
        root,
        Some("Amp".to_string()),
        Some("amp@example.com".to_string()),
    )
    .await
    .unwrap();
    let _ = upsert_mistral_account(
        root,
        Some("Mistral".to_string()),
        Some("mistral@example.com".to_string()),
    )
    .await
    .unwrap();

    let claude_env =
        subscription_env_for_active_account_with_runtime_root(root, runtime_root, "claude-crp")
            .await
            .unwrap();
    let claude_dir = PathBuf::from(claude_env.get("CLAUDE_CONFIG_DIR").unwrap());
    assert!(claude_dir.starts_with(runtime_root));

    let codex_env =
        subscription_env_for_active_account_with_runtime_root(root, runtime_root, "codex")
            .await
            .unwrap();
    let codex_home = PathBuf::from(codex_env.get("CODEX_HOME").unwrap());
    assert!(codex_home.starts_with(runtime_root));
    assert!(codex_home.join("auth.json").exists());

    let gemini_env =
        subscription_env_for_active_account_with_runtime_root(root, runtime_root, "gemini")
            .await
            .unwrap();
    let gemini_home = PathBuf::from(gemini_env.get("GEMINI_CLI_HOME").unwrap());
    assert!(gemini_home.starts_with(runtime_root));

    let qwen_env =
        subscription_env_for_active_account_with_runtime_root(root, runtime_root, "qwen")
            .await
            .unwrap();
    let qwen_home = PathBuf::from(qwen_env.get("HOME").unwrap());
    assert!(qwen_home.starts_with(runtime_root));

    let kimi_env =
        subscription_env_for_active_account_with_runtime_root(root, runtime_root, "kimi")
            .await
            .unwrap();
    let kimi_share = PathBuf::from(kimi_env.get(KIMI_SHARE_DIR_ENV).unwrap());
    assert!(kimi_share.starts_with(runtime_root));
    assert!(kimi_share
        .join("credentials")
        .join("kimi-code.json")
        .exists());
    let kimi_config = tokio::fs::read_to_string(kimi_share.join("config.toml"))
        .await
        .unwrap();
    assert!(kimi_config.contains("default_model = \"kimi-code/kimi-for-coding\""));

    let copilot_env =
        subscription_env_for_active_account_with_runtime_root(root, runtime_root, "copilot")
            .await
            .unwrap();
    let copilot_home = PathBuf::from(copilot_env.get("HOME").unwrap());
    assert!(copilot_home.starts_with(runtime_root));
    let copilot_config = PathBuf::from(copilot_env.get("XDG_CONFIG_HOME").unwrap());
    assert!(copilot_config.starts_with(runtime_root));
    assert_eq!(
        copilot_env.get("COPILOT_MODEL").map(String::as_str),
        Some("gpt-5-mini")
    );

    let cursor_env =
        subscription_env_for_active_account_with_runtime_root(root, runtime_root, "cursor")
            .await
            .unwrap();
    let cursor_config = PathBuf::from(cursor_env.get("CURSOR_CONFIG_DIR").unwrap());
    assert!(cursor_config.starts_with(runtime_root));

    let amp_env = subscription_env_for_active_account_with_runtime_root(root, runtime_root, "amp")
        .await
        .unwrap();
    let amp_home = PathBuf::from(amp_env.get("HOME").unwrap());
    assert!(amp_home.starts_with(runtime_root));
    let amp_config = PathBuf::from(amp_env.get("XDG_CONFIG_HOME").unwrap());
    assert!(amp_config.starts_with(runtime_root));
    let amp_cache = PathBuf::from(amp_env.get("XDG_CACHE_HOME").unwrap());
    assert!(amp_cache.starts_with(runtime_root));

    let mistral_env =
        subscription_env_for_active_account_with_runtime_root(root, runtime_root, "mistral")
            .await
            .unwrap();
    let mistral_home = PathBuf::from(mistral_env.get("HOME").unwrap());
    assert!(mistral_home.starts_with(runtime_root));
}
