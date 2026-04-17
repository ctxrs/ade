use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use ctx_core::ids::{WorkspaceId, WorktreeId};
use ctx_core::models::{Workspace, Worktree, WorktreeBootstrapStatus};

const DEFAULT_TIMEOUT_SEC: u64 = 60;
const MAX_LOG_BYTES: usize = 200 * 1024;

#[derive(Debug, Clone)]
pub struct BootstrapConfig {
    pub timeout: Duration,
    pub command: String,
    pub wait_for_completion: bool,
}

#[derive(Debug, Clone)]
pub struct BootstrapStep {
    pub label: String,
    pub command: String,
}

#[derive(Debug)]
pub struct BootstrapCommandResult {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

#[derive(Debug, Clone)]
pub struct BootstrapReport {
    pub status: WorktreeBootstrapStatus,
    pub started_at: chrono::DateTime<Utc>,
    pub finished_at: chrono::DateTime<Utc>,
    pub exit_code: Option<i64>,
    pub timeout_sec: i64,
    pub error: Option<String>,
    pub command: Option<String>,
    pub raw_log: String,
}

#[async_trait]
pub trait WorktreeBootstrapHost: Send + Sync + 'static {
    async fn load_bootstrap_config(
        &self,
        workspace: &Workspace,
    ) -> Result<Option<BootstrapConfig>>;

    async fn execute_bootstrap_step(
        &self,
        workspace: &Workspace,
        worktree: &Worktree,
        step: &BootstrapStep,
        timeout: Duration,
    ) -> Result<BootstrapCommandResult>;

    async fn persist_bootstrap_report(
        &self,
        workspace_id: WorkspaceId,
        worktree: &Worktree,
        report: BootstrapReport,
    );

    async fn register_bootstrap(&self, worktree_id: WorktreeId, wait_for_completion: bool);

    async fn finish_bootstrap(&self, worktree_id: WorktreeId);
}

pub async fn run_worktree_bootstrap<H>(
    host: &H,
    workspace: &Workspace,
    worktree: &Worktree,
) -> Result<()>
where
    H: WorktreeBootstrapHost,
{
    let Some(plan) = prepare_worktree_bootstrap(host, workspace, worktree).await? else {
        return Ok(());
    };
    run_worktree_bootstrap_plan(host, workspace, worktree, plan).await
}

pub async fn spawn_worktree_bootstrap<H>(
    host: std::sync::Arc<H>,
    workspace: Workspace,
    worktree: Worktree,
) -> Result<()>
where
    H: WorktreeBootstrapHost,
{
    let Some(plan) = prepare_worktree_bootstrap(host.as_ref(), &workspace, &worktree).await? else {
        return Ok(());
    };

    let worktree_id = worktree.id;
    host.register_bootstrap(worktree_id, plan.wait_for_completion)
        .await;

    tokio::spawn(async move {
        if let Err(e) =
            run_worktree_bootstrap_plan(host.as_ref(), &workspace, &worktree, plan).await
        {
            tracing::warn!(worktree_id = %worktree_id.0, "worktree bootstrap failed: {e:?}");
        }
        host.finish_bootstrap(worktree_id).await;
    });

    Ok(())
}

#[derive(Debug, Clone)]
struct WorktreeBootstrapPlan {
    timeout: Duration,
    steps: Vec<BootstrapStep>,
    wait_for_completion: bool,
}

async fn prepare_worktree_bootstrap<H>(
    host: &H,
    workspace: &Workspace,
    worktree: &Worktree,
) -> Result<Option<WorktreeBootstrapPlan>>
where
    H: WorktreeBootstrapHost,
{
    let has_vcs_ref = worktree.vcs_ref.is_some() || worktree.git_branch.is_some();
    if !has_vcs_ref {
        return Ok(None);
    }

    let bootstrap = match host.load_bootstrap_config(workspace).await {
        Ok(Some(cfg)) => cfg,
        Ok(None) => return Ok(None),
        Err(err) => {
            let started_at = Utc::now();
            let finished_at = Utc::now();
            let error = err.to_string();
            let error_details = format!("{err:#}");
            let mut log = String::new();
            log.push_str("# ctx worktree bootstrap\n");
            log.push_str(&format!("# Worktree: {}\n", worktree.root_path.trim()));
            log.push_str(&format!("# Started: {}\n\n", started_at.to_rfc3339()));
            log.push_str("[error]\n");
            log.push_str(&error_details);
            log.push('\n');
            log.push_str(&format!("\n# Finished: {}\n", finished_at.to_rfc3339()));
            log.push_str("# Status: Failed\n");
            host.persist_bootstrap_report(
                workspace.id,
                worktree,
                BootstrapReport {
                    status: WorktreeBootstrapStatus::Failed,
                    started_at,
                    finished_at,
                    exit_code: None,
                    timeout_sec: DEFAULT_TIMEOUT_SEC as i64,
                    error: Some(error),
                    command: None,
                    raw_log: log,
                },
            )
            .await;
            return Ok(None);
        }
    };

    let steps = build_bootstrap_steps(&bootstrap.command)?;
    if steps.is_empty() {
        return Ok(None);
    }

    Ok(Some(WorktreeBootstrapPlan {
        timeout: bootstrap.timeout,
        steps,
        wait_for_completion: bootstrap.wait_for_completion,
    }))
}

