use super::*;
use ctx_core::provider_policy::CODEX_APP_SERVER_ARGS;
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

mod browser;
mod claude;
mod codex;
mod kimi;
mod mistral;

pub(crate) use browser::{
    get_amp_login, get_gemini_login, get_qwen_login, start_amp_login, start_gemini_login,
    start_qwen_login,
};
#[cfg(test)]
pub(super) use claude::resolve_claude_login_runtime_from_config;
pub(crate) use claude::{get_claude_login, start_claude_login};
pub(crate) use codex::{complete_codex_login, get_codex_login, start_codex_login};
pub(crate) use kimi::{get_kimi_login, start_kimi_login};
pub(crate) use mistral::{get_mistral_login, start_mistral_login};

const CODEX_LOGIN_RPC_TIMEOUT: Duration = Duration::from_secs(30);
const CLAUDE_LOGIN_URL_WAIT: Duration = Duration::from_secs(4);
const GEMINI_LOGIN_TIMEOUT_DEFAULT: Duration = Duration::from_secs(300);
const GEMINI_LOGIN_POLL_INTERVAL: Duration = Duration::from_millis(700);
const QWEN_LOGIN_TIMEOUT_DEFAULT: Duration = Duration::from_secs(300);
const QWEN_LOGIN_POLL_INTERVAL: Duration = Duration::from_millis(700);
const AMP_LOGIN_TIMEOUT_DEFAULT: Duration = Duration::from_secs(300);
const AMP_LOGIN_POLL_INTERVAL: Duration = Duration::from_millis(700);
const MISTRAL_LOGIN_TIMEOUT_DEFAULT: Duration = Duration::from_secs(300);
const MISTRAL_LOGIN_POLL_INTERVAL: Duration = Duration::from_millis(700);
const QWEN_OAUTH_AUTH_METHOD_ID: &str = "qwen-oauth";
const AMP_BROWSER_AUTH_METHOD_ID: &str = "amp_browser_login";

fn is_loopback_host(value: &str) -> bool {
    let host = value.trim().to_ascii_lowercase();
    if host == "localhost" {
        return true;
    }
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return ip.is_loopback();
    }
    false
}

pub(super) fn expected_callback_from_auth_url(auth_url: &str) -> Option<String> {
    let parsed = Url::parse(auth_url).ok()?;
    let redirect = parsed
        .query_pairs()
        .find_map(|(key, value)| (key == "redirect_uri").then_some(value.into_owned()))?;
    let callback = Url::parse(&redirect).ok()?;
    let host = callback.host_str()?;
    if !is_loopback_host(host) {
        return None;
    }
    Some(callback.to_string())
}

pub(super) fn validate_callback_url(
    callback_url: &str,
    expected_callback_url: Option<&str>,
) -> anyhow::Result<()> {
    let callback = Url::parse(callback_url)
        .with_context(|| format!("invalid callback_url: {callback_url}"))?;
    if callback.scheme() != "http" {
        anyhow::bail!("callback_url must use http scheme");
    }
    let host = callback
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("callback_url must include host"))?;
    if !is_loopback_host(host) {
        anyhow::bail!("callback_url host must be loopback");
    }
    if callback.port().is_none() {
        anyhow::bail!("callback_url must include explicit port");
    }
    if !callback.path().starts_with("/auth/callback") {
        anyhow::bail!("callback_url path must start with /auth/callback");
    }
    if callback.query().is_none() {
        anyhow::bail!("callback_url must include query parameters");
    }

    if let Some(expected_raw) = expected_callback_url {
        let expected = Url::parse(expected_raw)
            .with_context(|| format!("invalid expected callback URL: {expected_raw}"))?;
        if callback.port() != expected.port() {
            anyhow::bail!("callback_url port mismatch");
        }
        if callback.path() != expected.path() {
            anyhow::bail!("callback_url path mismatch");
        }
    }
    Ok(())
}

fn gemini_login_timeout() -> Duration {
    let seconds = std::env::var("CTX_GEMINI_LOGIN_TIMEOUT_SECS")
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(GEMINI_LOGIN_TIMEOUT_DEFAULT.as_secs());
    Duration::from_secs(seconds)
}

fn qwen_login_timeout() -> Duration {
    let seconds = std::env::var("CTX_QWEN_LOGIN_TIMEOUT_SECS")
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(QWEN_LOGIN_TIMEOUT_DEFAULT.as_secs());
    Duration::from_secs(seconds)
}

fn amp_login_timeout() -> Duration {
    let seconds = std::env::var("CTX_AMP_LOGIN_TIMEOUT_SECS")
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(AMP_LOGIN_TIMEOUT_DEFAULT.as_secs());
    Duration::from_secs(seconds)
}

fn mistral_login_timeout() -> Duration {
    let seconds = std::env::var("CTX_MISTRAL_LOGIN_TIMEOUT_SECS")
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(MISTRAL_LOGIN_TIMEOUT_DEFAULT.as_secs());
    Duration::from_secs(seconds)
}

