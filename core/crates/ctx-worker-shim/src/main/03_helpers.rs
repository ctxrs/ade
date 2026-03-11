fn watcher(tx: mpsc::UnboundedSender<Event>) -> Result<RecommendedWatcher> {
    let watcher = notify::recommended_watcher(move |res| {
        if let Ok(event) = res {
            let _ = tx.send(event);
        }
    })?;
    Ok(watcher)
}

fn websocket_base(url: &str) -> String {
    let mut base = url.trim_end_matches('/').to_string();
    if base.starts_with("https://") {
        base = base.replacen("https://", "wss://", 1);
    } else if base.starts_with("http://") {
        base = base.replacen("http://", "ws://", 1);
    } else if !base.starts_with("ws://") && !base.starts_with("wss://") {
        base = format!("ws://{base}");
    }
    base
}

#[derive(Debug)]
struct GatewayCertVerifier {
    inner: Arc<WebPkiServerVerifier>,
    server_name: ServerName<'static>,
    pinned_der: Option<Vec<u8>>,
}

impl ServerCertVerifier for GatewayCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if let Some(pinned) = &self.pinned_der {
            if end_entity.as_ref() == pinned.as_slice() {
                return Ok(ServerCertVerified::assertion());
            }
        }
        self.inner.verify_server_cert(
            end_entity,
            intermediates,
            &self.server_name,
            ocsp_response,
            now,
        )
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

fn gateway_ws_connector(pem: &[u8]) -> Result<Connector> {
    let mut roots = RootCertStore::empty();
    let mut pinned_der: Option<Vec<u8>> = None;
    for cert in CertificateDer::pem_slice_iter(pem) {
        let cert = cert.context("parsing gateway CA")?;
        if pinned_der.is_none() {
            pinned_der = Some(cert.as_ref().to_vec());
        }
        roots.add(cert).context("adding gateway CA")?;
    }
    let verifier = WebPkiServerVerifier::builder(Arc::new(roots.clone()))
        .build()
        .context("building gateway verifier")?;
    let server_name =
        ServerName::try_from("ctx-gateway").context("building gateway server name")?;
    let verifier = GatewayCertVerifier {
        inner: verifier,
        server_name,
        pinned_der,
    };
    let mut config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    config
        .dangerous()
        .set_certificate_verifier(Arc::new(verifier));
    Ok(Connector::Rustls(Arc::new(config)))
}

fn gateway_http_client(gateway_ca_pem: Option<&[u8]>) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder();
    if let Some(pem) = gateway_ca_pem {
        let cert = reqwest::Certificate::from_pem(pem).context("parsing gateway CA certificate")?;
        builder = builder
            .add_root_certificate(cert)
            .danger_accept_invalid_hostnames(true);
    }
    builder.build().context("building gateway http client")
}

fn should_ignore_event(event: &Event) -> bool {
    event.paths.iter().all(|path| should_ignore_path(path))
}

async fn register_worker(client: &reqwest::Client, args: &ResolvedArgs) -> Result<()> {
    let url = format!("{}/workers/{}/register", args.gateway_url, args.worker_id);
    let reg = WorkerRegistration {
        worker_id: args.worker_id.clone(),
        agent_endpoint: None,
        ssh: None,
    };

    let mut req = client.post(url).json(&reg);
    if let Some(token) = args.gateway_token.as_deref() {
        req = req.header("x-ctx-gateway-token", token);
    }

    req.send()
        .await
        .context("registering worker")?
        .error_for_status()
        .context("registering worker status")?;

    Ok(())
}

async fn emit_diff(client: &reqwest::Client, args: &ResolvedArgs) -> Result<()> {
    let base = resolve_base_commit(&args.workdir, &args.base_commit).await?;
    let patch = build_worktree_patch(&args.workdir, &base).await?;

    let diff = DiffArtifact {
        worker_id: args.worker_id.clone(),
        base_commit_sha: patch.base_revision,
        head_commit_sha: patch.head_revision,
        generated_at: Utc::now(),
        patch: patch.patch,
        changed_files: patch.changed_files,
        file_count: patch.file_count,
        line_additions: patch.line_additions,
        line_deletions: patch.line_deletions,
    };

    let url = format!("{}/workers/{}/diff", args.gateway_url, args.worker_id);
    let mut req = client.post(url).json(&diff);
    if let Some(token) = args.gateway_token.as_deref() {
        req = req.header("x-ctx-gateway-token", token);
    }

    req.send()
        .await
        .context("sending diff")?
        .error_for_status()
        .context("diff response")?;

    info!("diff emitted");
    Ok(())
}

