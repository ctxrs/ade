use std::sync::Arc;

use http::header::{HeaderName, HeaderValue};
use tokio_tungstenite::{
    connect_async, tungstenite::client::IntoClientRequest, MaybeTlsStream, WebSocketStream,
};

use crate::daemon::AppState;

use super::WebSessionAccessError;

pub(crate) type WebSessionSignalUpstream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

pub(crate) struct WebSessionSignalViewerGuard {
    state: Arc<AppState>,
    session_id: String,
    released: bool,
}

impl WebSessionSignalViewerGuard {
    pub(crate) async fn release(&mut self) {
        if self.released {
            return;
        }
        let _ = self
            .state
            .transport
            .web_sessions
            .bump_viewers(&self.session_id, -1)
            .await;
        self.released = true;
    }
}

impl Drop for WebSessionSignalViewerGuard {
    fn drop(&mut self) {
        if self.released {
            return;
        }
        let state = self.state.clone();
        let session_id = self.session_id.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let _ = state
                    .transport
                    .web_sessions
                    .bump_viewers(&session_id, -1)
                    .await;
            });
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WebSessionSignalBridgeError {
    NotFound,
    WorkerRequest,
    WorkerAuthHeader,
    WorkerConnect,
}

pub(crate) async fn connect_web_session_signal_bridge(
    state: Arc<AppState>,
    session_id: String,
) -> Result<(WebSessionSignalUpstream, WebSessionSignalViewerGuard), WebSessionSignalBridgeError> {
    let handle = state
        .transport
        .web_sessions
        .get(&session_id)
        .await
        .ok_or(WebSessionSignalBridgeError::NotFound)?;
    let port = handle.worker_port().await;
    let url = format!("ws://127.0.0.1:{port}/signal");
    let viewer_guard = WebSessionSignalViewerGuard {
        state: state.clone(),
        session_id: session_id.clone(),
        released: false,
    };
    let _ = state
        .transport
        .web_sessions
        .bump_viewers(&session_id, 1)
        .await;

    let mut request = match url.into_client_request() {
        Ok(request) => request,
        Err(_) => {
            release_signal_viewer(viewer_guard).await;
            return Err(WebSessionSignalBridgeError::WorkerRequest);
        }
    };
    let header_value: Result<HeaderValue, WebSessionSignalBridgeError> = handle
        .worker_auth_secret()
        .parse()
        .map_err(|_| WebSessionSignalBridgeError::WorkerAuthHeader);
    let header_value = match header_value {
        Ok(header_value) => header_value,
        Err(error) => {
            release_signal_viewer(viewer_guard).await;
            return Err(error);
        }
    };
    request.headers_mut().insert(
        HeaderName::from_static(
            ctx_transport_runtime::web_sessions::WEB_SESSION_WORKER_AUTH_HEADER,
        ),
        header_value,
    );

    let upstream = match connect_async(request).await {
        Ok((upstream, _)) => upstream,
        Err(_) => {
            release_signal_viewer(viewer_guard).await;
            return Err(WebSessionSignalBridgeError::WorkerConnect);
        }
    };

    Ok((upstream, viewer_guard))
}

async fn release_signal_viewer(mut viewer_guard: WebSessionSignalViewerGuard) {
    viewer_guard.release().await;
}

pub(crate) async fn authorize_web_session_signal_bridge(
    state: &Arc<AppState>,
    id: &str,
    token: Option<&str>,
) -> Result<(), WebSessionAccessError> {
    super::authorize_web_session_signal_access(state, id, token).await
}
