use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context, Result};
use async_trait::async_trait;
use azure_core::auth::TokenCredential;
use azure_identity::{DefaultAzureCredential, TokenCredentialOptions};
use base64::Engine;
use reqwest::Method;
use serde_json::{json, Value};
use tokio::sync::RwLock;

use ctx_worker_protocol::{RepoSpec, SshInfo, StartWorkerRequest};

use crate::bootstrap::{render_bootstrap_script, BootstrapSpec};

use super::WorkerDriver;

pub struct AzureConfig {
    pub subscription_id: String,
    pub resource_group: String,
    pub location: String,
    pub vm_size: String,
    pub image: String,
    pub vnet: String,
    pub subnet: String,
    pub admin_username: String,
    pub ssh_public_key: String,
    pub disk_size_gb: i32,
    pub disk_sku: String,
    pub delete_disk_on_pause: bool,
    pub use_public_ip: bool,
    pub worker_shim_url: String,
    pub mount_path: String,
    pub workdir: String,
}

pub struct AzureDriver {
    credential: DefaultAzureCredential,
    http: reqwest::Client,
    config: AzureConfig,
    auth_token: Option<String>,
    state: RwLock<HashMap<String, AzureWorkerState>>,
}

#[derive(Clone, Default)]
struct AzureWorkerState {
    vm_name: Option<String>,
    disk_name: Option<String>,
    snapshot_name: Option<String>,
    nic_name: Option<String>,
    public_ip_name: Option<String>,
}

struct CreateVmParams<'a> {
    worker_id: &'a str,
    vm_name: &'a str,
    nic_name: &'a str,
    disk_name: &'a str,
    spec: &'a StartWorkerRequest,
    base_commit_sha: &'a str,
    gateway_url: &'a str,
}

impl AzureDriver {
    pub async fn new(config: AzureConfig, auth_token: Option<String>) -> Result<Self> {
        Ok(Self {
            credential: DefaultAzureCredential::create(TokenCredentialOptions::default())
                .context("azure credential init")?,
            http: reqwest::Client::new(),
            config,
            auth_token,
            state: RwLock::new(HashMap::new()),
        })
    }

    fn api_base(&self) -> String {
        format!(
            "https://management.azure.com/subscriptions/{}",
            self.config.subscription_id
        )
    }

    fn rg(&self) -> &str {
        &self.config.resource_group
    }

    fn subnet_id(&self) -> String {
        format!(
            "{}/resourceGroups/{}/providers/Microsoft.Network/virtualNetworks/{}/subnets/{}",
            self.api_base(),
            self.rg(),
            self.config.vnet,
            self.config.subnet
        )
    }

    fn disk_id(&self, name: &str) -> String {
        format!(
            "{}/resourceGroups/{}/providers/Microsoft.Compute/disks/{}",
            self.api_base(),
            self.rg(),
            name
        )
    }

    fn snapshot_id(&self, name: &str) -> String {
        format!(
            "{}/resourceGroups/{}/providers/Microsoft.Compute/snapshots/{}",
            self.api_base(),
            self.rg(),
            name
        )
    }

    fn nic_id(&self, name: &str) -> String {
        format!(
            "{}/resourceGroups/{}/providers/Microsoft.Network/networkInterfaces/{}",
            self.api_base(),
            self.rg(),
            name
        )
    }

    fn public_ip_id(&self, name: &str) -> String {
        format!(
            "{}/resourceGroups/{}/providers/Microsoft.Network/publicIPAddresses/{}",
            self.api_base(),
            self.rg(),
            name
        )
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
        if out.len() > 63 {
            out.truncate(63);
        }
        while out.ends_with('-') {
            out.pop();
        }
        out
    }

    async fn token(&self) -> Result<String> {
        let token = self
            .credential
            .get_token(&["https://management.azure.com/.default"])
            .await
            .context("azure token")?;
        Ok(token.token.secret().to_string())
    }

