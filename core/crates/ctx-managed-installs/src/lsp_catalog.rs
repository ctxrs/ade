use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspCatalog {
    pub version: u32,
    #[serde(default)]
    pub servers: Vec<LspCatalogEntry>,
}

impl Default for LspCatalog {
    fn default() -> Self {
        serde_json::from_str(include_str!("lsp_catalog.json")).unwrap_or(LspCatalog {
            version: 1,
            servers: vec![],
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspCatalogEntry {
    pub id: String,
    pub title: String,
    /// LSP language id used for didOpen (e.g. "rust", "python", "kotlin").
    pub language_id: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default)]
    pub filenames: Vec<String>,
    pub install: LspCatalogInstall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LspCatalogInstall {
    /// Delegates to our existing managed Node-based LSP installs (typescript/python/html/yaml/bash/dockerfile).
    ManagedNode { server_id: String },
    /// A downloadable archive or single binary at a URL (supports file:// in tests).
    UrlBinary {
        version: String,
        targets: HashMap<String, LspCatalogTarget>,
    },
    /// Install via `go install <module>@<version>` and use the resulting binary.
    GoInstall {
        module: String,
        version: String,
        binary: String,
    },
    /// Use a system-provided binary on PATH (no managed install).
    System { command: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspCatalogTarget {
    pub url: String,
    pub archive: LspCatalogArchive,
    pub bin_path: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LspCatalogArchive {
    None,
    Gz,
    TarGz,
    Zip,
}

pub fn builtin_catalog() -> LspCatalog {
    LspCatalog::default()
}

fn user_catalog_path(data_root: &Path) -> PathBuf {
    data_root.join("lsp").join("catalog.json")
}

pub async fn load_catalog(data_root: &Path) -> Result<LspCatalog> {
    let mut c = builtin_catalog();
    let path = user_catalog_path(data_root);
    if !path.exists() {
        return Ok(c);
    }
    let txt = tokio::fs::read_to_string(&path)
        .await
        .with_context(|| format!("reading {}", path.display()))?;
    let user: LspCatalog = serde_json::from_str(&txt).context("parsing user LSP catalog")?;
    if user.version != c.version {
        anyhow::bail!(
            "unsupported user LSP catalog version {} (expected {})",
            user.version,
            c.version
        );
    }

    // Merge by id (user entries override built-ins).
    let mut by_id: HashMap<String, LspCatalogEntry> =
        c.servers.into_iter().map(|s| (s.id.clone(), s)).collect();
    for s in user.servers {
        by_id.insert(s.id.clone(), s);
    }
    c.servers = by_id.into_values().collect();
    Ok(c)
}

pub async fn get_entry(data_root: &Path, id: &str) -> Result<LspCatalogEntry> {
    let c = load_catalog(data_root).await?;
    c.servers
        .into_iter()
        .find(|e| e.id == id)
        .ok_or_else(|| anyhow!("unknown LSP catalog server id: {id}"))
}
