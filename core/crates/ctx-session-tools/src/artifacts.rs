use std::path::Path;
use std::time::SystemTime;

pub const SESSION_IMAGE_BLOB_MAX_BYTES: usize = 25 * 1024 * 1024;
pub const SESSION_IMAGE_BLOB_MULTIPART_MAX_BYTES: usize = SESSION_IMAGE_BLOB_MAX_BYTES + 64 * 1024;
pub const SESSION_IMAGE_BLOB_TOO_LARGE_MESSAGE: &str =
    "Image attachments must be 25 MiB or smaller.";

pub fn normalize_session_artifact_name(name: Option<String>, path: &Path) -> Option<String> {
    if let Some(name) = name {
        let trimmed = name.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    path.file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
}

pub fn infer_session_artifact_mime_type(path: &Path, override_value: Option<String>) -> String {
    if let Some(value) = override_value {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    if path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("mdx"))
    {
        return "text/markdown".to_string();
    }
    mime_guess::from_path(path)
        .first_or_octet_stream()
        .essence_str()
        .to_string()
}

pub fn infer_session_upload_blob_mime_type(
    file_name: Option<&str>,
    override_value: Option<String>,
) -> String {
    match file_name {
        Some(name) => infer_session_artifact_mime_type(Path::new(name), override_value),
        None => override_value.unwrap_or_else(|| "application/octet-stream".to_string()),
    }
}

pub fn build_session_artifact_etag(size: u64, modified: SystemTime) -> Option<String> {
    let modified = modified.duration_since(SystemTime::UNIX_EPOCH).ok()?;
    Some(format!("\"{:x}-{:x}\"", size, modified.as_nanos()))
}

pub fn build_session_artifact_last_modified(modified: SystemTime) -> String {
    let modified = chrono::DateTime::<chrono::Utc>::from(modified);
    modified.format("%a, %d %b %Y %H:%M:%S GMT").to_string()
}

fn normalize_entity_tag(tag: &str) -> &str {
    tag.trim().strip_prefix("W/").unwrap_or(tag.trim())
}

pub fn session_artifact_if_none_match_matches(value: Option<&str>, etag: &str) -> bool {
    value.is_some_and(|raw| {
        raw.split(',').any(|part| {
            let candidate = part.trim();
            candidate == "*" || normalize_entity_tag(candidate) == normalize_entity_tag(etag)
        })
    })
}

pub fn session_artifact_if_range_allows_range_request(
    value: Option<&str>,
    etag: Option<&str>,
    last_modified: Option<&str>,
) -> bool {
    let Some(raw) = value else {
        return true;
    };
    let candidate = raw.trim();
    if candidate.starts_with('"') {
        return etag.is_some_and(|current_etag| candidate == current_etag);
    }
    if candidate.starts_with("W/") || candidate == "*" {
        return false;
    }
    let Some(current_last_modified) = last_modified else {
        return false;
    };
    let Ok(if_range_time) = chrono::DateTime::parse_from_rfc2822(candidate) else {
        return false;
    };
    let Ok(last_modified_time) = chrono::DateTime::parse_from_rfc2822(current_last_modified) else {
        return false;
    };
    last_modified_time <= if_range_time
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionArtifactRange {
    Ignore,
    Satisfiable { start: u64, end: u64 },
    Unsatisfiable,
}

fn parse_decimal_u64(raw: &str) -> Option<Result<u64, ()>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || !trimmed.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some(trimmed.parse::<u64>().map_err(|_| ()))
}

pub fn parse_session_artifact_range_header(
    range_header: Option<&str>,
    size: u64,
) -> SessionArtifactRange {
    let Some(header) = range_header else {
        return SessionArtifactRange::Ignore;
    };
    let Some((unit, range)) = header.trim().split_once('=') else {
        return SessionArtifactRange::Ignore;
    };
    if !unit.trim().eq_ignore_ascii_case("bytes") {
        return SessionArtifactRange::Ignore;
    }
    let range = range.trim();
    if range.contains(',') {
        return SessionArtifactRange::Ignore;
    }
    let Some((start_raw, end_raw)) = range.split_once('-') else {
        return SessionArtifactRange::Ignore;
    };
    if start_raw.is_empty() {
        let suffix = match parse_decimal_u64(end_raw) {
            Some(Ok(value)) => value,
            Some(Err(())) => {
                if size == 0 {
                    return SessionArtifactRange::Unsatisfiable;
                }
                return SessionArtifactRange::Satisfiable {
                    start: 0,
                    end: size.saturating_sub(1),
                };
            }
            None => {
                return SessionArtifactRange::Ignore;
            }
        };
        if suffix == 0 || size == 0 {
            return SessionArtifactRange::Unsatisfiable;
        }
        return SessionArtifactRange::Satisfiable {
            start: size.saturating_sub(suffix),
            end: size.saturating_sub(1),
        };
    }
    let start = match parse_decimal_u64(start_raw) {
        Some(Ok(value)) => value,
        Some(Err(())) => {
            return SessionArtifactRange::Unsatisfiable;
        }
        None => {
            return SessionArtifactRange::Ignore;
        }
    };
    if start >= size {
        return SessionArtifactRange::Unsatisfiable;
    }
    let end = if end_raw.is_empty() {
        size.saturating_sub(1)
    } else {
        match parse_decimal_u64(end_raw) {
            Some(Ok(value)) => value.min(size.saturating_sub(1)),
            Some(Err(())) => size.saturating_sub(1),
            None => {
                return SessionArtifactRange::Ignore;
            }
        }
    };
    if start > end {
        return SessionArtifactRange::Unsatisfiable;
    }
    SessionArtifactRange::Satisfiable { start, end }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::{Duration, SystemTime};

    use super::{
        build_session_artifact_etag, build_session_artifact_last_modified,
        infer_session_artifact_mime_type, normalize_session_artifact_name,
        parse_session_artifact_range_header, session_artifact_if_none_match_matches,
        session_artifact_if_range_allows_range_request, SessionArtifactRange,
    };

    #[test]
    fn infer_artifact_mime_type_treats_mdx_as_markdown() {
        assert_eq!(
            infer_session_artifact_mime_type(Path::new("/tmp/merge-queue-for-agents.mdx"), None),
            "text/markdown"
        );
    }

    #[test]
    fn infer_artifact_mime_type_preserves_explicit_override() {
        assert_eq!(
            infer_session_artifact_mime_type(
                Path::new("/tmp/merge-queue-for-agents.mdx"),
                Some("application/mdx".to_string())
            ),
            "application/mdx"
        );
    }

    #[test]
    fn infer_artifact_mime_type_keeps_existing_markdown_inference() {
        assert_eq!(
            infer_session_artifact_mime_type(Path::new("/tmp/notes.md"), None),
            "text/markdown"
        );
    }

    #[test]
    fn normalize_session_artifact_name_uses_trimmed_display_name() {
        assert_eq!(
            normalize_session_artifact_name(
                Some("  Display name  ".to_string()),
                Path::new("/tmp/fallback.txt")
            ),
            Some("Display name".to_string())
        );
    }

    #[test]
    fn normalize_session_artifact_name_falls_back_to_file_name() {
        assert_eq!(
            normalize_session_artifact_name(
                Some("   ".to_string()),
                Path::new("/tmp/fallback.txt")
            ),
            Some("fallback.txt".to_string())
        );
    }

    #[test]
    fn session_artifact_etag_includes_size_and_modified_nanos() {
        let modified = SystemTime::UNIX_EPOCH + Duration::from_nanos(0x20);
        assert_eq!(
            build_session_artifact_etag(0x10, modified),
            Some("\"10-20\"".to_string())
        );
    }

    #[test]
    fn session_artifact_last_modified_uses_http_date_shape() {
        let modified = SystemTime::UNIX_EPOCH;
        assert_eq!(
            build_session_artifact_last_modified(modified),
            "Thu, 01 Jan 1970 00:00:00 GMT"
        );
    }

    #[test]
    fn session_artifact_if_none_match_matches_strong_weak_and_wildcard_tags() {
        assert!(session_artifact_if_none_match_matches(
            Some("\"other\", W/\"abc\""),
            "\"abc\""
        ));
        assert!(session_artifact_if_none_match_matches(Some("*"), "\"abc\""));
        assert!(!session_artifact_if_none_match_matches(
            Some("\"other\""),
            "\"abc\""
        ));
    }

    #[test]
    fn session_artifact_if_range_accepts_current_strong_etag_only() {
        assert!(session_artifact_if_range_allows_range_request(
            Some("\"abc\""),
            Some("\"abc\""),
            None
        ));
        assert!(!session_artifact_if_range_allows_range_request(
            Some("W/\"abc\""),
            Some("\"abc\""),
            None
        ));
    }

    #[test]
    fn session_artifact_if_range_accepts_new_enough_date() {
        assert!(session_artifact_if_range_allows_range_request(
            Some("Thu, 01 Jan 1970 00:00:01 GMT"),
            None,
            Some("Thu, 01 Jan 1970 00:00:00 GMT")
        ));
        assert!(!session_artifact_if_range_allows_range_request(
            Some("Wed, 31 Dec 1969 23:59:59 GMT"),
            None,
            Some("Thu, 01 Jan 1970 00:00:00 GMT")
        ));
    }

    #[test]
    fn parse_session_artifact_range_header_parses_bounded_and_suffix_ranges() {
        assert_eq!(
            parse_session_artifact_range_header(Some("bytes=2-4"), 10),
            SessionArtifactRange::Satisfiable { start: 2, end: 4 }
        );
        assert_eq!(
            parse_session_artifact_range_header(Some("bytes=-3"), 10),
            SessionArtifactRange::Satisfiable { start: 7, end: 9 }
        );
    }

    #[test]
    fn parse_session_artifact_range_header_rejects_unsatisfiable_ranges() {
        assert_eq!(
            parse_session_artifact_range_header(Some("bytes=10-20"), 10),
            SessionArtifactRange::Unsatisfiable
        );
        assert_eq!(
            parse_session_artifact_range_header(Some("bytes=-0"), 10),
            SessionArtifactRange::Unsatisfiable
        );
    }

    #[test]
    fn parse_session_artifact_range_header_ignores_unsupported_shapes() {
        assert_eq!(
            parse_session_artifact_range_header(Some("items=1-2"), 10),
            SessionArtifactRange::Ignore
        );
        assert_eq!(
            parse_session_artifact_range_header(Some("bytes=1-2,4-5"), 10),
            SessionArtifactRange::Ignore
        );
    }
}
