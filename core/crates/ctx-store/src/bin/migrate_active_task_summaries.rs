use std::env;
use std::path::PathBuf;

use anyhow::{Context, Result};
use ctx_core::ids::WorkspaceId;
use ctx_store::Store;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: migrate_active_task_summaries <workspace_id> <db_path>");
        std::process::exit(1);
    }

    let workspace_id = WorkspaceId(
        uuid::Uuid::parse_str(&args[1])
            .with_context(|| format!("invalid workspace id: {}", args[1]))?,
    );
    let db_path = PathBuf::from(&args[2]);

    let store = Store::open_sqlite(&db_path, None)
        .await
        .with_context(|| format!("opening db at {}", db_path.display()))?;

    let (summaries, total_count) = store.list_workspace_active_page(workspace_id, 200).await?;
    let mut updated = 0usize;
    for summary in &summaries {
        store
            .upsert_workspace_active_task_summary_read_model(summary)
            .await?;
        updated += 1;
    }

    println!(
        "workspace {}: rebuilt {} summaries (active total_count={})",
        workspace_id.0, updated, total_count
    );

    store.close().await;
    Ok(())
}
