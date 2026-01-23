use std::collections::HashMap;
use std::time::Duration;

fn ctx_env(name: &str) -> std::result::Result<String, std::env::VarError> {
    std::env::var(format!("CTX_{name}"))
}

#[derive(Debug, Clone)]
pub struct LspManagerConfig {
    pub enabled: bool,
    pub rust_command: String,
    pub rust_args: Vec<String>,
    pub ts_command: String,
    pub ts_args: Vec<String>,
    pub py_command: String,
    pub py_args: Vec<String>,
    pub go_command: String,
    pub go_args: Vec<String>,
    pub html_command: String,
    pub html_args: Vec<String>,
    pub css_command: String,
    pub css_args: Vec<String>,
    pub json_command: String,
    pub json_args: Vec<String>,
    pub yaml_command: String,
    pub yaml_args: Vec<String>,
    pub bash_command: String,
    pub bash_args: Vec<String>,
    pub dockerfile_command: String,
    pub dockerfile_args: Vec<String>,
    pub clangd_command: String,
    pub clangd_args: Vec<String>,
    pub lua_command: String,
    pub lua_args: Vec<String>,
    pub toml_command: String,
    pub toml_args: Vec<String>,
    pub markdown_command: String,
    pub markdown_args: Vec<String>,
    /// User-provided servers keyed by language id (BYO LSP).
    pub custom_servers: HashMap<String, (String, Vec<String>)>,
    /// Extension -> language id mapping for BYO LSP (no dot, lowercased).
    pub custom_extension_map: HashMap<String, String>,
    /// Filename -> language id mapping for BYO LSP (exact match).
    pub custom_filename_map: HashMap<String, String>,
    pub diagnostics_wait: Duration,
    pub execute_commands_enabled: bool,
    pub execute_command_allowlist: Vec<String>,
    pub request_timeout: Duration,
}

impl Default for LspManagerConfig {
    fn default() -> Self {
        let enabled = ctx_env("LSP_ENABLED")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        let rust_command =
            ctx_env("LSP_RUST_COMMAND").unwrap_or_else(|_| "rust-analyzer".to_string());
        let rust_args = vec!["--stdio".to_string()];

        let ts_command =
            ctx_env("LSP_TS_COMMAND").unwrap_or_else(|_| "typescript-language-server".to_string());
        let ts_args = vec!["--stdio".to_string()];

        let py_command =
            ctx_env("LSP_PY_COMMAND").unwrap_or_else(|_| "pyright-langserver".to_string());
        let py_args = vec!["--stdio".to_string()];

        let go_command = ctx_env("LSP_GO_COMMAND").unwrap_or_else(|_| "gopls".to_string());
        let go_args = vec!["-mode=stdio".to_string()];

        let html_command = ctx_env("LSP_HTML_COMMAND")
            .unwrap_or_else(|_| "vscode-html-language-server".to_string());
        let html_args = vec!["--stdio".to_string()];

        let css_command =
            ctx_env("LSP_CSS_COMMAND").unwrap_or_else(|_| "vscode-css-language-server".to_string());
        let css_args = vec!["--stdio".to_string()];

        let json_command = ctx_env("LSP_JSON_COMMAND")
            .unwrap_or_else(|_| "vscode-json-language-server".to_string());
        let json_args = vec!["--stdio".to_string()];

        let yaml_command =
            ctx_env("LSP_YAML_COMMAND").unwrap_or_else(|_| "yaml-language-server".to_string());
        let yaml_args = vec!["--stdio".to_string()];

        let bash_command =
            ctx_env("LSP_BASH_COMMAND").unwrap_or_else(|_| "bash-language-server".to_string());
        let bash_args = vec!["start".to_string(), "--stdio".to_string()];

        let dockerfile_command =
            ctx_env("LSP_DOCKERFILE_COMMAND").unwrap_or_else(|_| "docker-langserver".to_string());
        let dockerfile_args = vec!["--stdio".to_string()];

        let clangd_command = ctx_env("LSP_CLANGD_COMMAND").unwrap_or_else(|_| "clangd".to_string());
        let clangd_args = vec!["--stdio".to_string()];

        let lua_command =
            ctx_env("LSP_LUA_COMMAND").unwrap_or_else(|_| "lua-language-server".to_string());
        let lua_args = vec![];

        let toml_command = ctx_env("LSP_TOML_COMMAND").unwrap_or_else(|_| "taplo".to_string());
        let toml_args = vec!["lsp".to_string(), "stdio".to_string()];

        let markdown_command =
            ctx_env("LSP_MARKDOWN_COMMAND").unwrap_or_else(|_| "marksman".to_string());
        let markdown_args = vec!["server".to_string()];

        let diagnostics_wait = Duration::from_secs(
            ctx_env("LSP_DIAGNOSTICS_WAIT_SECS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(6),
        );

        let execute_commands_enabled = ctx_env("LSP_EXECUTE_COMMANDS_ENABLED")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let execute_command_allowlist = ctx_env("LSP_EXECUTE_COMMAND_ALLOWLIST")
            .ok()
            .map(|v| {
                v.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let request_timeout = Duration::from_secs(
            ctx_env("LSP_REQUEST_TIMEOUT_SECS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(10),
        );

        Self {
            enabled,
            rust_command,
            rust_args,
            ts_command,
            ts_args,
            py_command,
            py_args,
            go_command,
            go_args,
            html_command,
            html_args,
            css_command,
            css_args,
            json_command,
            json_args,
            yaml_command,
            yaml_args,
            bash_command,
            bash_args,
            dockerfile_command,
            dockerfile_args,
            clangd_command,
            clangd_args,
            lua_command,
            lua_args,
            toml_command,
            toml_args,
            markdown_command,
            markdown_args,
            custom_servers: HashMap::new(),
            custom_extension_map: HashMap::new(),
            custom_filename_map: HashMap::new(),
            diagnostics_wait,
            execute_commands_enabled,
            execute_command_allowlist,
            request_timeout,
        }
    }
}