fn first_email_from_google_accounts(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Object(map) => {
            let email = map
                .get("email")
                .or_else(|| map.get("accountEmail"))
                .or_else(|| map.get("account_email"))
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|candidate| !candidate.is_empty())
                .map(ToString::to_string);
            email.or_else(|| map.values().find_map(first_email_from_google_accounts))
        }
        serde_json::Value::Array(values) => {
            values.iter().find_map(first_email_from_google_accounts)
        }
        _ => None,
    }
}

fn first_email_from_value(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Object(map) => {
            let direct = map
                .get("email")
                .or_else(|| map.get("accountEmail"))
                .or_else(|| map.get("account_email"))
                .or_else(|| map.get("user_email"))
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|candidate| !candidate.is_empty())
                .map(ToString::to_string);
            direct.or_else(|| map.values().find_map(first_email_from_value))
        }
        serde_json::Value::Array(values) => values.iter().find_map(first_email_from_value),
        _ => None,
    }
}

pub(super) fn extract_auth_url_from_value(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(raw) => {
            let trimmed = raw.trim();
            if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
                return Some(trimmed.to_string());
            }
            extract_auth_url(trimmed)
        }
        serde_json::Value::Object(map) => {
            let direct = [
                "auth_url",
                "authUrl",
                "url",
                "login_url",
                "loginUrl",
                "authorize_url",
            ]
            .into_iter()
            .find_map(|key| map.get(key))
            .and_then(extract_auth_url_from_value);
            direct.or_else(|| map.values().find_map(extract_auth_url_from_value))
        }
        serde_json::Value::Array(values) => values.iter().find_map(extract_auth_url_from_value),
        _ => None,
    }
}

