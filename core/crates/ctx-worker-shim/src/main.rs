use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use std::{env, fmt};

use anyhow::{Context, Result};
use chrono::Utc;
use clap::Parser;
use ctx_worker_protocol::{DiffArtifact, RelayMessage, WorkerRegistration};
use futures_util::{SinkExt, StreamExt};
use http::Request;
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::process::Command;
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::tungstenite::Message;
use tracing::{info, warn};

#[derive(Parser, Debug)]
#[command(name = "ctx-worker-shim")]
struct Args {
    #[arg(long)]
    gateway_url: Option<String>,
    #[arg(long)]
    worker_id: Option<String>,
    #[arg(long)]
    workdir: Option<PathBuf>,
    #[arg(long, default_value = "HEAD")]
    base_commit: String,
    #[arg(long, default_value_t = 1500)]
    diff_debounce_ms: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args = Args::parse();
    let args = ResolvedArgs::from_args(args)?;
    let client = reqwest::Client::new();

    register_worker(&client, &args).await?;
    emit_diff(&client, &args).await.ok();

    let diff_task = tokio::spawn(run_diff_watcher(client.clone(), args.clone()));
    let acp_task = tokio::spawn(run_acp_relay(args.clone()));

    let _ = tokio::try_join!(diff_task, acp_task)?;
    Ok(())
}

async fn run_diff_watcher(client: reqwest::Client, args: ResolvedArgs) -> Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel::<Event>();
    let mut watcher = watcher(tx)?;
    watcher
        .watch(&args.workdir, RecursiveMode::Recursive)
        .context("watching workdir")?;

    let debounce = Duration::from_millis(args.diff_debounce_ms);
    let mut pending = false;
    let timer = tokio::time::sleep(debounce);
    tokio::pin!(timer);

    loop {
        tokio::select! {
            Some(event) = rx.recv() => {
                if should_ignore_event(&event) {
                    continue;
                }
                pending = true;
                timer.as_mut().reset(tokio::time::Instant::now() + debounce);
            }
            _ = &mut timer, if pending => {
                pending = false;
                if let Err(err) = emit_diff(&client, &args).await {
                    warn!("failed to emit diff: {err:#}");
                }
            }
        }
    }
}

async fn run_acp_relay(args: ResolvedArgs) -> Result<()> {
    let url = format!(
        "{}/workers/{}/acp/worker",
        args.gateway_url, args.worker_id
    );
    let mut req = Request::builder().uri(url);
    if let Some(token) = args.gateway_token.as_deref() {
        req = req.header("x-ctx-gateway-token", token);
    }
    let req = req.body(()).context("building websocket request")?;

    let (ws_stream, _) = tokio_tungstenite::connect_async(req)
        .await
        .context("connecting to gateway acp relay")?;
    let (ws_write, mut ws_read) = ws_stream.split();
    let ws_write = std::sync::Arc::new(Mutex::new(ws_write));
    let sessions: std::sync::Arc<Mutex<HashMap<String, SessionRelay>>> =
        std::sync::Arc::new(Mutex::new(HashMap::new()));

    while let Some(msg) = ws_read.next().await {
        let msg = msg.context("reading websocket")?;
        if let Message::Text(text) = msg {
            let relay: RelayMessage = match serde_json::from_str(&text) {
                Ok(relay) => relay,
                Err(err) => {
                    warn!("invalid relay message: {err:#}");
                    continue;
                }
            };
            match relay {
                RelayMessage::Init {
                    session_id,
                    provider_id,
                    model_id,
                    env,
                    workdir,
                } => {
                    let workdir = workdir
                        .as_deref()
                        .map(PathBuf::from)
                        .unwrap_or_else(|| args.workdir.clone());
                    let mut sessions_guard = sessions.lock().await;
                    if sessions_guard.contains_key(&session_id) {
                        continue;
                    }
                    let relay = spawn_acp_session(
                        session_id.clone(),
                        &provider_id,
                        model_id.as_deref(),
                        env,
                        workdir,
                        ws_write.clone(),
                    )
                    .await?;
                    sessions_guard.insert(session_id, relay);
                }
                RelayMessage::Acp { session_id, payload } => {
                    let sessions_guard = sessions.lock().await;
                    if let Some(relay) = sessions_guard.get(&session_id) {
                        let payload = rewrite_cwd(&payload, &relay.workdir);
                        let _ = relay.tx.send(payload);
                    }
                }
                RelayMessage::Close { session_id } => {
                    let mut sessions_guard = sessions.lock().await;
                    if let Some(relay) = sessions_guard.remove(&session_id) {
                        relay.shutdown().await;
                    }
                }
            }
        }
    }

    Ok(())
}

