use anyhow::Context;
use url::Url;

pub fn validate_web_session_url(raw: &str) -> anyhow::Result<()> {
    let parsed = Url::parse(raw).context("url must be an absolute URL")?;
    if !matches!(parsed.scheme(), "http" | "https") {
        anyhow::bail!("url must use http:// or https://");
    }
    if parsed.host_str().is_none() {
        anyhow::bail!("url must include host");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_web_session_url;

    #[test]
    fn validate_web_session_url_accepts_http_and_https() {
        validate_web_session_url("http://127.0.0.1:3000").expect("http URL");
        validate_web_session_url("https://example.com/path").expect("https URL");
    }

    #[test]
    fn validate_web_session_url_rejects_relative_url() {
        let error = validate_web_session_url("/workbench").expect_err("relative URL");
        assert!(format!("{error:#}").contains("absolute URL"));
    }

    #[test]
    fn validate_web_session_url_rejects_non_http_scheme() {
        let error = validate_web_session_url("file:///tmp/index.html").expect_err("file URL");
        assert!(format!("{error:#}").contains("http:// or https://"));
    }
}
