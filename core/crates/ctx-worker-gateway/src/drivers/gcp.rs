use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use async_trait::async_trait;
use gcp_auth::{provider, TokenProvider};
use reqwest::Method;
use serde_json::{json, Value};
use tokio::sync::RwLock;

use ctx_worker_protocol::{RepoSpec, SshInfo, StartWorkerRequest};

use crate::bootstrap::{render_bootstrap_script, BootstrapSpec};

use super::WorkerDriver;

pub struct GcpConfig {
    pub project_id: String,
    pub zone: String,
    pub machine_type: String,
    pub image: String,
    pub network: Option<String>,
    pub subnetwork: Option<String>,
    pub service_account: Option<String>,
    pub scopes: Vec<String>,
    pub disk_size_gb: i64,
    pub disk_type: String,
    pub ssh_user: Option<String>,
    pub delete_disk_on_pause: bool,
    pub worker_shim_url: String,
    pub mount_path: String,
    pub workdir: String,
}

pub struct GcpDriver {
    auth: Arc<dyn TokenProvider>,
    http: reqwest::Client,
    config: GcpConfig,
    auth_token: Option<String>,
    state: RwLock<HashMap<String, GcpWorkerState>>,
}

#[derive(Clone, Default)]
struct GcpWorkerState {
    instance_name: Option<String>,
    disk_name: Option<String>,
    snapshot_name: Option<String>,
}

impl GcpDriver {
    pub async fn new(config: GcpConfig, auth_token: Option<String>) -> Result<Self> {
        let auth = provider().await.context("gcp auth init")?;
        Ok(Self {
            auth,
            http: reqwest::Client::new(),
            config,
            auth_token,
            state: RwLock::new(HashMap::new()),
        })
    }

    fn zone(&self) -> &str {
        &self.config.zone
    }

    fn project(&self) -> &str {
        &self.config.project_id
    }

    fn api_base(&self) -> String {
        format!(
            "https://compute.googleapis.com/compute/v1/projects/{}",
            self.project()
        )
    }

    fn region(&self) -> String {
        let zone = self.zone();
        if let Some(pos) = zone.rfind('-') {
            zone[..pos].to_string()
        } else {
            zone.to_string()
        }
    }

    fn machine_type_url(&self) -> String {
        if self.config.machine_type.contains('/') {
            self.config.machine_type.clone()
        } else {
            format!(
                "{}/zones/{}/machineTypes/{}",
                self.api_base(),
                self.zone(),
                self.config.machine_type
            )
        }
    }

    fn image_url(&self) -> String {
        if self.config.image.starts_with("projects/") || self.config.image.starts_with("https://") {
            self.config.image.clone()
        } else {
            format!(
                "projects/{}/global/images/{}",
                self.project(),
                self.config.image
            )
        }
    }

    fn disk_type_url(&self) -> String {
        if self.config.disk_type.contains('/') {
            self.config.disk_type.clone()
        } else {
            format!(
                "{}/zones/{}/diskTypes/{}",
                self.api_base(),
                self.zone(),
                self.config.disk_type
            )
        }
    }

    fn network_url(&self) -> String {
        if let Some(network) = &self.config.network {
            if network.starts_with("projects/") || network.starts_with("https://") {
                network.clone()
            } else {
                format!("{}/global/networks/{}", self.api_base(), network)
            }
        } else {
            format!("{}/global/networks/default", self.api_base())
        }
    }

    fn subnetwork_url(&self) -> Option<String> {
        let Some(subnetwork) = &self.config.subnetwork else {
            return None;
        };
        if subnetwork.starts_with("projects/") || subnetwork.starts_with("https://") {
            Some(subnetwork.clone())
        } else {
            Some(format!(
                "{}/regions/{}/subnetworks/{}",
                self.api_base(),
                self.region(),
                subnetwork
            ))
        }
    }

    async fn token(&self) -> Result<String> {
        let scopes = if self.config.scopes.is_empty() {
            vec!["https://www.googleapis.com/auth/cloud-platform"]
        } else {
            self.config.scopes.iter().map(|s| s.as_str()).collect()
        };
        let token = self.auth.token(&scopes).await.context("gcp token")?;
        Ok(token.as_str().to_string())
    }

