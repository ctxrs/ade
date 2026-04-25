use super::*;

#[test]
fn daemon_request_error_includes_method_and_url_context() {
    let manager = ConnectionManager::default();
    manager.set_local_attached(
        "http://127.0.0.1:65535".to_string(),
        "token".to_string(),
        None,
        LocalConnectionSource::ExistingCompatibleDaemon,
    );
    let err = manager
        .daemon_request(DesktopDaemonRequest {
            method: "GET".to_string(),
            path: "/api/health".to_string(),
            body: None,
            headers: Vec::new(),
        })
        .expect_err("request should fail on closed port");
    let message = format!("{err:#}");
    assert!(
        message.contains("sending request GET http://127.0.0.1:65535/api/health"),
        "expected method/url context in error, got: {message}"
    );
}

#[test]
fn daemon_request_reuses_connection_http_client() {
    reset_connection_http_client_build_count();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind test listener");
    let addr = listener.local_addr().expect("listener addr");
    let server = std::thread::spawn(move || {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().expect("accept request");
            let mut buf = [0_u8; 1024];
            let _ = std::io::Read::read(&mut stream, &mut buf);
            std::io::Write::write_all(
                &mut stream,
                b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{}",
            )
            .expect("write response");
        }
    });

    let manager = ConnectionManager::default();
    manager.set_local_attached(
        format!("http://{}", addr),
        "token".to_string(),
        None,
        LocalConnectionSource::EnvOverride,
    );

    for _ in 0..2 {
        let response = manager
            .daemon_request(DesktopDaemonRequest {
                method: "GET".to_string(),
                path: "/api/health".to_string(),
                body: None,
                headers: Vec::new(),
            })
            .expect("daemon request succeeds");
        assert_eq!(response.status, 200);
    }

    server.join().expect("join test server");
    assert_eq!(connection_http_client_build_count(), 1);
}
