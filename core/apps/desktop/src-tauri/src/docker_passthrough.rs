use std::path::PathBuf;

use anyhow::Result;

#[derive(Debug, Clone)]
pub struct DockerProxyConfig {
    pub socket_path: PathBuf,
    pub allow_roots: Vec<PathBuf>,
}

#[cfg(target_os = "linux")]
mod platform {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use anyhow::{Context, Result};
    use bytes::Bytes;
    use http_body_util::{BodyExt, Full};
    use hyper::body::Incoming;
    use hyper::client::conn::http1;
    use hyper::service::service_fn;
    use hyper::{Method, Request, Response, StatusCode};
    use hyper_util::rt::TokioIo;
    use serde_json::Value;
    use tokio::net::{UnixListener, UnixStream};
    use tokio::sync::oneshot;

    use super::DockerProxyConfig;

    const DOCKER_SOCKET_PATH: &str = "/var/run/docker.sock";

    #[derive(Debug)]
    pub struct DockerProxyHandle {
        socket_path: PathBuf,
        shutdown: Option<oneshot::Sender<()>>,
        task: Option<tauri::async_runtime::JoinHandle<()>>,
    }

    impl DockerProxyHandle {
        pub fn stop(mut self) {
            if let Some(tx) = self.shutdown.take() {
                let _ = tx.send(());
            }
            if let Some(task) = self.task.take() {
                task.abort();
            }
            let _ = std::fs::remove_file(&self.socket_path);
        }
    }

