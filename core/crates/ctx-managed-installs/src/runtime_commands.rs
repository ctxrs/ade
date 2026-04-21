use std::path::{Path, Path as StdPath, PathBuf};

use anyhow::{anyhow, Result};

use crate::AgentServerCommand;

fn file_stem_matches(path: &StdPath, name: &str) -> bool {
    path.file_stem()
        .and_then(|value| value.to_str())
        .map(|value| value.eq_ignore_ascii_case(name))
        .unwrap_or(false)
}

fn resolve_existing_absolute_path(raw: &str, label: &str) -> Result<PathBuf> {
    let path = PathBuf::from(raw);
    anyhow::ensure!(
        path.is_absolute(),
        "{label} must be an absolute path; got '{raw}'"
    );
    anyhow::ensure!(path.exists(), "{label} does not exist: {}", path.display());
    Ok(path)
}

fn gemini_cli_root_from_entrypoint(path: &StdPath) -> Option<PathBuf> {
    let file_name = path.file_name()?.to_str()?;
    let bundle_dir = path.parent()?;
    let package_dir = bundle_dir.parent()?;
    let scope_dir = package_dir.parent()?;
    let node_modules_dir = scope_dir.parent()?;
    if file_name != "gemini.js"
        || bundle_dir.file_name()?.to_str()? != "bundle"
        || package_dir.file_name()?.to_str()? != "gemini-cli"
        || scope_dir.file_name()?.to_str()? != "@google"
        || node_modules_dir.file_name()?.to_str()? != "node_modules"
    {
        return None;
    }
    Some(package_dir.to_path_buf())
}

pub(crate) fn is_acp_provider_id(provider_id: &str) -> bool {
    matches!(
        provider_id,
        "gemini"
            | "qwen"
            | "cursor"
            | "pi"
            | "opencode"
            | "mistral"
            | "goose"
            | "kimi"
            | "auggie"
            | "amp"
            | "droid"
            | "copilot"
            | "cline"
            | "openhands"
    )
}

fn validate_explicit_gemini_cli_command(command: &str, args: &[String]) -> Result<()> {
    let node_path = resolve_existing_absolute_path(command, "Gemini ACP runtime command")?;
    anyhow::ensure!(
        file_stem_matches(&node_path, "node"),
        "Gemini ACP runtime must use an explicit absolute node executable plus @google/gemini-cli/bundle/gemini.js; got command '{command}'"
    );

    let arg0 = args.first().ok_or_else(|| {
        anyhow!(
            "Gemini ACP runtime must pass an explicit absolute @google/gemini-cli/bundle/gemini.js entrypoint as the first argument"
        )
    })?;
    let cli_entry_path = resolve_existing_absolute_path(arg0, "Gemini ACP entrypoint")?;
    let cli_root = gemini_cli_root_from_entrypoint(&cli_entry_path).ok_or_else(|| {
        anyhow!(
            "Gemini ACP entrypoint must point to @google/gemini-cli/bundle/gemini.js; got '{}'",
            cli_entry_path.display()
        )
    })?;
    anyhow::ensure!(
        cli_root.join("package.json").exists(),
        "Gemini ACP entrypoint must live under a node_modules/@google/gemini-cli install tree: {}",
        cli_entry_path.display()
    );
    Ok(())
}

fn maybe_wrap_gemini_acp_command(
    data_root: &Path,
    cmd: AgentServerCommand,
) -> Result<AgentServerCommand> {
    let _ = data_root;
    validate_explicit_gemini_cli_command(&cmd.command, &cmd.args)?;
    Ok(cmd)
}

fn maybe_set_qwen_openai_auth_type(mut cmd: AgentServerCommand) -> AgentServerCommand {
    if cmd.args.iter().any(|arg| arg == "--auth-type") {
        return cmd;
    }
    cmd.args.push("--auth-type".to_string());
    cmd.args.push("openai".to_string());
    cmd
}

fn maybe_set_bridge_env_override(mut cmd: AgentServerCommand) -> AgentServerCommand {
    if cmd.args.iter().any(|arg| arg == "--override-with-envs") {
        return cmd;
    }
    cmd.args.push("--override-with-envs".to_string());
    cmd
}

fn goose_args_include_developer_builtin(args: &[String]) -> bool {
    args.windows(2).any(|window| {
        window[0] == "--with-builtin"
            && window[1]
                .split(',')
                .any(|value| value.trim() == "developer")
    })
}

fn maybe_set_goose_acp_subcommand(mut cmd: AgentServerCommand) -> AgentServerCommand {
    if !cmd.args.iter().any(|arg| arg == "acp")
        && file_stem_matches(StdPath::new(&cmd.command), "goose")
    {
        cmd.args.insert(0, "acp".to_string());
    }
    if !goose_args_include_developer_builtin(&cmd.args) {
        cmd.args.push("--with-builtin".to_string());
        cmd.args.push("developer".to_string());
    }
    cmd
}

pub(crate) fn normalize_acp_provider_command(
    data_root: &Path,
    provider_id: &str,
    cmd: AgentServerCommand,
) -> Result<AgentServerCommand> {
    if !is_acp_provider_id(provider_id) {
        return Ok(cmd);
    }
    let cmd = if provider_id == "gemini" {
        maybe_wrap_gemini_acp_command(data_root, cmd)?
    } else {
        cmd
    };
    let cmd = if provider_id == "qwen" {
        maybe_set_qwen_openai_auth_type(cmd)
    } else {
        cmd
    };
    let cmd = if provider_id == "goose" {
        maybe_set_goose_acp_subcommand(cmd)
    } else {
        cmd
    };
    let cmd = if provider_id == "openhands" {
        maybe_set_bridge_env_override(cmd)
    } else {
        cmd
    };
    Ok(cmd)
}

pub(crate) fn acp_bridge_command(
    bridge_cmd: &AgentServerCommand,
    acp_cmd: AgentServerCommand,
) -> AgentServerCommand {
    let mut parts = Vec::with_capacity(1 + acp_cmd.args.len());
    parts.push(acp_cmd.command);
    parts.extend(acp_cmd.args);
    let mut args = bridge_cmd.args.clone();
    args.push("--acp-command".to_string());
    args.push(parts.join(" "));
    AgentServerCommand {
        command: bridge_cmd.command.clone(),
        args,
        dependencies: Vec::new(),
        managed: None,
    }
}

pub(crate) fn managed_provider_runtime_command(
    data_root: &Path,
    provider_id: &str,
    managed_cmd: AgentServerCommand,
    bridge_cmd: Option<&AgentServerCommand>,
) -> Result<AgentServerCommand> {
    if !is_acp_provider_id(provider_id) {
        return Ok(managed_cmd);
    }

    let bridge_cmd = bridge_cmd.ok_or_else(|| {
        anyhow!("ACP bridge runtime is not configured or invalid for provider '{provider_id}'")
    })?;
    let acp_cmd = normalize_acp_provider_command(data_root, provider_id, managed_cmd)?;
    Ok(acp_bridge_command(bridge_cmd, acp_cmd))
}
