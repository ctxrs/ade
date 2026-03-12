use super::*;
use ctx_core::models::VcsKind;
use tempfile::tempdir;

#[tokio::test]
async fn sweeper_eviction_keeps_active_entries() {
    let temp = tempdir().unwrap();
    let stores = StoreManager::open(temp.path()).await.unwrap();
    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));
    let state = Arc::new(AppState::new(
        temp.path().to_path_buf(),
        stores.clone(),
        providers,
        "http://localhost".to_string(),
        None,
    ));

    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            temp.path().to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .unwrap();
    let store = state.store_for_workspace(workspace.id).await.unwrap();
    let worktree = store
        .create_worktree(
            workspace.id,
            temp.path().to_string_lossy().to_string(),
            "deadbeef".to_string(),
            None,
        )
        .await
        .unwrap();
    let task = store
        .create_task(workspace.id, "task".to_string(), None)
        .await
        .unwrap();
    let session = store
        .create_session(
            task.id,
            workspace.id,
            worktree.id,
            ctx_core::models::ExecutionEnvironment::Host,
            "fake".to_string(),
            "model".to_string(),
            "implementer".to_string(),
            None,
            None,
            None,
        )
        .await
        .unwrap();

    {
        let mut cache = state.sessions.session_head_cache.lock().await;
        cache.insert(session.id, TimedEntry::new(HashMap::new()));
    }
    let _ = state.get_broadcaster(session.id).await;
    let _ = state.subscribe_session_event_head(session.id).await;
    let _ = state.ensure_scheduler(session.clone()).await;

    let now = Instant::now();
    {
        let mut cache = state.sessions.session_head_cache.lock().await;
        if let Some(entry) = cache.get_mut(&session.id) {
            entry.last_access = now - Duration::from_secs(3600);
        }
    }
    {
        let mut map = state.sessions.broadcasters.lock().await;
        if let Some(entry) = map.get_mut(&session.id) {
            entry.last_access = now;
        }
    }
    {
        let mut map = state.sessions.schedulers.lock().await;
        if let Some(entry) = map.get_mut(&session.id) {
            entry.last_access = now;
        }
    }

    let config = CacheSweepConfig {
        session_ttl: Duration::from_secs(60),
        workspace_ttl: Duration::from_secs(365 * 24 * 60 * 60),
        interval: Duration::from_secs(1),
    };
    let stats = state.sweep_idle_caches(now, config).await;
    assert_eq!(stats.session_head_evicted, 1);
    assert!(state
        .sessions
        .session_head_cache
        .lock()
        .await
        .get(&session.id)
        .is_none());
    assert!(state
        .sessions
        .broadcasters
        .lock()
        .await
        .get(&session.id)
        .is_some());
    assert!(state
        .sessions
        .schedulers
        .lock()
        .await
        .get(&session.id)
        .is_some());
}

#[test]
fn normalizes_qwen_command_with_openai_auth_type() {
    let temp = tempdir().unwrap();
    let input = installer::AgentServerCommand {
        command: "/tmp/qwen".to_string(),
        args: vec!["--experimental-acp".to_string()],
        dependencies: Vec::new(),
        managed: None,
    };
    let normalized =
        normalize_acp_provider_command(temp.path(), "qwen", input).expect("normalized qwen");
    assert_eq!(
        normalized.args,
        vec![
            "--experimental-acp".to_string(),
            "--auth-type".to_string(),
            "openai".to_string(),
        ]
    );
}

#[test]
fn normalizes_openhands_command_with_env_override_flag() {
    let temp = tempdir().unwrap();
    let input = installer::AgentServerCommand {
        command: "/tmp/openhands".to_string(),
        args: vec!["acp".to_string()],
        dependencies: Vec::new(),
        managed: None,
    };
    let normalized = normalize_acp_provider_command(temp.path(), "openhands", input)
        .expect("normalized openhands");
    assert_eq!(
        normalized.args,
        vec!["acp".to_string(), "--override-with-envs".to_string()]
    );
}

#[test]
fn runtime_probe_command_wraps_acp_provider_with_bridge() {
    let temp = tempdir().unwrap();
    let cursor_cmd = temp.path().join("cursor-agent");
    let bridge_cmd = temp.path().join("acp-crp-bridge");
    std::fs::write(&cursor_cmd, b"cursor").unwrap();
    std::fs::write(&bridge_cmd, b"bridge").unwrap();
    let cfg = installer::AgentServerConfigFile {
        providers: HashMap::from([
            (
                "cursor".to_string(),
                installer::AgentServerCommand {
                    command: cursor_cmd.to_string_lossy().to_string(),
                    args: vec!["--experimental-acp".to_string()],
                    dependencies: vec!["cursor-dep".to_string()],
                    managed: None,
                },
            ),
            (
                "acp-crp-bridge".to_string(),
                installer::AgentServerCommand {
                    command: bridge_cmd.to_string_lossy().to_string(),
                    args: vec!["--log-level".to_string(), "debug".to_string()],
                    dependencies: vec!["bridge-dep".to_string()],
                    managed: None,
                },
            ),
        ]),
        provider_login_commands: HashMap::new(),
        managed_installs: HashMap::new(),
        managed_provider_targets: HashMap::new(),
        managed_install_targets: HashMap::new(),
    };

    let resolved = runtime_probe_command_as_agent_command(temp.path(), &cfg, "cursor")
        .expect("probe command")
        .expect("runtime command");

    assert_eq!(
        PathBuf::from(&resolved.command)
            .file_name()
            .and_then(|name| name.to_str()),
        Some("acp-crp-bridge")
    );
    assert_eq!(resolved.args.len(), 4);
    assert_eq!(resolved.args[0], "--log-level");
    assert_eq!(resolved.args[1], "debug");
    assert_eq!(resolved.args[2], "--acp-command");
    assert!(resolved
        .args
        .get(3)
        .is_some_and(|arg| arg.ends_with("/cursor-agent --experimental-acp")));
    assert_eq!(
        resolved.dependencies,
        vec!["bridge-dep".to_string(), "cursor-dep".to_string()]
    );
}

#[test]
fn runtime_probe_command_keeps_native_crp_provider_unwrapped() {
    let temp = tempdir().unwrap();
    let codex_cmd = temp.path().join("codex");
    std::fs::write(&codex_cmd, b"codex").unwrap();
    let cfg = installer::AgentServerConfigFile {
        providers: HashMap::from([(
            "codex".to_string(),
            installer::AgentServerCommand {
                command: codex_cmd.to_string_lossy().to_string(),
                args: vec!["serve".to_string()],
                dependencies: vec!["codex-dep".to_string()],
                managed: None,
            },
        )]),
        provider_login_commands: HashMap::new(),
        managed_installs: HashMap::new(),
        managed_provider_targets: HashMap::new(),
        managed_install_targets: HashMap::new(),
    };

    let resolved = runtime_probe_command_as_agent_command(temp.path(), &cfg, "codex")
        .expect("probe command")
        .expect("runtime command");

    assert_eq!(
        PathBuf::from(&resolved.command)
            .file_name()
            .and_then(|name| name.to_str()),
        Some("codex")
    );
    assert_eq!(resolved.args, vec!["serve".to_string()]);
    assert_eq!(resolved.dependencies, vec!["codex-dep".to_string()]);
}
