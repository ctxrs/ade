use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use std::{env, fmt};

use anyhow::{Context, Result};
use base64::Engine;
use chrono::Utc;
use clap::Parser;
use ctx_fs::patch::{build_worktree_patch, should_ignore_path};
use ctx_worker_protocol::{DiffArtifact, TerminalControlMessage, WorkerRegistration};
use futures_util::{SinkExt, StreamExt};
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::WebPkiServerVerifier;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, RootCertStore, SignatureScheme};
use tokio::process::Command;
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, connect_async_tls_with_config, Connector};
use tracing::{debug, info, warn};

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
    if let Err(err) = rustls::crypto::aws_lc_rs::default_provider().install_default() {
        warn!("failed to install rustls crypto provider: {err:?}");
    }

    let args = Args::parse();
    let args = ResolvedArgs::from_args(args)?;
    let client = gateway_http_client(args.gateway_ca_pem.as_deref())?;

    register_worker(&client, &args).await?;
    emit_diff(&client, &args).await.ok();

    let diff_task = tokio::spawn(run_diff_watcher(client.clone(), args.clone()));
    let terminal_task = tokio::spawn(run_terminal_control(args.clone()));

    let _ = tokio::try_join!(diff_task, terminal_task)?;
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
