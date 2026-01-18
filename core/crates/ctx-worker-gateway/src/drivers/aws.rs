use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context, Result};
use async_trait::async_trait;
use aws_config::Region;
use aws_sdk_ec2::types::{
    IamInstanceProfileSpecification, InstanceType, ResourceType, Tag, TagSpecification, VolumeType,
};
use aws_sdk_ec2::Client;
use base64::Engine;
use tokio::sync::RwLock;

use ctx_worker_protocol::{RepoSpec, SshInfo, StartWorkerRequest};

use crate::bootstrap::render_fetch_bootstrap_script;

use super::WorkerDriver;

pub struct AwsConfig {
    pub region: String,
    pub ami_id: String,
    pub instance_type: String,
    pub subnet_id: String,
    pub security_group_ids: Vec<String>,
    pub key_name: Option<String>,
    pub instance_profile: Option<String>,
    pub ssh_user: Option<String>,
    pub volume_size_gb: i32,
    pub volume_type: String,
    pub data_device_name: String,
    pub delete_volume_on_pause: bool,
    pub wait_for_snapshot: bool,
    pub availability_zone: Option<String>,
}

pub struct AwsDriver {
    client: Client,
    config: AwsConfig,
    auth_token: Option<String>,
    state: RwLock<HashMap<String, AwsWorkerState>>,
}

#[derive(Clone, Default)]
struct AwsWorkerState {
    instance_id: Option<String>,
    volume_id: Option<String>,
    snapshot_id: Option<String>,
    availability_zone: Option<String>,
    public_ip: Option<String>,
    private_ip: Option<String>,
}

impl AwsDriver {
    pub async fn new(config: AwsConfig, auth_token: Option<String>) -> Result<Self> {
        let sdk_config = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .region(Region::new(config.region.clone()))
            .load()
            .await;
        Ok(Self {
            client: Client::new(&sdk_config),
            config,
            auth_token,
            state: RwLock::new(HashMap::new()),
        })
    }

    async fn resolve_availability_zone(&self) -> Result<String> {
        if let Some(zone) = self.config.availability_zone.as_ref() {
            return Ok(zone.clone());
        }
        let resp = self
            .client
            .describe_subnets()
            .subnet_ids(self.config.subnet_id.clone())
            .send()
            .await
            .context("describe_subnets")?;
        let subnet = resp.subnets().first().context("missing subnet")?;
        subnet
            .availability_zone()
            .map(|v| v.to_string())
            .context("missing subnet availability zone")
    }

    fn volume_type(&self) -> VolumeType {
        match self.config.volume_type.to_ascii_lowercase().as_str() {
            "gp2" => VolumeType::Gp2,
            "io1" => VolumeType::Io1,
            "io2" => VolumeType::Io2,
            "st1" => VolumeType::St1,
            "sc1" => VolumeType::Sc1,
            "standard" => VolumeType::Standard,
            _ => VolumeType::Gp3,
        }
    }

    fn instance_type(&self) -> InstanceType {
        InstanceType::from(self.config.instance_type.as_str())
    }

    fn tag_specifications(
        &self,
        resource_type: ResourceType,
        worker_id: &str,
        spec: &StartWorkerRequest,
    ) -> TagSpecification {
        let tags = vec![
            Tag::builder().key("ctx-worker-id").value(worker_id).build(),
            Tag::builder()
                .key("ctx-task-id")
                .value(&spec.task_id)
                .build(),
        ];
        TagSpecification::builder()
            .resource_type(resource_type)
            .set_tags(Some(tags))
            .build()
    }

    async fn create_volume(
        &self,
        zone: &str,
        worker_id: &str,
        spec: &StartWorkerRequest,
    ) -> Result<String> {
        let resp = self
            .client
            .create_volume()
            .availability_zone(zone)
            .size(self.config.volume_size_gb)
            .volume_type(self.volume_type())
            .tag_specifications(self.tag_specifications(ResourceType::Volume, worker_id, spec))
            .send()
            .await
            .context("create_volume")?;
        resp.volume_id()
            .map(|v| v.to_string())
            .context("missing volume id")
    }