fn watcher(tx: mpsc::UnboundedSender<Event>) -> Result<RecommendedWatcher> {
    let watcher = notify::recommended_watcher(move |res| {
        if let Ok(event) = res {
            let _ = tx.send(event);
        }
    })?;
    Ok(watcher)
}

fn should_ignore_event(event: &Event) -> bool {
    event.paths.iter().all(|path| should_ignore_path(path))
}

fn should_ignore_path(path: &Path) -> bool {
    for component in path.components() {
        let name = component.as_os_str().to_string_lossy();
        if name == ".git"
            || name == ".ctx"
            || name == "node_modules"
            || name == "target"
            || name == "dist"
            || name == "build"
        {
            return true;
        }
    }
    false
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

    req
        .send()
        .await
        .context("registering worker")?
        .error_for_status()
        .context("registering worker status")?;

    Ok(())
}

async fn emit_diff(client: &reqwest::Client, args: &ResolvedArgs) -> Result<()> {
    let base = &args.base_commit;
    let head = git_output(&args.workdir, &["rev-parse", "HEAD"]).await?;
    let patch = git_output(
        &args.workdir,
        &["diff", "--binary", &format!("{base}..HEAD")],
    )
    .await?;
    let changed_files_raw =
        git_output(&args.workdir, &["diff", "--name-only", &format!("{base}..HEAD")]).await?;
    let changed_files = changed_files_raw
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.trim().to_string())
        .collect::<Vec<_>>();
    let (file_count, line_additions, line_deletions) = diff_stats(&args.workdir, base).await?;

    let diff = DiffArtifact {
        worker_id: args.worker_id.clone(),
        base_commit_sha: base.to_string(),
        head_commit_sha: head.trim().to_string(),
        generated_at: Utc::now(),
        patch,
        changed_files,
        file_count,
        line_additions,
        line_deletions,
    };

    let url = format!("{}/workers/{}/diff", args.gateway_url, args.worker_id);
    let mut req = client.post(url).json(&diff);
    if let Some(token) = args.gateway_token.as_deref() {
        req = req.header("x-ctx-gateway-token", token);
    }

    req
        .send()
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

async fn diff_stats(workdir: &Path, base: &str) -> Result<(i64, i64, i64)> {
    let output = git_output(workdir, &["diff", "--numstat", &format!("{base}..HEAD")]).await?;
    let mut files = 0i64;
    let mut additions = 0i64;
    let mut deletions = 0i64;

    for line in output.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 3 {
            continue;
        }
        files += 1;
        additions += parts[0].parse::<i64>().unwrap_or(0);
        deletions += parts[1].parse::<i64>().unwrap_or(0);
    }

    Ok((files, additions, deletions))
}

#[derive(Clone)]
struct ResolvedArgs {
    gateway_url: String,
    worker_id: String,
    workdir: PathBuf,
    base_commit: String,
    diff_debounce_ms: u64,
    gateway_token: Option<String>,
}

