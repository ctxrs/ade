use anyhow::Result;

#[tokio::test]
async fn rovo_server_mode_smoke() -> Result<()> {
    let base_url = match std::env::var("ROVO_DEV_ACP_TEST_BASE_URL") {
        Ok(value) => value,
        Err(_) => {
            eprintln!("ROVO_DEV_ACP_TEST_BASE_URL not set; skipping");
            return Ok(());
        }
    };

    let base = base_url.trim_end_matches('/');
    let client = reqwest::Client::new();

    let health = client
        .get(format!("{base}/healthcheck"))
        .send()
        .await?;
    assert!(health.status().is_success());

    let session = client
        .post(format!("{base}/v3/sessions/create"))
        .send()
        .await?;
    assert!(session.status().is_success());

    Ok(())
}