    async fn run_instance(
        &self,
        worker_id: &str,
        spec: &StartWorkerRequest,
        _base_commit_sha: &str,
        gateway_url: &str,
        volume_id: &str,
    ) -> Result<String> {
        let volume_id_normalized = volume_id.replace('-', "");
        let device_by_id = format!(
            "/dev/disk/by-id/nvme-Amazon_Elastic_Block_Store_{}",
            volume_id_normalized
        );
        let mut candidates = vec![
            device_by_id,
            "/dev/xvdf".to_string(),
            "/dev/sdf".to_string(),
            "/dev/nvme1n1".to_string(),
        ];
        candidates.retain(|value| !value.is_empty());

        // Use a tiny user-data script that fetches the full bootstrap from the gateway
        // to stay well under the 16KB EC2 user-data limit.
        let ca_b64 = spec.env.get("CTX_GATEWAY_CA_B64").map(|v| v.as_str());
        let fetch_script = render_fetch_bootstrap_script(
            worker_id,
            gateway_url,
            self.auth_token.as_deref(),
            ca_b64,
        );
        let user_data_b64 = base64::engine::general_purpose::STANDARD.encode(fetch_script);

        let mut run = self
            .client
            .run_instances()
            .image_id(&self.config.ami_id)
            .instance_type(self.instance_type())
            .min_count(1)
            .max_count(1)
            .subnet_id(&self.config.subnet_id)
            .user_data(user_data_b64)
            .tag_specifications(self.tag_specifications(ResourceType::Instance, worker_id, spec));

        if !self.config.security_group_ids.is_empty() {
            run = run.set_security_group_ids(Some(self.config.security_group_ids.clone()));
        }
        if let Some(key_name) = &self.config.key_name {
            run = run.key_name(key_name);
        }
        if let Some(profile) = &self.config.instance_profile {
            let profile = IamInstanceProfileSpecification::builder()
                .name(profile)
                .build();
            run = run.iam_instance_profile(profile);
        }

        let resp = run.send().await.context("run_instances")?;
        let instance = resp.instances().first().context("missing instance")?;
        let instance_id = instance
            .instance_id()
            .map(|id| id.to_string())
            .context("missing instance id")?;

        self.wait_instance_running(&instance_id).await?;
        self.client
            .attach_volume()
            .instance_id(&instance_id)
            .volume_id(volume_id)
            .device(&self.config.data_device_name)
            .send()
            .await
            .context("attach_volume")?;

        Ok(instance_id)
    }

