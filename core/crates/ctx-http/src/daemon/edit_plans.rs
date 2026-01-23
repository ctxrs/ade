use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::edit_plans::{EditPlan, EditPlanId};

pub(super) fn edit_plans_dir(data_root: &Path) -> PathBuf {
    data_root.join("edit_plans")
}

fn edit_plan_path(data_root: &Path, plan_id: EditPlanId) -> PathBuf {
    edit_plans_dir(data_root).join(format!("{}.json", plan_id.0))
}

pub(super) fn load_edit_plans_from_disk(data_root: &Path) -> HashMap<EditPlanId, EditPlan> {
    let mut out = HashMap::new();
    let dir = edit_plans_dir(data_root);
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(
                    "failed to read edit plan file {}: {e}",
                    path.to_string_lossy()
                );
                continue;
            }
        };
        match serde_json::from_slice::<EditPlan>(&bytes) {
            Ok(plan) => {
                out.insert(plan.id, plan);
            }
            Err(e) => {
                tracing::warn!(
                    "failed to parse edit plan file {}: {e}",
                    path.to_string_lossy()
                );
            }
        }
    }
    out
}

pub(super) fn persist_edit_plan_to_disk(data_root: &Path, plan: &EditPlan) -> anyhow::Result<()> {
    let path = edit_plan_path(data_root, plan.id);
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(plan)?;
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

pub(super) fn delete_edit_plan_file(data_root: &Path, plan_id: EditPlanId) -> anyhow::Result<()> {
    let path = edit_plan_path(data_root, plan_id);
    let _ = std::fs::remove_file(&path);
    Ok(())
}
