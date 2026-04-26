use super::*;
use ctx_desktop_ipc::{DesktopDaemonRequest, DesktopHttpResponse, DesktopUploadBlobReq};

use crate::desktop_local_daemon::ensure_local_connection_for_scope;

#[tauri::command]
pub(in super::super) async fn desktop_daemon_request(
    app: tauri::AppHandle,
    window: tauri::Window,
    req: DesktopDaemonRequest,
) -> Result<DesktopHttpResponse, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<DesktopHttpResponse, String> {
        let state = app.state::<ConnectionManager>();
        let manager: &ConnectionManager = state.inner();
        let scope = window.label().to_string();
        // Many UI paths can issue daemon requests before explicitly calling `desktop_connect_local`.
        // The connection manager gates this auto-bootstrap after explicit disconnect/remote intent.
        ensure_local_connection_for_scope(&app, manager, &scope)
            .map_err(|err| format!("{err:#}"))?;
        manager
            .daemon_request_for_scope(&scope, req)
            .map_err(|err| format!("{err:#}"))
    })
    .await
    .map_err(|e| format!("daemon request failed: {e}"))?
}

#[tauri::command]
pub(in super::super) async fn desktop_upload_blob(
    app: tauri::AppHandle,
    window: tauri::Window,
    req: DesktopUploadBlobReq,
) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<serde_json::Value, String> {
        let state = app.state::<ConnectionManager>();
        let scope = window.label().to_string();
        state
            .upload_blob_for_scope(&scope, req.bytes, req.mime_type, req.name)
            .map_err(to_err)
    })
    .await
    .map_err(|e| format!("blob upload failed: {e}"))?
}
