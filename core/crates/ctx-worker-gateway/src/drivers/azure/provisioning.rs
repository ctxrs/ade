use super::*;

pub(super) struct CreateVmParams<'a> {
    pub(super) worker_id: &'a str,
    pub(super) vm_name: &'a str,
    pub(super) nic_name: &'a str,
    pub(super) disk_name: &'a str,
    pub(super) spec: &'a StartWorkerRequest,
    pub(super) base_commit_sha: &'a str,
    pub(super) gateway_url: &'a str,
}

impl AzureDriver {
    pub(super) async fn create_public_ip(&self, name: &str) -> Result<()> {
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

    pub(super) async fn create_nic(&self, name: &str, public_ip_name: Option<&str>) -> Result<()> {
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
            let Some(props) = ip_config
                .get_mut("properties")
                .and_then(|v| v.as_object_mut())
            else {
                return Err(anyhow::anyhow!("invalid azure ip config shape"));
            };
            props.insert(
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

    pub(super) async fn create_disk(&self, name: &str, snapshot_id: Option<String>) -> Result<()> {
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

    pub(super) async fn create_snapshot(&self, name: &str, disk_id: &str) -> Result<()> {
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

    pub(super) async fn create_vm(&self, params: CreateVmParams<'_>) -> Result<()> {
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

    pub(super) async fn fetch_public_ip(&self, public_ip_name: &str) -> Result<Option<String>> {
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