    async fn wait_instance_running(&self, instance_id: &str) -> Result<()> {
        for _ in 0..40 {
            let resp = match self
                .client
                .describe_instances()
                .instance_ids(instance_id)
                .send()
                .await
            {
                Ok(resp) => resp,
                Err(err) => {
                    let msg = err.to_string();
                    let dbg = format!("{err:?}");
                    if msg.contains("InvalidInstanceID.NotFound")
                        || dbg.contains("InvalidInstanceID.NotFound")
                    {
                        tokio::time::sleep(Duration::from_secs(3)).await;
                        continue;
                    }
                    return Err(anyhow::Error::new(err).context("describe_instances"));
                }
            };
            if let Some(res) = resp.reservations().first() {
                if let Some(instance) = res.instances().first() {
                    let state = instance.state().and_then(|s| s.name()).map(|s| s.as_str());
                    if matches!(state, Some("running")) {
                        return Ok(());
                    }
                    if matches!(state, Some("terminated") | Some("shutting-down")) {
                        anyhow::bail!("instance terminated while waiting to run");
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
        anyhow::bail!("instance wait timeout")
    }

    async fn wait_instance_ips(
        &self,
        instance_id: &str,
    ) -> Result<(Option<String>, Option<String>)> {
        for _ in 0..40 {
            let resp = match self
                .client
                .describe_instances()
                .instance_ids(instance_id)
                .send()
                .await
            {
                Ok(resp) => resp,
                Err(err) => {
                    let msg = err.to_string();
                    let dbg = format!("{err:?}");
                    if msg.contains("InvalidInstanceID.NotFound")
                        || dbg.contains("InvalidInstanceID.NotFound")
                    {
                        tokio::time::sleep(Duration::from_secs(3)).await;
                        continue;
                    }
                    return Err(anyhow::Error::new(err).context("describe_instances"));
                }
            };
            if let Some(res) = resp.reservations().first() {
                if let Some(instance) = res.instances().first() {
                    let state = instance.state().and_then(|s| s.name()).map(|s| s.as_str());
                    if matches!(state, Some("running")) {
                        let public_ip = instance.public_ip_address().map(|v| v.to_string());
                        let private_ip = instance.private_ip_address().map(|v| v.to_string());
                        return Ok((public_ip, private_ip));
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
        Ok((None, None))
    }

    async fn volume_exists(&self, volume_id: &str) -> Result<bool> {
        let resp = self
            .client
            .describe_volumes()
            .volume_ids(volume_id)
            .send()
            .await
            .context("describe_volumes")?;
        Ok(!resp.volumes().is_empty())
    }

    async fn wait_snapshot(&self, snapshot_id: &str) -> Result<()> {
        for _ in 0..60 {
            let resp = self
                .client
                .describe_snapshots()
                .snapshot_ids(snapshot_id)
                .send()
                .await
                .context("describe_snapshots")?;
            if let Some(snapshot) = resp.snapshots().first() {
                if let Some(state) = snapshot.state() {
                    if state.as_str() == "completed" {
                        return Ok(());
                    }
                    if state.as_str() == "error" {
                        anyhow::bail!("snapshot failed");
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
        anyhow::bail!("snapshot wait timeout")
    }

    fn build_ssh_info(
        &self,
        public_ip: Option<String>,
        private_ip: Option<String>,
    ) -> Option<SshInfo> {
        let user = self.config.ssh_user.as_ref()?.clone();
        let host = public_ip.or(private_ip)?;
        Some(SshInfo {
            host,
            port: 22,
            user,
            fingerprint: None,
        })
    }
}

#[async_trait]
impl WorkerDriver for AwsDriver {
    async fn start(
        &self,
        worker_id: &str,
        spec: &StartWorkerRequest,
        base_commit_sha: &str,
        gateway_url: &str,
    ) -> Result<Option<SshInfo>> {
        if matches!(spec.repo, RepoSpec::Local { .. }) {
            anyhow::bail!("aws driver does not support local repo specs");
        }

        let zone = self.resolve_availability_zone().await?;
        let volume_id = self.create_volume(&zone, worker_id, spec).await?;
        {
            let mut state = self.state.write().await;
            state.insert(
                worker_id.to_string(),
                AwsWorkerState {
                    instance_id: None,
                    volume_id: Some(volume_id.clone()),
                    snapshot_id: None,
                    availability_zone: Some(zone.clone()),
                    public_ip: None,
                    private_ip: None,
                },
            );
        }
        let instance_id = self
            .run_instance(worker_id, spec, base_commit_sha, gateway_url, &volume_id)
            .await?;

        let (public_ip, private_ip) = self.wait_instance_ips(&instance_id).await?;
        let ssh = self.build_ssh_info(public_ip.clone(), private_ip.clone());

        let mut state = self.state.write().await;
        if let Some(entry) = state.get_mut(worker_id) {
            entry.instance_id = Some(instance_id);
            entry.public_ip = public_ip;
            entry.private_ip = private_ip;
        }

        Ok(ssh)
    }

    async fn stop(&self, worker_id: &str) -> Result<()> {
        let mut state = self.state.write().await;
        if let Some(entry) = state.get_mut(worker_id) {
            if let Some(instance_id) = entry.instance_id.take() {
                let _ = self
                    .client
                    .terminate_instances()
                    .instance_ids(instance_id)
                    .send()
                    .await;
            }
            if let Some(volume_id) = entry.volume_id.take() {
                let _ = self
                    .client
                    .delete_volume()
                    .volume_id(volume_id)
                    .send()
                    .await;
            }
            if let Some(snapshot_id) = entry.snapshot_id.take() {
                let _ = self
                    .client
                    .delete_snapshot()
                    .snapshot_id(snapshot_id)
                    .send()
                    .await;
            }
        }
        Ok(())
    }

    async fn pause(&self, worker_id: &str) -> Result<()> {
        let mut state = self.state.write().await;
        let entry = state.get_mut(worker_id).context("missing worker state")?;
        let Some(volume_id) = entry.volume_id.clone() else {
            anyhow::bail!("missing volume id");
        };

        let snapshot_resp = self
            .client
            .create_snapshot()
            .volume_id(&volume_id)
            .description(format!("ctx snapshot for worker {}", worker_id))
            .send()
            .await
            .context("create_snapshot")?;
        let snapshot_id = snapshot_resp
            .snapshot_id()
            .map(|v| v.to_string())
            .context("missing snapshot id")?;
        entry.snapshot_id = Some(snapshot_id.clone());

        if self.config.wait_for_snapshot {
            let _ = self.wait_snapshot(&snapshot_id).await;
        }

        if let Some(instance_id) = entry.instance_id.take() {
            let _ = self
                .client
                .terminate_instances()
                .instance_ids(instance_id)
                .send()
                .await;
        }

        if self.config.delete_volume_on_pause {
            if let Some(volume_id) = entry.volume_id.take() {
                let _ = self
                    .client
                    .delete_volume()
                    .volume_id(volume_id)
                    .send()
                    .await;
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
            anyhow::bail!("aws driver does not support local repo specs");
        }

        let zone = self.resolve_availability_zone().await?;
        let mut state = self.state.write().await;
        let entry = state.entry(worker_id.to_string()).or_default();

        let volume_id = if let Some(volume_id) = entry.volume_id.clone() {
            if self.volume_exists(&volume_id).await.unwrap_or(false) {
                volume_id
            } else {
                entry.volume_id = None;
                self.create_volume_from_snapshot(worker_id, spec, &zone, entry)
                    .await?
            }
        } else {
            self.create_volume_from_snapshot(worker_id, spec, &zone, entry)
                .await?
        };

        let instance_id = self
            .run_instance(worker_id, spec, base_commit_sha, gateway_url, &volume_id)
            .await?;
        let (public_ip, private_ip) = self.wait_instance_ips(&instance_id).await?;
        entry.instance_id = Some(instance_id);
        entry.volume_id = Some(volume_id);
        entry.availability_zone = Some(zone);
        entry.public_ip = public_ip.clone();
        entry.private_ip = private_ip.clone();

        Ok(self.build_ssh_info(public_ip, private_ip))
    }
}

impl AwsDriver {
    async fn create_volume_from_snapshot(
        &self,
        worker_id: &str,
        spec: &StartWorkerRequest,
        zone: &str,
        entry: &mut AwsWorkerState,
    ) -> Result<String> {
        let snapshot_id = entry
            .snapshot_id
            .clone()
            .context("missing snapshot id for resume")?;
        let resp = self
            .client
            .create_volume()
            .availability_zone(zone)
            .snapshot_id(snapshot_id)
            .volume_type(self.volume_type())
            .tag_specifications(self.tag_specifications(ResourceType::Volume, worker_id, spec))
            .send()
            .await
            .context("create_volume from snapshot")?;
        resp.volume_id()
            .map(|v| v.to_string())
            .context("missing volume id")
    }
}