fn auth_notice_code(payload: &serde_json::Value) -> &str {
    payload
        .get("code")
        .or_else(|| payload.get("kind"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
}

fn is_auth_success_notice_code(code: &str) -> bool {
    matches!(
        code,
        "auth_complete" | "auth_completed" | "auth_success" | "authenticated"
    )
}

fn is_auth_failure_notice_code(code: &str) -> bool {
    matches!(code, "auth_failed" | "auth_error")
}

pub(super) async fn resolve_runtime_provider_command_from_config(
    data_root: &std::path::Path,
    provider_id: &str,
) -> anyhow::Result<Option<installer::ProviderRuntimeCommand>> {
    let cfg = installer::load_agent_server_config(data_root)
        .await
        .context("loading agent server config")?;
    installer::resolve_runtime_provider_command(&cfg, provider_id)
        .with_context(|| format!("resolving runtime command for {provider_id}"))
}

pub(super) async fn resolve_provider_login_command_from_config(
    data_root: &std::path::Path,
    provider_id: &str,
) -> anyhow::Result<Option<installer::ProviderRuntimeCommand>> {
    let cfg = installer::load_agent_server_config(data_root)
        .await
        .context("loading agent server config")?;
    installer::resolve_provider_login_command(&cfg, provider_id)
        .with_context(|| format!("resolving prepared login executable for {provider_id}"))
}

fn strip_ansi_sequences(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let mut idx = 0usize;
    while idx < chars.len() {
        let ch = chars[idx];
        if ch == '\u{1b}' {
            idx += 1;
            if idx < chars.len() {
                if chars[idx] == '[' {
                    idx += 1;
                    while idx < chars.len() {
                        let c = chars[idx];
                        idx += 1;
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                    continue;
                }
                if chars[idx] == ']' {
                    idx += 1;
                    let mut payload = String::new();
                    while idx < chars.len() {
                        let c = chars[idx];
                        if c == '\u{7}' {
                            idx += 1;
                            break;
                        }
                        if c == '\u{1b}' && (idx + 1) < chars.len() && chars[idx + 1] == '\\' {
                            idx += 2;
                            break;
                        }
                        payload.push(c);
                        idx += 1;
                    }
                    if let Some(url) = payload
                        .strip_prefix("8;;")
                        .filter(|value| !value.is_empty())
                    {
                        out.push(' ');
                        out.push_str(url);
                        out.push(' ');
                    }
                    continue;
                }
            }
            continue;
        }
        if ch != '\r' && ch != '\u{7}' {
            out.push(ch);
        }
        idx += 1;
    }
    out
}

pub(super) fn normalize_claude_login_line(line: &str) -> String {
    strip_ansi_sequences(line.trim_end_matches('\r'))
}

fn is_auth_url_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric()
        || matches!(
            ch,
            '-' | '.'
                | '_'
                | '~'
                | ':'
                | '/'
                | '?'
                | '#'
                | '['
                | ']'
                | '@'
                | '!'
                | '$'
                | '&'
                | '\''
                | '('
                | ')'
                | '*'
                | '+'
                | ','
                | ';'
                | '='
                | '%'
        )
}

fn matches_auth_url_scheme_from(chars: &[char], start: usize, scheme: &str) -> bool {
    let mut idx = start;
    for expected in scheme.chars() {
        while idx < chars.len() && chars[idx].is_whitespace() {
            idx += 1;
        }
        if idx >= chars.len() || chars[idx].to_ascii_lowercase() != expected {
            return false;
        }
        idx += 1;
    }
    true
}

fn find_auth_url_start(chars: &[char], from_idx: usize) -> Option<usize> {
    let mut idx = from_idx;
    while idx < chars.len() {
        if chars[idx].eq_ignore_ascii_case(&'h')
            && (matches_auth_url_scheme_from(chars, idx, "https://")
                || matches_auth_url_scheme_from(chars, idx, "http://"))
        {
            return Some(idx);
        }
        idx += 1;
    }
    None
}

fn auth_scheme_can_continue_across_break(current: &str, next_fragment: &str) -> bool {
    let compact_current: String = current
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    let compact_next: String = next_fragment
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    if compact_current.is_empty() || compact_next.is_empty() {
        return false;
    }
    let combined = format!("{compact_current}{compact_next}");
    ["https://", "http://"]
        .into_iter()
        .any(|scheme| scheme.starts_with(&combined))
}

fn fragment_starts_full_auth_url(value: &str) -> bool {
    let compact = value
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    compact.starts_with("https://") || compact.starts_with("http://")
}

fn should_continue_auth_url_after_break(current: &str, next_fragment: &str) -> bool {
    if next_fragment.is_empty() {
        return false;
    }
    if fragment_starts_full_auth_url(next_fragment) && Url::parse(current).is_ok() {
        return false;
    }
    if auth_scheme_can_continue_across_break(current, next_fragment) {
        return true;
    }
    if next_fragment.chars().any(|ch| !ch.is_ascii_alphanumeric()) {
        return true;
    }
    if next_fragment
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_digit())
    {
        return true;
    }
    matches!(
        current.chars().last(),
        Some('%' | ':' | '=' | '&' | '?' | '/' | '#' | '-' | '_')
    )
}

pub(super) fn extract_auth_url(text: &str) -> Option<String> {
    let normalized = strip_ansi_sequences(text);
    let chars: Vec<char> = normalized.chars().collect();
    let mut idx = 0usize;
    while idx < chars.len() {
        idx = find_auth_url_start(&chars, idx)?;
        let mut end = idx;
        let mut candidate = String::new();
        while end < chars.len() {
            let ch = chars[end];
            if is_auth_url_char(ch) {
                candidate.push(ch);
                end += 1;
                continue;
            }
            if ch.is_whitespace() {
                let mut probe = end;
                while probe < chars.len() && chars[probe].is_whitespace() {
                    probe += 1;
                }
                if probe < chars.len() && is_auth_url_char(chars[probe]) {
                    let mut token_end = probe;
                    while token_end < chars.len() && is_auth_url_char(chars[token_end]) {
                        token_end += 1;
                    }
                    let next_fragment: String = chars[probe..token_end].iter().collect();
                    if should_continue_auth_url_after_break(&candidate, &next_fragment) {
                        end = probe;
                        continue;
                    }
                }
            }
            break;
        }
        let trimmed = candidate
            .trim_matches(|c: char| {
                matches!(
                    c,
                    '"' | '\'' | '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>' | ',' | ';' | '.'
                )
            })
            .to_string();
        if let Ok(parsed) = Url::parse(&trimmed) {
            if matches!(parsed.scheme(), "http" | "https") {
                return Some(trimmed);
            }
        }
        idx = end.saturating_add(1);
    }
    None
}

pub(super) fn auth_url_looks_complete(auth_url: &str) -> bool {
    let parsed = match Url::parse(auth_url) {
        Ok(value) => value,
        Err(_) => return false,
    };
    let maybe_redirect = parsed
        .query_pairs()
        .find_map(|(key, value)| (key == "redirect_uri").then_some(value.into_owned()));
    let Some(redirect_uri) = maybe_redirect else {
        return true;
    };
    let redirect = match Url::parse(&redirect_uri) {
        Ok(value) => value,
        Err(_) => return false,
    };
    let Some(host) = redirect.host_str() else {
        return false;
    };
    if !is_loopback_host(host) {
        return false;
    }
    redirect.port().is_some()
}

pub(super) async fn read_trailing_claude_login_lines(
    line_rx: &mut mpsc::UnboundedReceiver<String>,
    grace: Duration,
) -> Vec<String> {
    let mut lines = Vec::new();
    let mut deadline = Instant::now() + grace;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, line_rx.recv()).await {
            Ok(Some(line)) => {
                lines.push(line);
                deadline = Instant::now() + grace;
            }
            Ok(None) => break,
            Err(_) => break,
        }
    }
    lines
}