    async fn request(&self, method: Method, url: &str, body: Option<Value>) -> Result<Value> {
        let token = self.token().await?;
        let mut req = self.http.request(method, url).bearer_auth(token);
        if let Some(body) = body {
            req = req.json(&body);
        }
        let resp = req.send().await.context("gcp request")?;
        let status = resp.status();
        let text = resp.text().await.context("gcp response text")?;
        if !status.is_success() {
            anyhow::bail!("gcp request failed: {status} {text}");
        }
        if text.trim().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&text).context("gcp response json")
    }

    async fn wait_zone_operation(&self, op_name: &str) -> Result<()> {
        let url = format!(
            "{}/zones/{}/operations/{}",
            self.api_base(),
            self.zone(),
            op_name
        );
        for _ in 0..120 {
            let resp = self.request(Method::GET, &url, None).await?;
            let status = resp.get("status").and_then(|v| v.as_str());
            if status == Some("DONE") {
                if let Some(error) = resp.get("error") {
                    anyhow::bail!("gcp operation error: {error}");
                }
                return Ok(());
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
        anyhow::bail!("gcp operation timeout")
    }

    fn sanitize_name(prefix: &str, worker_id: &str) -> String {
        let raw = format!("{}-{}", prefix, worker_id);
        let mut out = String::new();
        for ch in raw.chars() {
            let ch = ch.to_ascii_lowercase();
            if ch.is_ascii_alphanumeric() || ch == '-' {
                out.push(ch);
            } else {
                out.push('-');
            }
        }
        if !out
            .chars()
            .next()
            .map(|c| c.is_ascii_alphabetic())
            .unwrap_or(false)
        {
            out.insert(0, 'a');
        }
        if out.len() > 63 {
            out.truncate(63);
        }
        while out.ends_with('-') {
            out.pop();
        }
        out
    }

    async fn create_disk(&self, name: &str, source_snapshot: Option<&str>) -> Result<()> {
        let url = format!("{}/zones/{}/disks", self.api_base(), self.zone());
        let mut body = json!({
            "name": name,
            "sizeGb": self.config.disk_size_gb,
            "type": self.disk_type_url(),
        });
        if let Some(snapshot) = source_snapshot {
            let Some(obj) = body.as_object_mut() else {
                return Err(anyhow::anyhow!("invalid gcp disk body shape"));
            };
            obj.insert(
                "sourceSnapshot".to_string(),
                Value::String(snapshot.to_string()),
            );
        }
        let resp = self.request(Method::POST, &url, Some(body)).await?;
        let op_name = resp
            .get("name")
            .and_then(|v| v.as_str())
            .context("missing disk op name")?;
        self.wait_zone_operation(op_name).await
    }

    async fn disk_exists(&self, name: &str) -> Result<bool> {
        let url = format!("{}/zones/{}/disks/{}", self.api_base(), self.zone(), name);
        let token = self.token().await?;
        let resp = self
            .http
            .get(&url)
            .bearer_auth(token)
            .send()
            .await
            .context("gcp disk exists")?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(false);
        }
        Ok(resp.status().is_success())
    }

    async fn delete_disk(&self, name: &str) -> Result<()> {
        let url = format!("{}/zones/{}/disks/{}", self.api_base(), self.zone(), name);
        let resp = self.request(Method::DELETE, &url, None).await?;
        let op_name = resp
            .get("name")
            .and_then(|v| v.as_str())
            .context("missing delete disk op")?;
        self.wait_zone_operation(op_name).await
    }

    async fn create_snapshot(&self, disk_name: &str, snapshot_name: &str) -> Result<()> {
        let url = format!(
            "{}/zones/{}/disks/{}/createSnapshot",
            self.api_base(),
            self.zone(),
            disk_name
        );
        let body = json!({
            "name": snapshot_name,
        });
        let resp = self.request(Method::POST, &url, Some(body)).await?;
        let op_name = resp
            .get("name")
            .and_then(|v| v.as_str())
            .context("missing snapshot op name")?;
        self.wait_zone_operation(op_name).await
    }

    async fn delete_instance(&self, name: &str) -> Result<()> {
        let url = format!(
            "{}/zones/{}/instances/{}",
            self.api_base(),
            self.zone(),
            name
        );
        let resp = self.request(Method::DELETE, &url, None).await?;
        let op_name = resp
            .get("name")
            .and_then(|v| v.as_str())
            .context("missing delete instance op")?;
        self.wait_zone_operation(op_name).await
    }

    async fn create_instance(
        &self,
        worker_id: &str,
        name: &str,
        disk_name: &str,
        spec: &StartWorkerRequest,
        base_commit_sha: &str,
        gateway_url: &str,
    ) -> Result<()> {
        let device_by_id = format!("/dev/disk/by-id/google-{disk_name}");
        let bootstrap = BootstrapSpec {
            worker_id,
            gateway_url,
            gateway_token: self.auth_token.as_deref(),
            base_commit: base_commit_sha,
            diff_debounce_ms: spec.diff_debounce_ms.unwrap_or(1500),
            repo: &spec.repo,
            provider_id: spec.provider_id.as_deref(),
            env: &spec.env,
            shim_url: &self.config.worker_shim_url,
            workdir: &self.config.workdir,
            mount_path: &self.config.mount_path,
            mount_device_candidates: vec![device_by_id],
        };
        let startup_script = render_bootstrap_script(&bootstrap);

        let mut network_interface = json!({
            "network": self.network_url(),
            "accessConfigs": [{
                "name": "External NAT",
                "type": "ONE_TO_ONE_NAT"
            }]
        });
        if let Some(subnetwork) = self.subnetwork_url() {
            let Some(obj) = network_interface.as_object_mut() else {
                return Err(anyhow::anyhow!("invalid gcp network interface shape"));
            };
            obj.insert("subnetwork".to_string(), Value::String(subnetwork));
        }

        let mut instance = json!({
            "name": name,
            "machineType": self.machine_type_url(),
            "disks": [
                {
                    "boot": true,
                    "autoDelete": true,
                    "initializeParams": {
                        "sourceImage": self.image_url()
                    }
                },
                {
                    "boot": false,
                    "autoDelete": false,
                    "source": format!("{}/zones/{}/disks/{}", self.api_base(), self.zone(), disk_name),
                    "deviceName": disk_name
                }
            ],
            "metadata": {
                "items": [
                    { "key": "startup-script", "value": startup_script }
                ]
            },
            "networkInterfaces": [network_interface],
            "labels": {
                "ctx-worker": "true"
            }
        });

        if let Some(service_account) = &self.config.service_account {
            let scopes = if self.config.scopes.is_empty() {
                vec!["https://www.googleapis.com/auth/cloud-platform".to_string()]
            } else {
                self.config.scopes.clone()
            };
            let Some(obj) = instance.as_object_mut() else {
                return Err(anyhow::anyhow!("invalid gcp instance body shape"));
            };
            obj.insert(
                "serviceAccounts".to_string(),
                json!([{
                    "email": service_account,
                    "scopes": scopes,
                }]),
            );
        }

        let url = format!("{}/zones/{}/instances", self.api_base(), self.zone());
        let resp = self.request(Method::POST, &url, Some(instance)).await?;
        let op_name = resp
            .get("name")
            .and_then(|v| v.as_str())
            .context("missing instance op name")?;
        self.wait_zone_operation(op_name).await
    }

    async fn fetch_instance_ip(&self, name: &str) -> Result<Option<String>> {
        let url = format!(
            "{}/zones/{}/instances/{}",
            self.api_base(),
            self.zone(),
            name
        );
        let resp = self.request(Method::GET, &url, None).await?;
        let ip = resp
            .get("networkInterfaces")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|v| v.get("accessConfigs"))
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|v| v.get("natIP"))
            .and_then(|v| v.as_str())
            .map(|v| v.to_string());
        Ok(ip)
    }
}

