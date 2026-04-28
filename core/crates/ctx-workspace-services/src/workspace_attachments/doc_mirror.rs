use super::*;
use std::io::Write;

use tempfile::NamedTempFile;
use toml::Value as TomlValue;

pub(super) async fn materialize_doc_mirror(
    data_root: &Path,
    workspace: &Workspace,
    attachment: &WorkspaceAttachment,
    refresh: bool,
) -> Result<MaterializationResult> {
    let revision = revision_key(attachment);
    let dest = materialized_path_for_attachment(data_root, attachment);
    let should_update = refresh || !dest.exists();
    if should_update {
        if dest.exists() {
            tokio::fs::remove_dir_all(&dest).await?;
        }
        tokio::fs::create_dir_all(&dest).await?;
        run_doc_mirror_script(workspace, attachment, &dest).await?;
    }
    Ok(MaterializationResult {
        path: dest,
        materialized_id: revision,
    })
}

async fn run_doc_mirror_script(
    workspace: &Workspace,
    attachment: &WorkspaceAttachment,
    dest: &Path,
) -> Result<()> {
    if looks_like_url(&attachment.source) {
        return run_doc_mirror_cli(workspace, attachment, dest).await;
    }
    let script_path = resolve_workspace_local_source(
        Path::new(&workspace.root_path),
        &attachment.source,
        "doc mirror script",
    )?;
    if !script_path.is_file() {
        anyhow::bail!("doc mirror script not found: {}", script_path.display());
    }

    let mut cmd = if script_path.extension().and_then(|value| value.to_str()) == Some("py") {
        let mut cmd = Command::new("python3");
        cmd.arg(&script_path);
        cmd
    } else if script_path.extension().and_then(|value| value.to_str()) == Some("sh") {
        let mut cmd = Command::new("bash");
        cmd.arg(&script_path);
        cmd
    } else {
        Command::new(&script_path)
    };

    cmd.arg(dest)
        .current_dir(&workspace.root_path)
        .env("CTX_DOCS_OUTPUT_DIR", dest)
        .env("CTX_DOCS_OUTPUT_DIR", dest)
        .kill_on_drop(true);
    let output = cmd.output().await.context("running doc mirror script")?;
    if !output.status.success() {
        anyhow::bail!(
            "doc mirror script failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

fn docs_mirror_bin() -> PathBuf {
    std::env::var_os("CTX_DOCS_MIRROR_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("ctx-docs-mirror"))
}

fn looks_like_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}

async fn run_doc_mirror_cli(
    workspace: &Workspace,
    attachment: &WorkspaceAttachment,
    dest: &Path,
) -> Result<()> {
    let mut table = toml::value::Table::new();
    table.insert(
        "source".to_string(),
        TomlValue::String(attachment.source.clone()),
    );
    table.insert(
        "docs_url".to_string(),
        TomlValue::String(attachment.source.clone()),
    );
    let cfg = TomlValue::Table(table);
    let cfg_text = toml::to_string_pretty(&cfg).context("serializing docs mirror config")?;
    let mut temp = NamedTempFile::new().context("creating docs mirror config file")?;
    temp.write_all(cfg_text.as_bytes())
        .context("writing docs mirror config")?;
    temp.flush().context("flushing docs mirror config")?;

    let bin = docs_mirror_bin();
    let mut cmd = Command::new(&bin);
    cmd.arg("mirror")
        .arg("--config")
        .arg(temp.path())
        .arg("--out")
        .arg(dest)
        .current_dir(&workspace.root_path)
        .kill_on_drop(true);
    let output = cmd.output().await.context("running ctx-docs-mirror")?;
    if !output.status.success() {
        anyhow::bail!(
            "ctx-docs-mirror failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}
