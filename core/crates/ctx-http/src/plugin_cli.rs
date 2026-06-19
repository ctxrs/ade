use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use ctx_core::models::{
    PluginDiagnostic, PluginDiagnosticSeverity, PluginInventoryItem, PluginLoadStatus,
    PluginManifest,
};
use ctx_daemon::daemon::plugins::PLUGIN_MANIFEST_FILE_NAMES;
use ctx_daemon::daemon::PluginInventoryRuntime;
use directories::BaseDirs;
use serde::Serialize;

#[derive(Debug, Args)]
pub(crate) struct PluginCommand {
    #[command(subcommand)]
    pub(crate) command: PluginSubcommand,
}

#[derive(Debug, Subcommand)]
pub(crate) enum PluginSubcommand {
    /// Validate a local plugin manifest.
    Validate(PluginValidateArgs),
    /// List plugins discovered by a local root scan.
    List(PluginInventoryArgs),
    /// Rescan local plugin roots without notifying a running daemon.
    Reload(PluginInventoryArgs),
}

#[derive(Debug, Args)]
pub(crate) struct PluginValidateArgs {
    /// Plugin manifest file, or plugin directory containing ctx-plugin.json/plugin.json.
    pub(crate) path: PathBuf,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct PluginInventoryArgs {
    /// Local plugin root to scan. Repeat to scan multiple roots.
    #[arg(long, action = clap::ArgAction::Append)]
    pub(crate) root: Vec<PathBuf>,
    /// Emit machine-readable JSON.
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Debug, Serialize)]
struct PluginListJson {
    mode: &'static str,
    revision: i64,
    roots: Vec<String>,
    plugins: Vec<PluginListItemJson>,
}

#[derive(Debug, Serialize)]
struct PluginListItemJson {
    id: String,
    status: PluginLoadStatus,
    path: String,
    diagnostics: Vec<PluginDiagnostic>,
}

#[derive(Debug, Serialize)]
struct PluginReloadJson {
    mode: &'static str,
    revision: i64,
    roots: Vec<String>,
    plugin_count: usize,
    status_counts: BTreeMap<String, usize>,
}

pub(crate) async fn run(command: PluginCommand) -> Result<()> {
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    run_with_writer(command, &mut stdout).await
}

async fn run_with_writer(command: PluginCommand, writer: &mut dyn Write) -> Result<()> {
    match command.command {
        PluginSubcommand::Validate(args) => validate_manifest(args, writer),
        PluginSubcommand::List(args) => list_plugins(args, writer).await,
        PluginSubcommand::Reload(args) => reload_plugins(args, writer).await,
    }
}

fn validate_manifest(args: PluginValidateArgs, writer: &mut dyn Write) -> Result<()> {
    let manifest_path = resolve_manifest_path(&args.path)?;
    let bytes = std::fs::read(&manifest_path)
        .with_context(|| format!("reading plugin manifest {}", manifest_path.display()))?;
    let manifest: PluginManifest = serde_json::from_slice(&bytes)
        .with_context(|| format!("parsing plugin manifest {}", manifest_path.display()))?;
    manifest
        .validate()
        .map_err(|error| anyhow::anyhow!("invalid plugin manifest: {error:?}"))?;

    writeln!(
        writer,
        "ok: {} is a valid plugin manifest ({})",
        manifest_path.display(),
        manifest.id
    )?;
    Ok(())
}

