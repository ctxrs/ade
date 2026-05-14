use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub(in crate::api) struct ShutdownDaemonReq {
    pub(super) confirm: bool,
    #[serde(default)]
    pub(super) reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub(in crate::api) struct ShutdownDaemonResp {
    pub(super) accepted: bool,
    pub(super) activity: ctx_daemon::daemon::DaemonTurnActivitySummary,
}

#[derive(Debug, Deserialize)]
pub(in crate::api) struct BeginUpdateDrainReq {
    pub(super) confirm: bool,
    #[serde(default)]
    pub(super) reason: Option<String>,
    #[serde(default)]
    pub(super) owner: Option<String>,
}

#[derive(Debug, Serialize)]
pub(in crate::api) struct BeginUpdateDrainResp {
    pub(super) acquired: bool,
    pub(super) activity: ctx_daemon::daemon::DaemonTurnActivitySummary,
}

#[derive(Debug, Deserialize)]
pub(in crate::api) struct ReleaseUpdateDrainReq {
    pub(super) confirm: bool,
}

#[derive(Debug, Serialize)]
pub(in crate::api) struct ReleaseUpdateDrainResp {
    pub(super) released: bool,
}
