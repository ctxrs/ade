use super::*;

use crate::desktop_local_daemon::ensure_local_connection;

#[derive(Debug, Deserialize)]
pub(in super::super) struct DesktopDaemonRequest {
    pub(in super::super) method: String,
    pub(in super::super) path: String,
    #[serde(default)]
    pub(in super::super) body: Option<String>,
    #[serde(default)]
    pub(in super::super) headers: Vec<(String, String)>,
}

#[derive(Debug, Serialize)]
pub(in super::super) struct DesktopHttpResponse {
    pub(in super::super) status: u16,
    pub(in super::super) body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(in super::super) content_type: Option<String>,
}

#[tauri::command]
pub(in super::super) async fn desktop_daemon_request(
    app: tauri::AppHandle,
    req: DesktopDaemonRequest,
) -> Result<DesktopHttpResponse, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<DesktopHttpResponse, String> {
        let state = app.state::<ConnectionManager>();
        let manager: &ConnectionManager = state.inner();
        // Many UI paths (including initial app load to a workbench route) can issue daemon requests
        // before explicitly calling `desktop_connect_local`. Auto-connect here to avoid spurious
        // "daemon unavailable" overlays on cold start.
        ensure_local_connection(&app, manager).map_err(|err| format!("{err:#}"))?;
        manager
            .daemon_request(req)
            .map_err(|err| format!("{err:#}"))
    })
    .await
    .map_err(|e| format!("daemon request failed: {e}"))?
}

#[tauri::command]
pub(in super::super) async fn desktop_upload_blob(
    app: tauri::AppHandle,
    bytes: Vec<u8>,
    mime_type: String,
    name: Option<String>,
) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<serde_json::Value, String> {
        let state = app.state::<ConnectionManager>();
        state.upload_blob(bytes, mime_type, name).map_err(to_err)
    })
    .await
    .map_err(|e| format!("blob upload failed: {e}"))?
}