    async fn request(&self, method: Method, url: &str, body: Option<Value>) -> Result<Value> {
        let token = self.token().await?;
        let mut req = self.http.request(method, url).bearer_auth(token);
        if let Some(body) = body {
            req = req.json(&body);
        }
        let resp = req.send().await.context("azure request")?;
        let status = resp.status();
        let text = resp.text().await.context("azure response text")?;
        if !status.is_success() {
            anyhow::bail!("azure request failed: {status} {text}");
        }
        if text.trim().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&text).context("azure response json")
    }

    async fn wait_resource(&self, url: &str) -> Result<()> {
        for _ in 0..120 {
            let resp = self.request(Method::GET, url, None).await?;
            let status = resp
                .get("properties")
                .and_then(|v| v.get("provisioningState"))
                .and_then(|v| v.as_str());
            if status == Some("Succeeded") {
                return Ok(());
            }
            if status == Some("Failed") {
                anyhow::bail!("azure provisioning failed");
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
        anyhow::bail!("azure provisioning timeout")
    }

    async fn delete_resource(&self, url: &str) -> Result<()> {
        let _ = self.request(Method::DELETE, url, None).await;
        Ok(())
    }

    fn image_reference(&self) -> Value {
        if self.config.image.starts_with("/subscriptions/") {
            json!({ "id": self.config.image })
        } else if self.config.image.contains(':') {
            let parts: Vec<&str> = self.config.image.split(':').collect();
            if parts.len() == 4 {
                json!({
                    "publisher": parts[0],
                    "offer": parts[1],
                    "sku": parts[2],
                    "version": parts[3],
                })
            } else {
                json!({ "id": self.config.image })
            }
        } else {
            json!({ "id": self.config.image })
        }
    }

    async fn create_public_ip(&self, name: &str) -> Result<()> {
        let url = format!(
            "{}/resourceGroups/{}/providers/Microsoft.Network/publicIPAddresses/{}?api-version=2023-09-01",
            self.api_base(),
            self.rg(),
            name
        );
        let body = json!({
            "location": self.config.location.clone(),
            "sku": { "name": "Standard" },
            "properties": { "publicIPAllocationMethod": "Dynamic" }
        });
        self.request(Method::PUT, &url, Some(body)).await?;
        self.wait_resource(&url).await
    }

    async fn create_nic(&self, name: &str, public_ip_name: Option<&str>) -> Result<()> {
        let url = format!(
            "{}/resourceGroups/{}/providers/Microsoft.Network/networkInterfaces/{}?api-version=2023-09-01",
            self.api_base(),
            self.rg(),
            name
        );
        let mut ip_config = json!({
            "name": "ipconfig1",
            "properties": {
                "subnet": { "id": self.subnet_id() }
            }
        });
        if let Some(public_ip_name) = public_ip_name {
            ip_config
                .get_mut("properties")
                .and_then(|v| v.as_object_mut())
                .expect("ipconfig props")
                .insert(
                    "publicIPAddress".to_string(),
                    json!({ "id": self.public_ip_id(public_ip_name) }),
                );
        }

        let body = json!({
            "location": self.config.location.clone(),
            "properties": {
                "ipConfigurations": [ip_config]
            }
        });
        self.request(Method::PUT, &url, Some(body)).await?;
        self.wait_resource(&url).await
    }

    async fn create_disk(&self, name: &str, snapshot_id: Option<String>) -> Result<()> {
        let url = format!(
            "{}/resourceGroups/{}/providers/Microsoft.Compute/disks/{}?api-version=2023-03-01",
            self.api_base(),
            self.rg(),
            name
        );
        let mut creation = json!({
            "createOption": "Empty"
        });
        if let Some(source) = snapshot_id {
            creation = json!({
                "createOption": "Copy",
                "sourceResourceId": source
            });
        }
        let body = json!({
            "location": self.config.location.clone(),
            "sku": { "name": self.config.disk_sku.clone() },
            "properties": {
                "diskSizeGB": self.config.disk_size_gb,
                "creationData": creation
            }
        });
        self.request(Method::PUT, &url, Some(body)).await?;
        self.wait_resource(&url).await
    }

    async fn create_snapshot(&self, name: &str, disk_id: &str) -> Result<()> {
        let url = format!(
            "{}/resourceGroups/{}/providers/Microsoft.Compute/snapshots/{}?api-version=2023-03-01",
            self.api_base(),
            self.rg(),
            name
        );
        let body = json!({
            "location": self.config.location.clone(),
            "properties": {
                "creationData": {
                    "createOption": "Copy",
                    "sourceResourceId": disk_id
                }
            }
        });
        self.request(Method::PUT, &url, Some(body)).await?;
        self.wait_resource(&url).await
    }

    async fn create_vm(&self, params: CreateVmParams<'_>) -> Result<()> {
        let device_by_id = "/dev/disk/azure/scsi1/lun0".to_string();
        let bootstrap = BootstrapSpec {
            worker_id: params.worker_id,
            gateway_url: params.gateway_url,
            gateway_token: self.auth_token.as_deref(),
            base_commit: params.base_commit_sha,
            diff_debounce_ms: params.spec.diff_debounce_ms.unwrap_or(1500),
            repo: &params.spec.repo,
            provider_id: params.spec.provider_id.as_deref(),
            env: &params.spec.env,
            shim_url: &self.config.worker_shim_url,
            workdir: &self.config.workdir,
            mount_path: &self.config.mount_path,
            mount_device_candidates: vec![device_by_id],
        };
        let custom_data = render_bootstrap_script(&bootstrap);
        let custom_data_b64 = base64::engine::general_purpose::STANDARD.encode(custom_data);
        let url = format!(
            "{}/resourceGroups/{}/providers/Microsoft.Compute/virtualMachines/{}?api-version=2023-07-01",
            self.api_base(),
            self.rg(),
            params.vm_name
        );
        let body = json!({
            "location": self.config.location.clone(),
            "properties": {
                "hardwareProfile": { "vmSize": self.config.vm_size.clone() },
                "storageProfile": {
                    "imageReference": self.image_reference(),
                    "osDisk": {
                        "createOption": "FromImage"
                    },
                    "dataDisks": [{
                        "lun": 0,
                        "createOption": "Attach",
                        "managedDisk": {
                        "id": self.disk_id(params.disk_name)
                    }
                }]
            },
            "osProfile": {
                "computerName": params.vm_name,
                "adminUsername": self.config.admin_username.clone(),
                "customData": custom_data_b64,
                    "linuxConfiguration": {
                        "disablePasswordAuthentication": true,
                        "ssh": {
                            "publicKeys": [{
                                "path": format!(
                                    "/home/{}/.ssh/authorized_keys",
                                    self.config.admin_username.as_str()
                                ),
                                "keyData": self.config.ssh_public_key.clone()
                            }]
                        }
                    }
                },
                "networkProfile": {
                    "networkInterfaces": [{
                        "id": self.nic_id(params.nic_name),
                        "properties": { "primary": true }
                    }]
                }
            }
        });
        self.request(Method::PUT, &url, Some(body)).await?;
        self.wait_resource(&url).await
    }

    async fn fetch_public_ip(&self, public_ip_name: &str) -> Result<Option<String>> {
        let url = format!(
            "{}/resourceGroups/{}/providers/Microsoft.Network/publicIPAddresses/{}?api-version=2023-09-01",
            self.api_base(),
            self.rg(),
            public_ip_name
        );
        let resp = self.request(Method::GET, &url, None).await?;
        let ip = resp
            .get("properties")
            .and_then(|v| v.get("ipAddress"))
            .and_then(|v| v.as_str())
            .map(|v| v.to_string());
        Ok(ip)
    }
}

#[async_trait]
impl WorkerDriver for AzureDriver {
    async fn start(
        &self,
        worker_id: &str,
        spec: &StartWorkerRequest,
        base_commit_sha: &str,
        gateway_url: &str,
    ) -> Result<Option<SshInfo>> {
        if matches!(spec.repo, RepoSpec::Local { .. }) {
            anyhow::bail!("azure driver does not support local repo specs");
        }

        let vm_name = Self::sanitize_name("ctx-vm", worker_id);
        let disk_name = Self::sanitize_name("ctx-disk", worker_id);
        let nic_name = Self::sanitize_name("ctx-nic", worker_id);
        let public_ip_name = if self.config.use_public_ip {
            Some(Self::sanitize_name("ctx-ip", worker_id))
        } else {
            None
        };

        if let Some(name) = public_ip_name.as_ref() {
            self.create_public_ip(name).await?;
        }
        self.create_nic(&nic_name, public_ip_name.as_deref())
            .await?;
        self.create_disk(&disk_name, None).await?;
        self.create_vm(CreateVmParams {
            worker_id,
            vm_name: &vm_name,
            nic_name: &nic_name,
            disk_name: &disk_name,
            spec,
            base_commit_sha,
            gateway_url,
        })
        .await?;

        let public_ip = if let Some(name) = public_ip_name.as_ref() {
            self.fetch_public_ip(name).await.ok().flatten()
        } else {
            None
        };

        let ssh = public_ip.as_ref().map(|host| SshInfo {
            host: host.clone(),
            port: 22,
            user: self.config.admin_username.clone(),
            fingerprint: None,
        });

        let mut state = self.state.write().await;
        state.insert(
            worker_id.to_string(),
            AzureWorkerState {
                vm_name: Some(vm_name),
                disk_name: Some(disk_name),
                snapshot_name: None,
                nic_name: Some(nic_name),
                public_ip_name,
            },
        );

        Ok(ssh)
    }

    async fn stop(&self, worker_id: &str) -> Result<()> {
        let mut state = self.state.write().await;
        if let Some(entry) = state.get_mut(worker_id) {
            if let Some(vm_name) = entry.vm_name.take() {
                let url = format!(
                    "{}/resourceGroups/{}/providers/Microsoft.Compute/virtualMachines/{}?api-version=2023-07-01",
                    self.api_base(),
                    self.rg(),
                    vm_name
                );
                let _ = self.delete_resource(&url).await;
            }
            if let Some(nic_name) = entry.nic_name.take() {
                let url = format!(
                    "{}/resourceGroups/{}/providers/Microsoft.Network/networkInterfaces/{}?api-version=2023-09-01",
                    self.api_base(),
                    self.rg(),
                    nic_name
                );
                let _ = self.delete_resource(&url).await;
            }
            if let Some(public_ip) = entry.public_ip_name.take() {
                let url = format!(
                    "{}/resourceGroups/{}/providers/Microsoft.Network/publicIPAddresses/{}?api-version=2023-09-01",
                    self.api_base(),
                    self.rg(),
                    public_ip
                );
                let _ = self.delete_resource(&url).await;
            }
            if let Some(disk_name) = entry.disk_name.take() {
                let url = format!(
                    "{}/resourceGroups/{}/providers/Microsoft.Compute/disks/{}?api-version=2023-03-01",
                    self.api_base(),
                    self.rg(),
                    disk_name
                );
                let _ = self.delete_resource(&url).await;
            }
            if let Some(snapshot_name) = entry.snapshot_name.take() {
                let url = format!(
                    "{}/resourceGroups/{}/providers/Microsoft.Compute/snapshots/{}?api-version=2023-03-01",
                    self.api_base(),
                    self.rg(),
                    snapshot_name
                );
                let _ = self.delete_resource(&url).await;
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
        self.create_snapshot(&snapshot_name, &self.disk_id(&disk_name))
            .await?;
        entry.snapshot_name = Some(snapshot_name);

        if let Some(vm_name) = entry.vm_name.take() {
            let url = format!(
                "{}/resourceGroups/{}/providers/Microsoft.Compute/virtualMachines/{}?api-version=2023-07-01",
                self.api_base(),
                self.rg(),
                vm_name
            );
            let _ = self.delete_resource(&url).await;
        }
        if let Some(nic_name) = entry.nic_name.take() {
            let url = format!(
                "{}/resourceGroups/{}/providers/Microsoft.Network/networkInterfaces/{}?api-version=2023-09-01",
                self.api_base(),
                self.rg(),
                nic_name
            );
            let _ = self.delete_resource(&url).await;
        }
        if let Some(public_ip) = entry.public_ip_name.take() {
            let url = format!(
                "{}/resourceGroups/{}/providers/Microsoft.Network/publicIPAddresses/{}?api-version=2023-09-01",
                self.api_base(),
                self.rg(),
                public_ip
            );
            let _ = self.delete_resource(&url).await;
        }

        if self.config.delete_disk_on_pause {
            if let Some(disk_name) = entry.disk_name.take() {
                let url = format!(
                    "{}/resourceGroups/{}/providers/Microsoft.Compute/disks/{}?api-version=2023-03-01",
                    self.api_base(),
                    self.rg(),
                    disk_name
                );
                let _ = self.delete_resource(&url).await;
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
            anyhow::bail!("azure driver does not support local repo specs");
        }

        let vm_name = Self::sanitize_name("ctx-vm", worker_id);
        let disk_name = Self::sanitize_name("ctx-disk", worker_id);
        let nic_name = Self::sanitize_name("ctx-nic", worker_id);
        let public_ip_name = if self.config.use_public_ip {
            Some(Self::sanitize_name("ctx-ip", worker_id))
        } else {
            None
        };

        let disk_url = format!(
            "{}/resourceGroups/{}/providers/Microsoft.Compute/disks/{}?api-version=2023-03-01",
            self.api_base(),
            self.rg(),
            disk_name
        );
        let disk_exists = self.request(Method::GET, &disk_url, None).await.is_ok();
        if !disk_exists {
            let snapshot_name = Self::sanitize_name("ctx-snap", worker_id);
            self.create_disk(&disk_name, Some(self.snapshot_id(&snapshot_name)))
                .await?;
        }

        if let Some(name) = public_ip_name.as_ref() {
            self.create_public_ip(name).await?;
        }
        self.create_nic(&nic_name, public_ip_name.as_deref())
            .await?;
        self.create_vm(CreateVmParams {
            worker_id,
            vm_name: &vm_name,
            nic_name: &nic_name,
            disk_name: &disk_name,
            spec,
            base_commit_sha,
            gateway_url,
        })
        .await?;

        let public_ip = if let Some(name) = public_ip_name.as_ref() {
            self.fetch_public_ip(name).await.ok().flatten()
        } else {
            None
        };

        let ssh = public_ip.as_ref().map(|host| SshInfo {
            host: host.clone(),
            port: 22,
            user: self.config.admin_username.clone(),
            fingerprint: None,
        });

        let mut state = self.state.write().await;
        state.insert(
            worker_id.to_string(),
            AzureWorkerState {
                vm_name: Some(vm_name),
                disk_name: Some(disk_name),
                snapshot_name: Some(Self::sanitize_name("ctx-snap", worker_id)),
                nic_name: Some(nic_name),
                public_ip_name,
            },
        );

        Ok(ssh)
    }
}
