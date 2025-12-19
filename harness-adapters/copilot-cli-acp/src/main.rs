use anyhow::{Context, Result};
use clap::Parser;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::{Command, ExitStatus, Stdio};
use std::thread;

use copilot_cli_acp::{build_args, validate_extra_args, CopilotLaunchConfig};

#[derive(Parser, Debug)]
#[command(name = "copilot-cli-acp", version, about = "ACP stdio wrapper for GitHub Copilot CLI")]
struct Args {
    /// Path to the copilot CLI binary.
    #[arg(long, env = "COPILOT_CLI_PATH", default_value = "copilot")]
    copilot_path: PathBuf,

    /// Directory to store Copilot CLI logs.
    #[arg(long, env = "COPILOT_CLI_LOG_DIR")]
    log_dir: Option<PathBuf>,

    /// Copilot CLI log level (info, debug, trace, etc.).
    #[arg(long, env = "COPILOT_CLI_LOG_LEVEL")]
    log_level: Option<String>,

    /// Additional arguments forwarded to the Copilot CLI.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    copilot_args: Vec<String>,
}

fn main() -> Result<()> {
    let args = Args::parse();

    validate_extra_args(&args.copilot_args)?;

    let config = CopilotLaunchConfig {
        copilot_path: args.copilot_path,
        log_dir: args.log_dir,
        log_level: args.log_level,
        extra_args: args.copilot_args,
    };

    let status = run_copilot(config)?;
    std::process::exit(exit_code(status));
}

fn run_copilot(config: CopilotLaunchConfig) -> Result<ExitStatus> {
    let args = build_args(&config);

    let mut child = Command::new(&config.copilot_path)
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to spawn copilot CLI at {}", config.copilot_path.display()))?;

    let mut child_stdin = child.stdin.take().context("missing child stdin")?;
    let mut child_stdout = child.stdout.take().context("missing child stdout")?;
    let mut child_stderr = child.stderr.take().context("missing child stderr")?;

    let stdin_thread = thread::spawn(move || {
        let mut stdin = io::stdin();
        let _ = io::copy(&mut stdin, &mut child_stdin);
    });

    let stdout_thread = thread::spawn(move || {
        let mut stdout = io::stdout();
        let _ = io::copy(&mut child_stdout, &mut stdout);
        let _ = stdout.flush();
    });

    let stderr_thread = thread::spawn(move || {
        let mut stderr = io::stderr();
        let _ = io::copy(&mut child_stderr, &mut stderr);
        let _ = stderr.flush();
    });

    let status = child.wait().context("failed to wait for copilot CLI")?;

    let _ = stdin_thread.join();
    let _ = stdout_thread.join();
    let _ = stderr_thread.join();

    Ok(status)
}

fn exit_code(status: ExitStatus) -> i32 {
    match status.code() {
        Some(code) => code,
        None => 1,
    }
}