#[async_trait]
impl WorkerDriver for GcpDriver {
    async fn start(
        &self,
        worker_id: &str,
        spec: &StartWorkerRequest,
        base_commit_sha: &str,
        gateway_url: &str,
    ) -> Result<Option<SshInfo>> {
        if matches!(spec.repo, RepoSpec::Local { .. }) {
            anyhow::bail!("gcp driver does not support local repo specs");
        }

        let instance_name = Self::sanitize_name("ctx-worker", worker_id);
        let disk_name = Self::sanitize_name("ctx-session", worker_id);

        self.create_disk(&disk_name, None).await?;
        {
            let mut state = self.state.write().await;
            state.insert(
                worker_id.to_string(),
                GcpWorkerState {
                    instance_name: Some(instance_name.clone()),
                    disk_name: Some(disk_name.clone()),
                    snapshot_name: None,
                },
            );
        }
        self.create_instance(
            worker_id,
            &instance_name,
            &disk_name,
            spec,
            base_commit_sha,
            gateway_url,
        )
        .await?;
        let public_ip = self.fetch_instance_ip(&instance_name).await.ok().flatten();

        let ssh = self.config.ssh_user.as_ref().and_then(|user| {
            public_ip.as_ref().map(|host| SshInfo {
                host: host.clone(),
                port: 22,
                user: user.clone(),
                fingerprint: None,
            })
        });

        let mut state = self.state.write().await;
        if let Some(entry) = state.get_mut(worker_id) {
            entry.instance_name = Some(instance_name);
            entry.disk_name = Some(disk_name);
        }

        Ok(ssh)
    }

