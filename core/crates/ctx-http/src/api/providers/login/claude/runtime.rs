use super::*;

pub(super) const CLAUDE_BROWSER_AUTH_TIER: &str = "provider-browser-auth";

pub(super) struct ClaudeLoginSpawn {
    pub(super) line_rx: mpsc::UnboundedReceiver<String>,
    pub(super) exit_rx: oneshot::Receiver<anyhow::Result<portable_pty::ExitStatus>>,
    pub(super) killer: Arc<StdMutex<Box<dyn portable_pty::ChildKiller + Send + Sync>>>,
    pub(super) browser_open_capture_path: PathBuf,
    pub(super) browser_open_shim_dir: tempfile::TempDir,
}

pub(super) async fn resolve_claude_login_runtime_from_config(
    data_root: &std::path::Path,
) -> anyhow::Result<installer::ProviderRuntimeCommand> {
    if let Some(runtime_command) =
        resolve_runtime_provider_command_from_config(data_root, "claude-cli").await?
    {
        return Ok(runtime_command);
    }

    anyhow::bail!(
        "runtime_command_missing: provider=claude-cli (ctx requires a managed or explicitly configured Claude CLI runtime command; host PATH lookup is not supported)"
    )
}

pub(super) async fn resolve_claude_login_runtime(
    state: &Arc<AppState>,
) -> anyhow::Result<installer::ProviderRuntimeCommand> {
    resolve_claude_login_runtime_from_config(&state.core.data_root).await
}

pub(super) fn claude_login_should_skip_browser_open(raw_tier: Option<&str>) -> bool {
    matches!(
        raw_tier.map(str::trim),
        Some(tier) if tier.eq_ignore_ascii_case(CLAUDE_BROWSER_AUTH_TIER)
    )
}

pub(super) fn claude_browser_open_shim_script(skip_browser_open: bool) -> &'static str {
    if skip_browser_open {
        r#"#!/bin/sh
url="${1:-}"
capture_path="${CTX_CLAUDE_AUTH_URL_CAPTURE_PATH:-}"
if [ -n "$url" ] && [ -n "$capture_path" ]; then
  printf '%s\n' "$url" > "$capture_path"
fi
if [ -n "$url" ]; then
  printf 'CTX_CLAUDE_AUTH_URL:%s\n' "$url"
fi
exit 0
"#
    } else {
        r#"#!/bin/sh
url="${1:-}"
capture_path="${CTX_CLAUDE_AUTH_URL_CAPTURE_PATH:-}"
if [ -n "$url" ] && [ -n "$capture_path" ]; then
  printf '%s\n' "$url" > "$capture_path"
fi
if [ -n "$url" ]; then
  printf 'CTX_CLAUDE_AUTH_URL:%s\n' "$url"
fi
if [ -z "$url" ]; then
  exit 1
fi
if command -v open >/dev/null 2>&1; then
  exec open "$url"
fi
if command -v xdg-open >/dev/null 2>&1; then
  exec xdg-open "$url"
fi
exit 1
"#
    }
}

fn create_claude_browser_open_shim() -> anyhow::Result<(tempfile::TempDir, PathBuf, PathBuf)> {
    let temp_dir = tempfile::Builder::new()
        .prefix("ctx-claude-browser-open-")
        .tempdir()
        .context("creating Claude browser-open shim tempdir")?;
    let script_path = temp_dir.path().join("open-browser");
    let capture_path = temp_dir.path().join("auth-url");
    let skip_browser_open =
        claude_login_should_skip_browser_open(std::env::var("CTX_E2E_TIER").ok().as_deref());
    // This script is the browser-open golden path for Claude setup-token, so
    // it must stay POSIX `sh` compatible.
    let script_body = claude_browser_open_shim_script(skip_browser_open);
    std::fs::write(&script_path, script_body)
        .with_context(|| format!("writing Claude browser-open shim {}", script_path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o700))
            .with_context(|| {
                format!(
                    "marking Claude browser-open shim executable {}",
                    script_path.display()
                )
            })?;
    }
    Ok((temp_dir, script_path, capture_path))
}

pub(super) fn spawn_claude_setup_token_command(
    runtime: &installer::ProviderRuntimeCommand,
) -> anyhow::Result<ClaudeLoginSpawn> {
    let pty = NativePtySystem::default();
    let pair = pty
        .openpty(PtySize {
            rows: 40,
            cols: 400,
            pixel_width: 0,
            pixel_height: 0,
        })
        .context("opening pty for claude setup-token")?;

    let mut cmd = CommandBuilder::new(&runtime.command_abs_path);
    let (browser_open_shim_dir, browser_open_shim_path, browser_open_capture_path) =
        create_claude_browser_open_shim()?;
    for arg in &runtime.args {
        cmd.arg(arg);
    }
    cmd.arg("setup-token");
    cmd.env("NO_COLOR", "1");
    cmd.env("TERM", "xterm-256color");
    cmd.env("BROWSER", browser_open_shim_path);
    cmd.env(
        "CTX_CLAUDE_AUTH_URL_CAPTURE_PATH",
        &browser_open_capture_path,
    );

    let mut child = pair.slave.spawn_command(cmd).with_context(|| {
        format!(
            "spawning claude setup-token via {}",
            runtime.command_abs_path
        )
    })?;
    let killer = Arc::new(StdMutex::new(child.clone_killer()));
    drop(pair.slave);

    let reader = pair
        .master
        .try_clone_reader()
        .context("cloning pty reader for claude setup-token")?;
    let (line_tx, line_rx) = mpsc::unbounded_channel();
    std::thread::spawn(move || {
        pump_claude_login_output(reader, line_tx);
    });

    let (exit_tx, exit_rx) = oneshot::channel();
    std::thread::spawn(move || {
        let result = child
            .wait()
            .context("waiting for claude setup-token process");
        let _ = exit_tx.send(result);
    });

    Ok(ClaudeLoginSpawn {
        line_rx,
        exit_rx,
        killer,
        browser_open_capture_path,
        browser_open_shim_dir,
    })
}

fn pump_claude_login_output<R>(mut reader: R, tx: mpsc::UnboundedSender<String>)
where
    R: std::io::Read,
{
    let mut pending = String::new();
    let mut buf = [0u8; 4096];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                pending.push_str(&String::from_utf8_lossy(&buf[..n]));
                while let Some(newline_idx) = pending.find('\n') {
                    let raw = pending[..newline_idx].to_string();
                    pending.drain(..=newline_idx);
                    if tx.send(normalize_claude_login_line(&raw)).is_err() {
                        return;
                    }
                }
            }
            Err(_) => break,
        }
    }
    if !pending.is_empty() {
        let _ = tx.send(normalize_claude_login_line(&pending));
    }
}
