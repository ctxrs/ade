use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use reqwest::Method;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::config::{resolve_daemon_config, DaemonConfig};

pub struct Client {
    pub(crate) base_url: String,
    pub(crate) auth_token: Option<String>,
    pub(crate) http: reqwest::Client,
}

impl Client {
    pub fn new(config: DaemonConfig) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("building http client")?;
        Ok(Self {
            base_url: config.base_url,
            auth_token: config.auth_token,
            http,
        })
    }

    pub fn from_env() -> Result<Self> {
        Self::new(resolve_daemon_config()?)
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub(crate) fn url_for(&self, path: &str) -> Result<String> {
        if !path.starts_with('/') {
            return Err(anyhow!("path must start with '/'"));
        }
        Ok(format!("{}{}", self.base_url, path))
    }

    pub(crate) async fn request_json<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&impl Serialize>,
    ) -> Result<T> {
        let url = self.url_for(path)?;
        let mut req = self.http.request(method, url);
        if let Some(token) = &self.auth_token {
            req = req.bearer_auth(token);
        }
        if let Some(value) = body {
            req = req.json(value);
        }
        let resp = req.send().await.context("sending request")?;
        let status = resp.status();
        let text = resp.text().await.context("reading response body")?;
        if !status.is_success() {
            let snippet = text.trim();
            let msg = if snippet.is_empty() {
                format!("request failed with status {}", status.as_u16())
            } else {
                format!(
                    "request failed with status {}: {}",
                    status.as_u16(),
                    snippet
                )
            };
            return Err(anyhow!(msg));
        }
        if text.trim().is_empty() {
            return Err(anyhow!("empty response body from {}", path));
        }
        serde_json::from_str(&text).with_context(|| format!("parsing JSON response from {}", path))
    }

    pub(crate) async fn request_empty(
        &self,
        method: Method,
        path: &str,
        body: Option<&impl Serialize>,
    ) -> Result<()> {
        let url = self.url_for(path)?;
        let mut req = self.http.request(method, url);
        if let Some(token) = &self.auth_token {
            req = req.bearer_auth(token);
        }
        if let Some(value) = body {
            req = req.json(value);
        }
        let resp = req.send().await.context("sending request")?;
        let status = resp.status();
        let text = resp.text().await.context("reading response body")?;
        if !status.is_success() {
            let snippet = text.trim();
            let msg = if snippet.is_empty() {
                format!("request failed with status {}", status.as_u16())
            } else {
                format!(
                    "request failed with status {}: {}",
                    status.as_u16(),
                    snippet
                )
            };
            return Err(anyhow!(msg));
        }
        Ok(())
    }
}
