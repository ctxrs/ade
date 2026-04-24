use std::collections::{HashMap, HashSet};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex as StdMutex};

use tokio::sync::Mutex;

use crate::adapters::ProviderSessionSweepConfig;

use super::runtime::CrpAgentConfig;

mod driver;
mod open_handshake;
mod reaper;
mod registry;
mod state;

pub(super) use self::state::{session_shutdown_reason, CrpPromptRequest, CrpSession};

pub(super) struct CrpSessionPool {
    agent: CrpAgentConfig,
    sessions: Mutex<HashMap<String, Arc<CrpSession>>>,
    active_prompts: Arc<StdMutex<HashSet<String>>>,
    busy_sessions: Arc<StdMutex<HashMap<String, usize>>>,
    pinned_sessions: Arc<StdMutex<HashSet<String>>>,
    default_sweep_config: ProviderSessionSweepConfig,
    supports_session_status: bool,
    reap_in_flight: AtomicBool,
    reap_requested: AtomicBool,
}

impl CrpSessionPool {
    pub(super) fn new(agent: CrpAgentConfig, supports_session_status: bool) -> Self {
        Self {
            agent,
            sessions: Mutex::new(HashMap::new()),
            active_prompts: Arc::new(StdMutex::new(HashSet::new())),
            busy_sessions: Arc::new(StdMutex::new(HashMap::new())),
            pinned_sessions: Arc::new(StdMutex::new(HashSet::new())),
            default_sweep_config: ProviderSessionSweepConfig::from_env(),
            supports_session_status,
            reap_in_flight: AtomicBool::new(false),
            reap_requested: AtomicBool::new(false),
        }
    }
}
