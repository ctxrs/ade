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
        super::remove_materialized_revision_if_exists(data_root, attachment).await?;
        super::ensure_materialized_revision_parent(data_root, attachment).await?;
        tokio::fs::create_dir(&dest).await?;
        run_doc_mirror_cli(workspace, attachment, &dest).await?;
    } else {
        super::validate_materialized_path(data_root, attachment).await?;
    }
    Ok(MaterializationResult {
        path: dest,
        materialized_id: revision,
    })
}

pub(super) fn validate_doc_mirror_source(
    _workspace: &Workspace,
    attachment: &WorkspaceAttachment,
) -> Result<()> {
    validate_doc_mirror_source_value(&attachment.source)
}

pub(super) fn validate_doc_mirror_source_value(source: &str) -> Result<()> {
    if looks_like_url(source) {
        return Ok(());
    }
    anyhow::bail!(
        "doc_mirror source must be an http(s) URL; executable local doc mirror scripts are not supported"
    )
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
        .env("CTX_DOCS_OUTPUT_DIR", dest)
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
