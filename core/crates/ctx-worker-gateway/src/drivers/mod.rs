use async_trait::async_trait;
use ctx_worker_protocol::{SshInfo, StartWorkerRequest};

pub mod aws;
pub mod azure;
pub mod gcp;
pub mod local;

#[async_trait]
pub trait WorkerDriver: Send + Sync {
    async fn start(
        &self,
        worker_id: &str,
        spec: &StartWorkerRequest,
        base_commit_sha: &str,
        gateway_url: &str,
    ) -> anyhow::Result<Option<SshInfo>>;
    async fn stop(&self, worker_id: &str) -> anyhow::Result<()>;
    async fn pause(&self, worker_id: &str) -> anyhow::Result<()>;
    async fn resume(
        &self,
        worker_id: &str,
        spec: &StartWorkerRequest,
        base_commit_sha: &str,
        gateway_url: &str,
    ) -> anyhow::Result<Option<SshInfo>>;
}