    pub fn start_docker_proxy(config: DockerProxyConfig) -> Result<DockerProxyHandle> {
        if let Some(parent) = config.socket_path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("creating docker proxy socket dir {}", parent.display())
            })?;
        }
        if config.socket_path.exists() {
            std::fs::remove_file(&config.socket_path).with_context(|| {
                format!("removing existing docker proxy socket {}", config.socket_path.display())
            })?;
        }

        let allow_roots = sanitize_allow_roots(config.allow_roots);
        let socket_path = config.socket_path.clone();
        let (shutdown, shutdown_rx) = oneshot::channel();
        let task = tauri::async_runtime::spawn(async move {
            if let Err(err) = run_proxy(socket_path.clone(), allow_roots, shutdown_rx).await {
                eprintln!("docker proxy stopped: {err:#}");
            }
        });

        Ok(DockerProxyHandle {
            socket_path: config.socket_path,
            shutdown: Some(shutdown),
            task: Some(task),
        })
    }

    fn sanitize_allow_roots(roots: Vec<PathBuf>) -> Vec<PathBuf> {
        let mut cleaned = Vec::new();
        for root in roots {
            if !root.is_absolute() {
                continue;
            }
            let resolved = root.canonicalize().unwrap_or(root);
            if cleaned.iter().any(|existing| existing == &resolved) {
                continue;
            }
            cleaned.push(resolved);
        }
        cleaned
    }

    async fn run_proxy(
        socket_path: PathBuf,
        allow_roots: Vec<PathBuf>,
        mut shutdown: oneshot::Receiver<()>,
    ) -> Result<()> {
        let listener = UnixListener::bind(&socket_path)
            .with_context(|| format!("binding docker proxy socket {}", socket_path.display()))?;
        let policy = Arc::new(DockerProxyPolicy::new(allow_roots));

        loop {
            tokio::select! {
                _ = &mut shutdown => break,
                accept = listener.accept() => {
                    let (stream, _) = match accept {
                        Ok(value) => value,
                        Err(err) => {
                            eprintln!("docker proxy accept error: {err:#}");
                            continue;
                        }
                    };
                    let policy = policy.clone();
                    tokio::spawn(async move {
                        let io = TokioIo::new(stream);
                        let service = service_fn(move |req| handle_request(policy.clone(), req));
                        if let Err(err) = http1::Builder::new()
                            .serve_connection(io, service)
                            .await
                        {
                            eprintln!("docker proxy connection error: {err:#}");
                        }
                    });
                }
            }
        }

        let _ = std::fs::remove_file(&socket_path);
        Ok(())
    }

    type ProxyBody = http_body_util::combinators::BoxBody<
        Bytes,
        Box<dyn std::error::Error + Send + Sync>,
    >;

    async fn handle_request(
        policy: Arc<DockerProxyPolicy>,
        req: Request<Incoming>,
    ) -> Result<Response<ProxyBody>, std::convert::Infallible> {
        if policy.should_inspect(req.method(), req.uri().path()) {
            let (parts, body) = req.into_parts();
            let body_bytes = match hyper::body::to_bytes(body).await {
                Ok(bytes) => bytes,
                Err(_) => {
                    return Ok(error_response(
                        StatusCode::BAD_REQUEST,
                        "docker proxy failed to read request body",
                    ));
                }
            };
            if let Err(reason) =
                policy.check_request(&parts.method, parts.uri.path(), body_bytes.as_ref())
            {
                return Ok(error_response(StatusCode::FORBIDDEN, &reason));
            }
            let req = Request::from_parts(parts, Full::new(body_bytes));
            let resp = match proxy_request(req).await {
                Ok(resp) => resp,
                Err(status) => error_response(status, "docker proxy upstream error"),
            };
            return Ok(resp);
        }

        let resp = match proxy_request(req).await {
            Ok(resp) => resp,
            Err(status) => error_response(status, "docker proxy upstream error"),
        };
        Ok(resp)
    }

    fn error_response(status: StatusCode, message: &str) -> Response<ProxyBody> {
        let body = Full::new(Bytes::from(message.to_string()))
            .map_err(|err| match err {})
            .boxed();
        Response::builder()
            .status(status)
            .header("content-type", "text/plain; charset=utf-8")
            .body(body)
            .unwrap_or_else(|_| {
                Response::new(Full::new(Bytes::new()).map_err(|err| match err {}).boxed())
            })
    }

    async fn proxy_request<B>(mut req: Request<B>) -> Result<Response<ProxyBody>, StatusCode>
    where
        B: hyper::body::Body<Data = Bytes> + Send + 'static,
        B::Error: std::error::Error + Send + Sync + 'static,
    {
        let uri = req
            .uri()
            .path_and_query()
            .map(|pq| pq.as_str())
            .unwrap_or("/");
        *req.uri_mut() = uri.parse().map_err(|_| StatusCode::BAD_REQUEST)?;

        let stream = UnixStream::connect(DOCKER_SOCKET_PATH)
            .await
            .map_err(|_| StatusCode::BAD_GATEWAY)?;
        let io = TokioIo::new(stream);
        let (mut sender, conn) = http1::handshake(io)
            .await
            .map_err(|_| StatusCode::BAD_GATEWAY)?;
        tokio::spawn(async move {
            if let Err(err) = conn.await {
                eprintln!("docker proxy upstream error: {err:#}");
            }
        });

        let resp = sender
            .send_request(req)
            .await
            .map_err(|_| StatusCode::BAD_GATEWAY)?;
        let (parts, body) = resp.into_parts();
        let body = body
            .map_err(|err| Box::new(err) as Box<dyn std::error::Error + Send + Sync>)
            .boxed();
        Ok(Response::from_parts(parts, body))
    }

    #[derive(Debug, Clone)]
    struct DockerProxyPolicy {
        allow_roots: Vec<PathBuf>,
    }

    impl DockerProxyPolicy {
        fn new(allow_roots: Vec<PathBuf>) -> Self {
            Self { allow_roots }
        }

        fn should_inspect(&self, method: &Method, path: &str) -> bool {
            if !matches!(method, &Method::POST | &Method::PUT) {
                return false;
            }
            let normalized = strip_version_prefix(path);
            if normalized == "/containers/create" {
                return true;
            }
            normalized.starts_with("/containers/") && normalized.ends_with("/update")
        }

        fn check_request(
            &self,
            method: &Method,
            path: &str,
            body: &[u8],
        ) -> Result<(), String> {
            if !self.should_inspect(method, path) {
                return Ok(());
            }
            if body.is_empty() {
                return Ok(());
            }
            let value: Value = match serde_json::from_slice(body) {
                Ok(value) => value,
                Err(_) => return Ok(()),
            };
            self.check_value(&value)
        }

        fn check_value(&self, value: &Value) -> Result<(), String> {
            if let Some(host_config) = value.get("HostConfig").and_then(|v| v.as_object()) {
                self.check_host_config(host_config)?;
            }
            if let Some(mounts) = value.get("Mounts") {
                self.check_mounts(mounts)?;
            }
            Ok(())
        }

        fn check_host_config(
            &self,
            host_config: &serde_json::Map<String, Value>,
        ) -> Result<(), String> {
            if host_config
                .get("Privileged")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                return Err("docker passthrough blocked: privileged containers are disabled".into());
            }
            for key in ["NetworkMode", "PidMode", "IpcMode", "UsernsMode", "UTSMode"] {
                if host_config.get(key).and_then(|v| v.as_str()) == Some("host") {
                    return Err(format!(
                        "docker passthrough blocked: {key} host namespace is disabled"
                    ));
                }
            }
            if let Some(binds) = host_config.get("Binds") {
                self.check_binds(binds)?;
            }
            if let Some(mounts) = host_config.get("Mounts") {
                self.check_mounts(mounts)?;
            }
            Ok(())
        }

        fn check_binds(&self, binds: &Value) -> Result<(), String> {
            let Some(items) = binds.as_array() else {
                return Ok(());
            };
            for entry in items {
                let Some(bind) = entry.as_str() else {
                    continue;
                };
                let source = bind.splitn(2, ':').next().unwrap_or("");
                self.check_bind_source(source)?;
            }
            Ok(())
        }

        fn check_mounts(&self, mounts: &Value) -> Result<(), String> {
            let Some(items) = mounts.as_array() else {
                return Ok(());
            };
            for entry in items {
                let Some(obj) = entry.as_object() else {
                    continue;
                };
                let mount_type = obj
                    .get("Type")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                if mount_type != "bind" {
                    continue;
                }
                if let Some(source) = obj.get("Source").and_then(|v| v.as_str()) {
                    self.check_bind_source(source)?;
                }
            }
            Ok(())
        }

        fn check_bind_source(&self, source: &str) -> Result<(), String> {
            let trimmed = source.trim();
            if trimmed.is_empty() {
                return Ok(());
            }
            let path = Path::new(trimmed);
            if !path.is_absolute() {
                return Ok(());
            }
            let resolved = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
            if self.allow_roots.iter().any(|root| resolved.starts_with(root)) {
                return Ok(());
            }
            Err(format!(
                "docker passthrough blocked: bind mount outside allowlist ({})",
                path.display()
            ))
        }
    }

    fn strip_version_prefix(path: &str) -> &str {
        let Some(rest) = path.strip_prefix("/v") else {
            return path;
        };
        let mut idx = 0usize;
        let mut saw_digit = false;
        for ch in rest.chars() {
            if ch.is_ascii_digit() || ch == '.' {
                if ch.is_ascii_digit() {
                    saw_digit = true;
                }
                idx += ch.len_utf8();
                continue;
            }
            break;
        }
        if saw_digit && rest.as_bytes().get(idx) == Some(&b'/') {
            return &rest[idx..];
        }
        path
    }

    #[cfg(test)]
    mod tests {
        use super::DockerProxyPolicy;
        use serde_json::json;
        use std::path::PathBuf;

        #[test]
        fn policy_blocks_privileged() {
            let policy = DockerProxyPolicy::new(vec![PathBuf::from("/workspace")]);
            let value = json!({
                "HostConfig": {
                    "Privileged": true
                }
            });
            let err = policy.check_value(&value).unwrap_err();
            assert!(err.contains("privileged"));
        }

        #[test]
        fn policy_blocks_host_network() {
            let policy = DockerProxyPolicy::new(vec![PathBuf::from("/workspace")]);
            let value = json!({
                "HostConfig": {
                    "NetworkMode": "host"
                }
            });
            let err = policy.check_value(&value).unwrap_err();
            assert!(err.contains("NetworkMode"));
        }

        #[test]
        fn policy_blocks_bind_outside_allowlist() {
            let policy = DockerProxyPolicy::new(vec![PathBuf::from("/workspace")]);
            let value = json!({
                "HostConfig": {
                    "Binds": ["/etc:/etc:ro"]
                }
            });
            let err = policy.check_value(&value).unwrap_err();
            assert!(err.contains("allowlist"));
        }

        #[test]
        fn policy_allows_bind_inside_allowlist() {
            let policy = DockerProxyPolicy::new(vec![PathBuf::from("/workspace")]);
            let value = json!({
                "HostConfig": {
                    "Binds": ["/workspace/project:/app:rw"]
                }
            });
            assert!(policy.check_value(&value).is_ok());
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod platform {
    use super::DockerProxyConfig;
    use anyhow::Result;

    #[derive(Debug)]
    pub struct DockerProxyHandle;

    impl DockerProxyHandle {
        pub fn stop(self) {}
    }

    pub fn start_docker_proxy(_config: DockerProxyConfig) -> Result<DockerProxyHandle> {
        Ok(DockerProxyHandle)
    }
}

pub use platform::{start_docker_proxy, DockerProxyHandle};