async fn list_plugins(args: PluginInventoryArgs, writer: &mut dyn Write) -> Result<()> {
    let runtime = plugin_inventory_runtime(&args.root)?;
    let inventory = runtime.reload().await?;
    if args.json {
        serde_json::to_writer_pretty(
            &mut *writer,
            &PluginListJson {
                mode: "local_scan",
                revision: inventory.revision,
                roots: inventory.roots,
                plugins: inventory
                    .plugins
                    .into_iter()
                    .map(plugin_list_item_json)
                    .collect(),
            },
        )?;
        writeln!(writer)?;
        return Ok(());
    }

    writeln!(writer, "mode: local_scan")?;
    writeln!(writer, "revision: {}", inventory.revision)?;
    writeln!(writer, "roots: {}", inventory.roots.join(", "))?;
    if inventory.plugins.is_empty() {
        writeln!(writer, "plugins: 0")?;
        return Ok(());
    }

    for plugin in inventory.plugins {
        writeln!(
            writer,
            "{}\t{}\t{}\tdiagnostics={}",
            plugin.id,
            status_label(plugin.status),
            plugin.path,
            plugin.diagnostics.len()
        )?;
        for diagnostic in &plugin.diagnostics {
            writeln!(
                writer,
                "  {} {}{}",
                severity_label(&diagnostic.severity),
                diagnostic
                    .code
                    .as_deref()
                    .map(|code| format!("[{code}] "))
                    .unwrap_or_default(),
                diagnostic.message
            )?;
        }
    }
    Ok(())
}

async fn reload_plugins(args: PluginInventoryArgs, writer: &mut dyn Write) -> Result<()> {
    let runtime = plugin_inventory_runtime(&args.root)?;
    let inventory = runtime.reload().await?;
    let status_counts = status_counts(&inventory.plugins);
    if args.json {
        serde_json::to_writer_pretty(
            &mut *writer,
            &PluginReloadJson {
                mode: "local_scan",
                revision: inventory.revision,
                roots: inventory.roots,
                plugin_count: inventory.plugins.len(),
                status_counts,
            },
        )?;
        writeln!(writer)?;
        return Ok(());
    }

    writeln!(writer, "mode: local_scan")?;
    writeln!(writer, "revision: {}", inventory.revision)?;
    writeln!(writer, "roots: {}", inventory.roots.join(", "))?;
    writeln!(writer, "plugins: {}", inventory.plugins.len())?;
    writeln!(
        writer,
        "loaded: {}",
        status_counts.get("loaded").unwrap_or(&0)
    )?;
    writeln!(
        writer,
        "error: {}",
        status_counts.get("error").unwrap_or(&0)
    )?;
    writeln!(
        writer,
        "not_loaded: {}",
        status_counts.get("not_loaded").unwrap_or(&0)
    )?;
    Ok(())
}

