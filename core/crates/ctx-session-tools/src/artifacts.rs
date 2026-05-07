use std::path::Path;

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

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{infer_session_artifact_mime_type, normalize_session_artifact_name};

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
}
