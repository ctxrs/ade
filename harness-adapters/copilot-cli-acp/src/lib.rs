use anyhow::{bail, Result};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct CopilotLaunchConfig {
    pub copilot_path: PathBuf,
    pub log_dir: Option<PathBuf>,
    pub log_level: Option<String>,
    pub extra_args: Vec<String>,
}

pub fn validate_extra_args(extra_args: &[String]) -> Result<()> {
    for arg in extra_args {
        let arg_str = arg.as_str();
        if arg_str == "--acp" || arg_str.starts_with("--acp=") {
            bail!("do not pass {arg}; copilot-cli-acp always runs --acp --stdio");
        }
        if arg_str == "--stdio" || arg_str.starts_with("--stdio=") {
            bail!("do not pass {arg}; copilot-cli-acp always runs --acp --stdio");
        }
        if arg_str == "--port" || arg_str.starts_with("--port=") {
            bail!("copilot-cli-acp is stdio-only; --port is not supported");
        }
    }
    Ok(())
}

pub fn build_args(config: &CopilotLaunchConfig) -> Vec<String> {
    let mut args = vec!["--acp".to_string(), "--stdio".to_string()];

    if let Some(log_dir) = &config.log_dir {
        args.push("--log-dir".to_string());
        args.push(log_dir.to_string_lossy().to_string());
    }

    if let Some(log_level) = &config.log_level {
        args.push("--log-level".to_string());
        args.push(log_level.clone());
    }

    args.extend(config.extra_args.iter().cloned());
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn build_args_includes_acp_stdio_and_extras() {
        let config = CopilotLaunchConfig {
            copilot_path: PathBuf::from("/usr/bin/copilot"),
            log_dir: Some(PathBuf::from("/tmp/logs")),
            log_level: Some("debug".to_string()),
            extra_args: vec!["--allow-all-tools".to_string(), "--banner".to_string()],
        };

        let args = build_args(&config);

        assert_eq!(
            args,
            vec![
                "--acp",
                "--stdio",
                "--log-dir",
                "/tmp/logs",
                "--log-level",
                "debug",
                "--allow-all-tools",
                "--banner",
            ]
        );
    }

    #[test]
    fn validate_rejects_acp_flags() {
        let err = validate_extra_args(&["--acp".to_string()]).unwrap_err();
        assert!(err.to_string().contains("--acp"));

        let err = validate_extra_args(&["--stdio".to_string()]).unwrap_err();
        assert!(err.to_string().contains("--stdio"));

        let err = validate_extra_args(&["--stdio=true".to_string()]).unwrap_err();
        assert!(err.to_string().contains("--stdio"));
    }

    #[test]
    fn validate_rejects_port_flag() {
        let err = validate_extra_args(&["--port".to_string()]).unwrap_err();
        assert!(err.to_string().contains("--port"));

        let err = validate_extra_args(&["--port=9999".to_string()]).unwrap_err();
        assert!(err.to_string().contains("--port"));
    }

    #[test]
    fn validate_accepts_other_flags() {
        validate_extra_args(&["--allow-all-tools".to_string()]).unwrap();
    }
}
