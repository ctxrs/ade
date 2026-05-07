use http::Method;

pub fn mobile_secure_proxy_allows_request(method: &Method, path: &str) -> bool {
    if *method == Method::GET && path == "/api/health" {
        return true;
    }
    if *method == Method::GET && path == "/api/workspaces" {
        return true;
    }
    if *method == Method::GET {
        if let Some(workspace_id) = path.strip_prefix("/api/workspaces/") {
            return !workspace_id.is_empty() && !workspace_id.contains('/');
        }
    }
    false
}

pub fn secure_proxy_path_is_unnormalized(path: &str) -> bool {
    if path.contains('%') {
        return true;
    }
    let mut saw_leading = false;
    for segment in path.split('/') {
        if !saw_leading {
            saw_leading = true;
            if !segment.is_empty() {
                return true;
            }
            continue;
        }
        if segment.is_empty() || segment == "." || segment == ".." {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::{mobile_secure_proxy_allows_request, secure_proxy_path_is_unnormalized};
    use http::Method;

    #[test]
    fn mobile_secure_proxy_allows_only_workspace_read_routes() {
        assert!(mobile_secure_proxy_allows_request(
            &Method::GET,
            "/api/health"
        ));
        assert!(mobile_secure_proxy_allows_request(
            &Method::GET,
            "/api/workspaces"
        ));
        assert!(mobile_secure_proxy_allows_request(
            &Method::GET,
            "/api/workspaces/00000000-0000-0000-0000-000000000001"
        ));

        assert!(!mobile_secure_proxy_allows_request(
            &Method::POST,
            "/api/workspaces"
        ));
        assert!(!mobile_secure_proxy_allows_request(
            &Method::GET,
            "/api/workspaces/00000000-0000-0000-0000-000000000001/tasks"
        ));
        assert!(!mobile_secure_proxy_allows_request(
            &Method::GET,
            "/api/providers"
        ));
    }

    #[test]
    fn secure_proxy_rejects_unnormalized_paths() {
        assert!(!secure_proxy_path_is_unnormalized("/api/workspaces"));
        assert!(!secure_proxy_path_is_unnormalized(
            "/api/workspaces/00000000-0000-0000-0000-000000000001"
        ));

        assert!(secure_proxy_path_is_unnormalized("api/workspaces"));
        assert!(secure_proxy_path_is_unnormalized("/api//workspaces"));
        assert!(secure_proxy_path_is_unnormalized("/api/./workspaces"));
        assert!(secure_proxy_path_is_unnormalized("/api/../providers"));
        assert!(secure_proxy_path_is_unnormalized("/api/%2e%2e/providers"));
    }
}
