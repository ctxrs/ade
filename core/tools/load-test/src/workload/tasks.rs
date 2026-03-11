use anyhow::{anyhow, Context, Result};
use rand::rngs::StdRng;
use rand::Rng;

use crate::scenario::{Cli, ScenarioSpec};
use ctx_client::{Client, CreateSessionRequest, CreateTaskRequest};
use ctx_core::ids::{SessionId, TaskId, WorkspaceId};

pub(crate) async fn resolve_workspace(client: &Client, id: Option<&str>) -> Result<WorkspaceId> {
    if let Some(id) = id {
        let parsed = uuid::Uuid::parse_str(id).context("invalid workspace id")?;
        return Ok(WorkspaceId(parsed));
    }
    let workspaces = client.list_workspaces().await?;
    let first = workspaces
        .first()
        .ok_or_else(|| anyhow!("no workspaces found; create one first"))?;
    Ok(first.id)
}

pub(crate) async fn setup_tasks_and_sessions(
    client: &Client,
    workspace_id: WorkspaceId,
    scenario: &ScenarioSpec,
    rng: &mut StdRng,
    cli: &Cli,
) -> Result<(Vec<TaskId>, Vec<SessionId>)> {
    let mut tasks = Vec::new();
    let mut sessions = Vec::new();
    for idx in 0..scenario.workload.tasks {
        let title = format!("Load test {} task {}", scenario.name, idx + 1);
        let task = client
            .create_task(
                workspace_id,
                &CreateTaskRequest {
                    id: None,
                    title,
                    description: None,
                    create_default_session: Some(false),
                },
            )
            .await
            .with_context(|| "creating task")?;
        tasks.push(task.id);

        let subagents =
            rng.gen_range(scenario.workload.subagents_min..=scenario.workload.subagents_max);
        for _ in 0..subagents {
            let session = client
                .create_session(
                    task.id,
                    &CreateSessionRequest {
                        id: None,
                        provider_id: cli.provider_id.clone(),
                        model_id: cli.model_id.clone(),
                        parent_session_id: None,
                        relationship: None,
                        execution_environment: None,
                        worktree_id: None,
                        initial_prompt: None,
                        initial_message_id: None,
                        initial_turn_id: None,
                    },
                )
                .await
                .with_context(|| "creating session (ensure CTX_SHOW_FAKE_PROVIDER=1)")?;
            sessions.push(session.id);
        }
    }
    Ok((tasks, sessions))
}
