use agent_client_protocol::Client as _;
use clap::Parser;
use tokio::sync::mpsc;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

use droid_acp::agent::DroidAcpAgent;

#[derive(Parser, Debug)]
#[command(name = "droid-acp")]
#[command(about = "ACP adapter for Droid CLI", long_about = None)]
struct Args {
    #[arg(long, env = "DROID_PATH", default_value = "droid")]
    droid_path: String,
    #[arg(long, env = "DROID_DEFAULT_MODEL")]
    default_model: Option<String>,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> agent_client_protocol::Result<()> {
    let args = Args::parse();

    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    let outgoing = tokio::io::stdout().compat_write();
    let incoming = tokio::io::stdin().compat();

    let local_set = tokio::task::LocalSet::new();
    local_set
        .run_until(async move {
            let (tx, mut rx) = mpsc::unbounded_channel();
            let agent = DroidAcpAgent::new(args.droid_path, args.default_model, tx);

            let (conn, handle_io) = agent_client_protocol::AgentSideConnection::new(
                agent,
                outgoing,
                incoming,
                |fut| {
                    tokio::task::spawn_local(fut);
                },
            );

            tokio::task::spawn_local(async move {
                while let Some((notification, tx)) = rx.recv().await {
                    let result = conn.session_notification(notification).await;
                    if let Err(err) = result {
                        tracing::error!("Failed to send session notification: {err}");
                        break;
                    }
                    tx.send(()).ok();
                }
            });

            handle_io.await
        })
        .await
}