async fn git_output(workdir: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(workdir)
        .output()
        .await
        .context("running git")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git command failed: {stderr}");
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

async fn git_rev_parse(workdir: &Path, rev: &str) -> Result<Option<String>> {
    let output = Command::new("git")
        .args(["rev-parse", rev])
        .current_dir(workdir)
        .output()
        .await
        .context("running git")?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(Some(
        String::from_utf8_lossy(&output.stdout).trim().to_string(),
    ))
}

async fn resolve_base_commit(workdir: &Path, base: &str) -> Result<String> {
    if git_commit_exists(workdir, base).await? {
        return Ok(base.to_string());
    }
    if let Some(head) = git_rev_parse(workdir, "HEAD").await? {
        warn!(base, head, "base commit missing; falling back to HEAD");
        return Ok(head);
    }
    let empty_tree = git_output(workdir, &["hash-object", "-t", "tree", "/dev/null"]).await?;
    let empty_tree = empty_tree.trim().to_string();
    warn!(base, empty_tree, "base commit missing; using empty tree");
    Ok(empty_tree)
}

async fn git_commit_exists(workdir: &Path, rev: &str) -> Result<bool> {
    let output = Command::new("git")
        .args(["rev-parse", "--verify", &format!("{rev}^{{commit}}")])
        .current_dir(workdir)
        .output()
        .await
        .context("running git")?;
    Ok(output.status.success())
}
#[derive(Clone)]
struct ResolvedArgs {
    gateway_url: String,
    worker_id: String,
    workdir: PathBuf,
    base_commit: String,
    diff_debounce_ms: u64,
    gateway_token: Option<String>,
    gateway_ca_pem: Option<Vec<u8>>,
}

impl ResolvedArgs {
    fn from_args(args: Args) -> Result<Self> {
        let gateway_url = resolve_string(args.gateway_url, "CTX_GATEWAY_URL")?;
        let worker_id = resolve_string(args.worker_id, "CTX_WORKER_ID")?;
        let workdir = resolve_path(args.workdir, "CTX_WORKDIR")?;
        let gateway_token = env::var("CTX_WORKER_GATEWAY_TOKEN")
            .ok()
            .filter(|v| !v.is_empty());
        let gateway_ca_pem = env::var("CTX_GATEWAY_CA_B64")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .map(|value| {
                base64::engine::general_purpose::STANDARD
                    .decode(value.as_bytes())
                    .context("decoding CTX_GATEWAY_CA_B64")
            })
            .transpose()?;

        Ok(Self {
            gateway_url,
            worker_id,
            workdir,
            base_commit: args.base_commit,
            diff_debounce_ms: args.diff_debounce_ms,
            gateway_token,
            gateway_ca_pem,
        })
    }
}

fn resolve_string(value: Option<String>, env_key: &str) -> Result<String> {
    if let Some(value) = value {
        return Ok(value);
    }
    if let Ok(value) = env::var(env_key) {
        if !value.trim().is_empty() {
            return Ok(value);
        }
    }
    Err(MissingConfig {
        key: env_key.to_string(),
    }
    .into())
}

fn resolve_path(value: Option<PathBuf>, env_key: &str) -> Result<PathBuf> {
    if let Some(value) = value {
        return Ok(value);
    }
    if let Ok(value) = env::var(env_key) {
        let path = PathBuf::from(value);
        return Ok(path);
    }
    Err(MissingConfig {
        key: env_key.to_string(),
    }
    .into())
}

#[derive(Debug)]
struct MissingConfig {
    key: String,
}

impl fmt::Display for MissingConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "missing required config: {}", self.key)
    }
}

impl std::error::Error for MissingConfig {}

#[cfg(test)]
mod helpers_tests {
    use super::*;

    #[test]
    fn shim_websocket_base_normalizes_http_and_https() {
        assert_eq!(websocket_base("http://example.com"), "ws://example.com");
        assert_eq!(websocket_base("http://example.com/"), "ws://example.com");
        assert_eq!(websocket_base("https://example.com"), "wss://example.com");
        assert_eq!(websocket_base("https://example.com/"), "wss://example.com");
    }

    #[test]
    fn shim_websocket_base_preserves_ws_schemes_and_trims_slash() {
        assert_eq!(websocket_base("ws://example.com"), "ws://example.com");
        assert_eq!(websocket_base("ws://example.com/"), "ws://example.com");
        assert_eq!(websocket_base("wss://example.com"), "wss://example.com");
        assert_eq!(websocket_base("wss://example.com/"), "wss://example.com");
    }

    #[test]
    fn shim_websocket_base_defaults_to_ws_when_no_scheme_present() {
        assert_eq!(websocket_base("example.com"), "ws://example.com");
        assert_eq!(websocket_base("example.com/"), "ws://example.com");
    }
}