async fn run_worktree_bootstrap_plan<H>(
    host: &H,
    workspace: &Workspace,
    worktree: &Worktree,
    plan: WorktreeBootstrapPlan,
) -> Result<()>
where
    H: WorktreeBootstrapHost,
{
    let WorktreeBootstrapPlan { timeout, steps, .. } = plan;
    let started_at = Utc::now();
    let mut log = String::new();
    log.push_str("# ctx worktree bootstrap\n");
    log.push_str(&format!("# Worktree: {}\n", worktree.root_path.trim()));
    log.push_str(&format!("# Started: {}\n\n", started_at.to_rfc3339()));

    let mut last_exit = None;
    let mut last_step: Option<BootstrapStep> = None;
    let mut failure_status = None;
    let mut failure_error = None;

    for step in &steps {
        last_step = Some(step.clone());
        log.push_str(&format!("$ {}\n", step.label));

        let result = match host
            .execute_bootstrap_step(workspace, worktree, step, timeout)
            .await
        {
            Ok(result) => result,
            Err(err) => {
                failure_status = Some(WorktreeBootstrapStatus::Failed);
                failure_error = Some(err.to_string());
                break;
            }
        };
        last_exit = result.exit_code.map(|v| v as i64);

        append_output(&mut log, &result.stdout, "stdout");
        append_output(&mut log, &result.stderr, "stderr");

        if result.timed_out {
            failure_status = Some(WorktreeBootstrapStatus::Timeout);
            failure_error = Some(format!("bootstrap timed out after {}s", timeout.as_secs()));
            break;
        }

        if let Some(code) = result.exit_code {
            if code != 0 {
                failure_status = Some(WorktreeBootstrapStatus::Failed);
                failure_error = Some(format!("command exited with code {code}"));
                break;
            }
        }
    }

    let finished_at = Utc::now();
    log.push_str(&format!("\n# Finished: {}\n", finished_at.to_rfc3339()));
    if let Some(status) = &failure_status {
        log.push_str(&format!("# Status: {status:?}\n"));
    } else {
        log.push_str("# Status: success\n");
    }

    let (status, error) = match failure_status {
        Some(status) => (status, failure_error),
        None => (WorktreeBootstrapStatus::Success, None),
    };

    host.persist_bootstrap_report(
        workspace.id,
        worktree,
        BootstrapReport {
            status,
            started_at,
            finished_at,
            exit_code: last_exit,
            timeout_sec: timeout.as_secs() as i64,
            error,
            command: last_step.as_ref().map(|step| step.command.clone()),
            raw_log: log,
        },
    )
    .await;

    Ok(())
}

fn build_bootstrap_steps(command: &str) -> Result<Vec<BootstrapStep>> {
    let mut steps = Vec::new();
    let trimmed = command.trim();
    if !trimmed.is_empty() {
        steps.push(BootstrapStep {
            label: trimmed.to_string(),
            command: trimmed.to_string(),
        });
    }
    Ok(steps)
}

fn append_output(log: &mut String, output: &str, label: &str) {
    let trimmed = output.trim_end_matches('\n');
    if trimmed.is_empty() {
        return;
    }
    log.push_str(&format!("[{label}]\n"));
    log.push_str(trimmed);
    log.push('\n');
}

pub fn truncate_log(input: &str) -> (String, bool) {
    if input.len() <= MAX_LOG_BYTES {
        return (input.to_string(), false);
    }
    let mut out = input.chars().take(MAX_LOG_BYTES).collect::<String>();
    out.push_str("\n...(truncated)\n");
    (out, true)
}

#[cfg(test)]
mod tests {
    use super::{build_bootstrap_steps, truncate_log};

    #[test]
    fn build_bootstrap_steps_ignores_blank_commands() {
        assert!(build_bootstrap_steps("   ").expect("steps").is_empty());
        assert_eq!(
            build_bootstrap_steps("echo hi").expect("steps")[0].command,
            "echo hi"
        );
    }

    #[test]
    fn truncate_log_marks_truncated_output() {
        let input = "x".repeat(250_000);
        let (out, truncated) = truncate_log(&input);
        assert!(truncated);
        assert!(out.contains("...(truncated)"));
    }
}