impl ResolvedArgs {
    fn from_args(args: Args) -> Result<Self> {
        let gateway_url = resolve_string(args.gateway_url, "CTX_GATEWAY_URL")?;
        let worker_id = resolve_string(args.worker_id, "CTX_WORKER_ID")?;
        let workdir = resolve_path(args.workdir, "CTX_WORKDIR")?;
        let gateway_token = env::var("CTX_WORKER_GATEWAY_TOKEN").ok().filter(|v| !v.is_empty());

        Ok(Self {
            gateway_url,
            worker_id,
            workdir,
            base_commit: args.base_commit,
            diff_debounce_ms: args.diff_debounce_ms,
            gateway_token,
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

struct SessionRelay {
    tx: mpsc::UnboundedSender<String>,
    workdir: PathBuf,
    shutdown_tx: Option<mpsc::UnboundedSender<()>>,
}

impl SessionRelay {
    async fn shutdown(self) {
        if let Some(tx) = self.shutdown_tx {
            let _ = tx.send(());
        }
    }
}

async fn spawn_acp_session(
    session_id: String,
    provider_id: &str,
    model_id: Option<&str>,
    env: HashMap<String, String>,
    workdir: PathBuf,
    ws_write: std::sync::Arc<Mutex<
        futures_util::stream::SplitSink<
            tokio_tungstenite::WebSocketStream<
                tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
            >,
            Message,
        >,
    >>,
) -> Result<SessionRelay> {
    let (cmd, args) = provider_command(provider_id);
    let mut command = Command::new(cmd);
    command.args(args);
    if let Some(model_id) = model_id {
        command.env("CTX_MODEL_ID", model_id);
    }
    for (key, value) in env {
        command.env(key, value);
    }
    command.current_dir(&workdir);
    command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit());

    let mut child = command.spawn().context("spawning acp provider")?;
    let stdin = child.stdin.take().context("missing stdin")?;
    let stdout = child.stdout.take().context("missing stdout")?;

    let (line_tx, mut line_rx) = mpsc::unbounded_channel::<String>();
    let (shutdown_tx, mut shutdown_rx) = mpsc::unbounded_channel::<()>();

    let session_id_clone = session_id.clone();
    let ws_write_clone = ws_write.clone();
    tokio::spawn(async move {
        let mut reader = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            let relay = RelayMessage::Acp {
                session_id: session_id_clone.clone(),
                payload: line,
            };
            if let Ok(text) = serde_json::to_string(&relay) {
                let mut sink = ws_write_clone.lock().await;
                let _ = sink.send(Message::Text(text.into())).await;
            }
        }
    });

    tokio::spawn(async move {
        let mut writer = BufWriter::new(stdin);
        loop {
            tokio::select! {
                Some(line) = line_rx.recv() => {
                    let _ = writer.write_all(line.as_bytes()).await;
                    let _ = writer.write_all(b"\n").await;
                    let _ = writer.flush().await;
                }
                _ = shutdown_rx.recv() => {
                    break;
                }
            }
        }
    });

    Ok(SessionRelay {
        tx: line_tx,
        workdir,
        shutdown_tx: Some(shutdown_tx),
    })
}

fn provider_command(provider_id: &str) -> (&'static str, Vec<&'static str>) {
    match provider_id {
        "codex" => ("codex-acp", Vec::new()),
        "claude" => ("claude-code-acp", Vec::new()),
        "gemini" => ("gemini", vec!["--experimental-acp"]),
        _ => ("codex-acp", Vec::new()),
    }
}

fn rewrite_cwd(payload: &str, workdir: &Path) -> String {
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(payload) else {
        return payload.to_string();
    };
    let method = value.get("method").and_then(|v| v.as_str());
    if matches!(method, Some("session/new")) {
        if let Some(params) = value.get_mut("params") {
            if let Some(map) = params.as_object_mut() {
                map.insert(
                    "cwd".to_string(),
                    serde_json::Value::String(workdir.to_string_lossy().to_string()),
                );
            }
        }
    }
    serde_json::to_string(&value).unwrap_or_else(|_| payload.to_string())
}
