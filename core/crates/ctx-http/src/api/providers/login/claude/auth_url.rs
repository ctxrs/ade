use super::*;

pub(super) const CLAUDE_BROWSER_OPEN_MARKER: &str = "CTX_CLAUDE_AUTH_URL:";
pub(super) const CLAUDE_UNSUPPORTED_MANUAL_FALLBACK_ERROR: &str =
    "Claude setup-token fell back to manual code entry, which ctx does not support. Browser launch likely failed before Claude could receive the localhost callback.";

pub(super) fn claude_login_hit_unsupported_manual_fallback(text: &str) -> bool {
    text.to_ascii_lowercase()
        .contains("browser didn't open? use the url below to sign in")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ClaudeAuthUrlSource {
    BrowserOpenCapture,
    BrowserOpenMarker,
    Transcript,
}

fn extract_claude_browser_open_marker_url(text: &str) -> Option<String> {
    for raw_line in text.lines() {
        let line = raw_line.trim();
        let Some(marker_idx) = line.find(CLAUDE_BROWSER_OPEN_MARKER) else {
            continue;
        };
        let candidate = line[marker_idx + CLAUDE_BROWSER_OPEN_MARKER.len()..].trim();
        if candidate.is_empty() {
            continue;
        }
        if Url::parse(candidate).is_ok() {
            return Some(candidate.to_string());
        }
        if let Some(parsed) = extract_auth_url(candidate) {
            return Some(parsed);
        }
    }
    None
}

pub(super) fn extract_preferred_claude_auth_url(
    text: &str,
) -> Option<(String, ClaudeAuthUrlSource)> {
    if let Some(marker_url) = extract_claude_browser_open_marker_url(text) {
        return Some((marker_url, ClaudeAuthUrlSource::BrowserOpenMarker));
    }
    extract_auth_url(text).map(|value| (value, ClaudeAuthUrlSource::Transcript))
}

pub(super) fn should_replace_observed_claude_auth_url(
    current: Option<&str>,
    candidate: &str,
    source: ClaudeAuthUrlSource,
) -> bool {
    match source {
        ClaudeAuthUrlSource::BrowserOpenCapture | ClaudeAuthUrlSource::BrowserOpenMarker => {
            current != Some(candidate)
        }
        ClaudeAuthUrlSource::Transcript => match current {
            None => true,
            Some(existing) => {
                !auth_url_looks_complete(existing) && candidate.len() >= existing.len()
            }
        },
    }
}

pub(super) fn read_claude_browser_open_capture_url(path: &std::path::Path) -> Option<String> {
    let raw = std::fs::read_to_string(path).ok()?;
    let candidate = raw.trim();
    if candidate.is_empty() {
        return None;
    }
    if Url::parse(candidate).is_ok() {
        return Some(candidate.to_string());
    }
    extract_auth_url(candidate)
}

pub(super) fn claude_manual_fallback_is_terminal(
    text: &str,
    browser_open_capture_path: &std::path::Path,
) -> bool {
    claude_login_hit_unsupported_manual_fallback(text)
        && read_claude_browser_open_capture_url(browser_open_capture_path).is_none()
}

pub(super) fn refresh_claude_auth_url_from_capture_path(
    observed_auth_url: &mut Option<String>,
    capture_path: &std::path::Path,
) -> bool {
    let Some(candidate) = read_claude_browser_open_capture_url(capture_path) else {
        return false;
    };
    if should_replace_observed_claude_auth_url(
        observed_auth_url.as_deref(),
        &candidate,
        ClaudeAuthUrlSource::BrowserOpenCapture,
    ) {
        *observed_auth_url = Some(candidate);
        return true;
    }
    false
}

fn is_claude_setup_token_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'
}

fn is_setup_token_fragment(value: &str) -> bool {
    !value.is_empty() && value.chars().all(is_claude_setup_token_char)
}

fn leading_setup_token_fragment(value: &str) -> &str {
    let mut end = 0usize;
    for (idx, ch) in value.char_indices() {
        if is_claude_setup_token_char(ch) {
            end = idx + ch.len_utf8();
            continue;
        }
        break;
    }
    &value[..end]
}

fn is_setup_token_continuation_fragment(value: &str) -> bool {
    if !is_setup_token_fragment(value) {
        return false;
    }
    value
        .chars()
        .any(|ch| ch.is_ascii_digit() || ch == '-' || ch == '_')
}

fn trim_known_setup_token_prose_suffix(token: &str) -> String {
    const PROSE_CANONICAL: &str = "StorethistokensecurelyYouwontbeabletoseeitagain";
    const MIN_MATCH_LEN: usize = 5;

    let mut out = token.to_string();
    for phrase in [
        PROSE_CANONICAL,
        "storethistokensecurelyyouwontbeabletoseeitagain",
    ] {
        let max = std::cmp::min(out.len(), phrase.len());
        let mut truncate_at: Option<usize> = None;
        for len in (MIN_MATCH_LEN..=max).rev() {
            if out.ends_with(&phrase[..len]) {
                truncate_at = Some(out.len() - len);
                break;
            }
        }
        if let Some(idx) = truncate_at {
            out.truncate(idx);
        }
    }
    out
}

pub(super) fn extract_claude_setup_token(output: &str) -> Option<String> {
    let lines: Vec<&str> = output.lines().collect();
    for (idx, line) in lines.iter().enumerate() {
        let Some(start) = line.find("sk-ant-oat") else {
            continue;
        };
        let first_fragment = leading_setup_token_fragment(&line[start..]);
        if first_fragment.is_empty() {
            continue;
        }
        let mut token = first_fragment.to_string();
        for next in lines.iter().skip(idx + 1) {
            let trimmed = next.trim();
            if trimmed.is_empty() {
                break;
            }
            let fragment = leading_setup_token_fragment(trimmed);
            if !is_setup_token_continuation_fragment(fragment) {
                break;
            }
            token.push_str(fragment);
        }
        let cleaned = trim_known_setup_token_prose_suffix(&token);
        if cleaned.len() > 40 {
            return Some(cleaned);
        }
    }
    None
}