    async fn stop(&self, worker_id: &str) -> Result<()> {
        let mut state = self.state.write().await;
        if let Some(entry) = state.get_mut(worker_id) {
            if let Some(name) = entry.instance_name.take() {
                let _ = self.delete_instance(&name).await;
            }
            if let Some(disk) = entry.disk_name.take() {
                let _ = self.delete_disk(&disk).await;
            }
            if let Some(snapshot) = entry.snapshot_name.take() {
                let url = format!("{}/global/snapshots/{}", self.api_base(), snapshot);
                let _ = self.request(Method::DELETE, &url, None).await;
            }
        }
        Ok(())
    }

    async fn pause(&self, worker_id: &str) -> Result<()> {
        let mut state = self.state.write().await;
        let entry = state.get_mut(worker_id).context("missing worker state")?;
        let Some(disk_name) = entry.disk_name.clone() else {
            anyhow::bail!("missing disk name");
        };
        let snapshot_name = Self::sanitize_name("ctx-snap", worker_id);
        self.create_snapshot(&disk_name, &snapshot_name).await?;
        entry.snapshot_name = Some(snapshot_name);

        if let Some(instance_name) = entry.instance_name.take() {
            let _ = self.delete_instance(&instance_name).await;
        }

        if self.config.delete_disk_on_pause {
            if let Some(disk_name) = entry.disk_name.take() {
                let _ = self.delete_disk(&disk_name).await;
            }
        }

        Ok(())
    }

    async fn resume(
        &self,
        worker_id: &str,
        spec: &StartWorkerRequest,
        base_commit_sha: &str,
        gateway_url: &str,
    ) -> Result<Option<SshInfo>> {
        if matches!(spec.repo, RepoSpec::Local { .. }) {
            anyhow::bail!("gcp driver does not support local repo specs");
        }

        let instance_name = Self::sanitize_name("ctx-worker", worker_id);
        let disk_name = Self::sanitize_name("ctx-session", worker_id);

        if !self.disk_exists(&disk_name).await.unwrap_or(false) {
            let snapshot_name = Self::sanitize_name("ctx-snap", worker_id);
            let snapshot_url = format!("{}/global/snapshots/{}", self.api_base(), snapshot_name);
            self.create_disk(&disk_name, Some(&snapshot_url)).await?;
        }

        self.create_instance(
            worker_id,
            &instance_name,
            &disk_name,
            spec,
            base_commit_sha,
            gateway_url,
        )
        .await?;
        let public_ip = self.fetch_instance_ip(&instance_name).await.ok().flatten();

        let ssh = self.config.ssh_user.as_ref().and_then(|user| {
            public_ip.as_ref().map(|host| SshInfo {
                host: host.clone(),
                port: 22,
                user: user.clone(),
                fingerprint: None,
            })
        });

        let mut state = self.state.write().await;
        state.insert(
            worker_id.to_string(),
            GcpWorkerState {
                instance_name: Some(instance_name),
                disk_name: Some(disk_name),
                snapshot_name: Some(Self::sanitize_name("ctx-snap", worker_id)),
            },
        );

        Ok(ssh)
    }
}