fn resolve_manifest_path(path: &Path) -> Result<PathBuf> {
    if path.is_file() {
        return Ok(path.to_path_buf());
    }
    if path.is_dir() {
        for name in PLUGIN_MANIFEST_FILE_NAMES {
            let candidate = path.join(name);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    anyhow::bail!(
        "{} is not a plugin manifest file or directory containing {}",
        path.display(),
        PLUGIN_MANIFEST_FILE_NAMES.join("/")
    )
}

fn plugin_inventory_runtime(explicit_roots: &[PathBuf]) -> Result<PluginInventoryRuntime> {
    if !explicit_roots.is_empty() {
        return Ok(PluginInventoryRuntime::new_with_roots(
            explicit_roots.to_vec(),
        ));
    }

    let data_root = match std::env::var("CTX_DATA_ROOT") {
        Ok(value) if !value.trim().is_empty() => PathBuf::from(value),
        _ => BaseDirs::new()
            .context("resolving home dir")?
            .home_dir()
            .join(".ctx"),
    };
    let data_root = ctx_http_auth::daemon::prepare_daemon_data_root(data_root)?;
    Ok(PluginInventoryRuntime::new(data_root))
}

fn plugin_list_item_json(plugin: PluginInventoryItem) -> PluginListItemJson {
    PluginListItemJson {
        id: plugin.id,
        status: plugin.status,
        path: plugin.path,
        diagnostics: plugin.diagnostics,
    }
}

fn status_counts(plugins: &[PluginInventoryItem]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::from([
        ("error".to_string(), 0),
        ("loaded".to_string(), 0),
        ("not_loaded".to_string(), 0),
    ]);
    for plugin in plugins {
        *counts
            .entry(status_label(plugin.status).to_string())
            .or_default() += 1;
    }
    counts
}

fn status_label(status: PluginLoadStatus) -> &'static str {
    match status {
        PluginLoadStatus::NotLoaded => "not_loaded",
        PluginLoadStatus::Loaded => "loaded",
        PluginLoadStatus::Error => "error",
    }
}

fn severity_label(severity: &PluginDiagnosticSeverity) -> &'static str {
    match severity {
        PluginDiagnosticSeverity::Info => "info",
        PluginDiagnosticSeverity::Warning => "warning",
        PluginDiagnosticSeverity::Error => "error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, Commands};
    use clap::Parser;
    use serde_json::json;
    use std::ffi::OsString;
    use std::sync::OnceLock;
    use tempfile::TempDir;

    #[test]
    fn parses_plugin_list_args() {
        let cli = Cli::try_parse_from([
            "ctx",
            "plugin",
            "list",
            "--root",
            "/tmp/plugins-a",
            "--root",
            "/tmp/plugins-b",
            "--json",
        ])
        .unwrap();

        let Commands::Plugin(command) = cli.command else {
            panic!("expected plugin command");
        };
        let PluginSubcommand::List(args) = command.command else {
            panic!("expected plugin list command");
        };
        assert_eq!(
            args.root,
            vec![
                PathBuf::from("/tmp/plugins-a"),
                PathBuf::from("/tmp/plugins-b")
            ]
        );
        assert!(args.json);
    }

    #[test]
    fn validate_accepts_manifest_file() {
        let temp = TempDir::new().unwrap();
        let manifest_path = write_manifest(
            temp.path(),
            "plugin.json",
            valid_manifest("com.example.valid"),
        );

        let mut output = Vec::new();
        validate_manifest(
            PluginValidateArgs {
                path: manifest_path.clone(),
            },
            &mut output,
        )
        .unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("ok:"));
        assert!(output.contains("com.example.valid"));
        assert!(output.contains(manifest_path.to_string_lossy().as_ref()));
    }

    #[test]
    fn validate_accepts_plugin_directory() {
        let temp = TempDir::new().unwrap();
        write_manifest(
            temp.path(),
            "ctx-plugin.json",
            valid_manifest("com.example.directory"),
        );

        let mut output = Vec::new();
        validate_manifest(
            PluginValidateArgs {
                path: temp.path().to_path_buf(),
            },
            &mut output,
        )
        .unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("com.example.directory"));
        assert!(output.contains("ctx-plugin.json"));
    }

    #[test]
    fn validate_rejects_invalid_manifest() {
        let temp = TempDir::new().unwrap();
        let manifest_path = write_manifest(
            temp.path(),
            "plugin.json",
            json!({
                "schema_version": 1,
                "id": "",
                "name": "Invalid",
                "version": "1.0.0",
                "contributes": {
                    "commands": [{ "id": "invalid.run", "title": "Run" }]
                }
            }),
        );

        let mut output = Vec::new();
        let error = validate_manifest(
            PluginValidateArgs {
                path: manifest_path,
            },
            &mut output,
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("invalid plugin manifest"));
        assert!(error.contains("EmptyField"));
    }

    #[tokio::test]
    async fn list_json_includes_plugin_status_path_and_diagnostics() {
        let temp = TempDir::new().unwrap();
        let plugin_dir = temp.path().join("valid");
        std::fs::create_dir(&plugin_dir).unwrap();
        let manifest_path = write_manifest(
            &plugin_dir,
            "plugin.json",
            valid_manifest("com.example.list"),
        );

        let mut output = Vec::new();
        run_with_writer(
            PluginCommand {
                command: PluginSubcommand::List(PluginInventoryArgs {
                    root: vec![temp.path().to_path_buf()],
                    json: true,
                }),
            },
            &mut output,
        )
        .await
        .unwrap();

        let value: serde_json::Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(value["mode"], "local_scan");
        assert_eq!(value["plugins"][0]["id"], "com.example.list");
        assert_eq!(value["plugins"][0]["status"], "loaded");
        assert_eq!(
            value["plugins"][0]["path"],
            manifest_path.to_string_lossy().as_ref()
        );
        assert_eq!(value["plugins"][0]["diagnostics"], json!([]));
    }

    #[tokio::test]
    async fn list_empty_plugin_roots_env_matches_daemon_semantics() {
        let _guard = process_env_test_lock().lock().await;
        let temp = TempDir::new().unwrap();
        let _plugin_roots_guard = EnvVarGuard::set("CTX_PLUGIN_ROOTS", "");
        let _data_root_guard =
            EnvVarGuard::set("CTX_DATA_ROOT", temp.path().to_string_lossy().as_ref());

        let mut output = Vec::new();
        run_with_writer(
            PluginCommand {
                command: PluginSubcommand::List(PluginInventoryArgs {
                    root: Vec::new(),
                    json: true,
                }),
            },
            &mut output,
        )
        .await
        .unwrap();

        let value: serde_json::Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(value["roots"], json!([]));
        assert_eq!(value["plugins"], json!([]));
    }

    #[tokio::test]
    async fn reload_json_includes_counts() {
        let temp = TempDir::new().unwrap();
        let plugin_dir = temp.path().join("valid");
        std::fs::create_dir(&plugin_dir).unwrap();
        write_manifest(
            &plugin_dir,
            "plugin.json",
            valid_manifest("com.example.reload"),
        );

        let mut output = Vec::new();
        run_with_writer(
            PluginCommand {
                command: PluginSubcommand::Reload(PluginInventoryArgs {
                    root: vec![temp.path().to_path_buf()],
                    json: true,
                }),
            },
            &mut output,
        )
        .await
        .unwrap();

        let value: serde_json::Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(value["mode"], "local_scan");
        assert_eq!(value["revision"], 1);
        assert_eq!(value["plugin_count"], 1);
        assert_eq!(value["status_counts"]["loaded"], 1);
        assert_eq!(value["status_counts"]["error"], 0);
    }

    #[tokio::test]
    async fn reload_human_output_names_local_scan_roots_and_counts() {
        let temp = TempDir::new().unwrap();
        let plugin_dir = temp.path().join("valid");
        std::fs::create_dir(&plugin_dir).unwrap();
        write_manifest(
            &plugin_dir,
            "plugin.json",
            valid_manifest("com.example.reload_human"),
        );

        let mut output = Vec::new();
        run_with_writer(
            PluginCommand {
                command: PluginSubcommand::Reload(PluginInventoryArgs {
                    root: vec![temp.path().to_path_buf()],
                    json: false,
                }),
            },
            &mut output,
        )
        .await
        .unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("mode: local_scan"));
        assert!(output.contains(&format!("roots: {}", temp.path().display())));
        assert!(output.contains("plugins: 1"));
        assert!(output.contains("loaded: 1"));
    }

    fn write_manifest(dir: &Path, name: &str, value: serde_json::Value) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
        path
    }

    fn valid_manifest(id: &str) -> serde_json::Value {
        json!({
            "schema_version": 1,
            "id": id,
            "name": "Test Plugin",
            "version": "1.0.0",
            "entrypoints": [
                {
                    "id": "main",
                    "kind": "process",
                    "command": "node",
                    "args": ["index.js"]
                }
            ],
            "contributes": {
                "commands": [
                    {
                        "id": format!("{id}.run"),
                        "title": "Run",
                        "entrypoint": "main"
                    }
                ]
            }
        })
    }

    fn process_env_test_lock() -> &'static tokio::sync::Mutex<()> {
        static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
    }

    struct EnvVarGuard {
        key: &'static str,
        prev: Option<OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let prev = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, prev }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(value) = self.prev.take() {
                std::env::set_var(self.key, value);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }
}
