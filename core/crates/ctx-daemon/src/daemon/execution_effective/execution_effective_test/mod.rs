use super::{
    effective_execution_settings, effective_execution_settings_classified,
    effective_execution_settings_for_environment, effective_install_target,
};

use std::collections::HashMap;
use std::path::Path;

use ctx_core::ids::WorkspaceId;
use ctx_core::models::{ExecutionEnvironment as SessionExecutionEnvironment, VcsKind, Workspace};
use ctx_store::StoreManager;
use ctx_workspace_config::{ExecutionConfigUpdate, ExecutionEnvironment};

use crate::daemon::DaemonState;
use ctx_provider_install::install_state::InstallTarget;
use ctx_settings_model::{self, ContainerNetworkMode, ExecutionMode, ExecutionSettings, Settings};
use ctx_settings_service::{
    install_target_for_settings, validate_workspace_execution_settings_override,
    EXECUTION_POLICY_TEST_ENV_LOCK,
};

static STORE_MANAGER_OPEN_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct EnvVarGuard {
    key: &'static str,
    previous: Option<String>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let previous = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, previous }
    }

    fn remove(key: &'static str) -> Self {
        let previous = std::env::var(key).ok();
        std::env::remove_var(key);
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if let Some(value) = &self.previous {
            std::env::set_var(self.key, value);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

struct ExecutionEnvGuards {
    _lock: tokio::sync::MutexGuard<'static, ()>,
    _policy: EnvVarGuard,
    _mode: EnvVarGuard,
}

async fn clean_execution_env() -> ExecutionEnvGuards {
    let lock = EXECUTION_POLICY_TEST_ENV_LOCK.lock().await;
    ExecutionEnvGuards {
        _lock: lock,
        _policy: EnvVarGuard::remove("CTX_HOST_EXECUTION_POLICY"),
        _mode: EnvVarGuard::remove("CTX_EXECUTION_MODE"),
    }
}

async fn sandbox_only_execution_env() -> ExecutionEnvGuards {
    let lock = EXECUTION_POLICY_TEST_ENV_LOCK.lock().await;
    ExecutionEnvGuards {
        _lock: lock,
        _policy: EnvVarGuard::set("CTX_HOST_EXECUTION_POLICY", "sandbox_only"),
        _mode: EnvVarGuard::remove("CTX_EXECUTION_MODE"),
    }
}

async fn open_store_manager(path: &Path) -> StoreManager {
    let _guard = STORE_MANAGER_OPEN_LOCK.lock().await;
    StoreManager::open(path).await.expect("open stores")
}

async fn state_with_workspace() -> (tempfile::TempDir, DaemonState, Workspace) {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo_root = temp.path().join("repo");
    std::fs::create_dir_all(&repo_root).expect("create repo root");
    let stores = open_store_manager(temp.path()).await;
    let state = DaemonState::new(
        temp.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://127.0.0.1:4310".to_string(),
        None,
    );
    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            repo_root.to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .expect("create workspace");
    (temp, state, workspace)
}

async fn set_daemon_execution_settings(state: &DaemonState, execution: ExecutionSettings) {
    ctx_settings_service::save_settings(
        state.global_store(),
        &Settings {
            execution: Some(execution),
            ..Settings::default()
        },
    )
    .await
    .expect("save daemon settings");
}

mod install_target;
mod persisted_sessions;
mod sandbox_only_policy;
mod workspace_overrides;
