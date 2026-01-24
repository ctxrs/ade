use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn env_flag(name: &str) -> Option<bool> {
    let raw = std::env::var(name).ok()?;
    let value = raw.trim().to_ascii_lowercase();
    Some(matches!(value.as_str(), "1" | "true" | "yes" | "on"))
}

fn merge_malloc_conf(existing: Option<&str>, extra: &str) -> String {
    match existing {
        Some(current) if !current.trim().is_empty() => format!("{current},{extra}"),
        _ => extra.to_string(),
    }
}

pub fn apply_acp_heap_profile_env(
    provider_id: &str,
    env: &mut HashMap<String, String>,
    data_root: &Path,
) {
    if provider_id != "codex" {
        return;
    }

    let enabled = env_flag("CTX_ACP_HEAP_PROFILE").unwrap_or(false);
    let dir = std::env::var("CTX_ACP_HEAP_PROFILE_DIR").ok().or_else(|| {
        if enabled {
            Some(
                data_root
                    .join("logs")
                    .join("providers")
                    .join("heap")
                    .to_string_lossy()
                    .to_string(),
            )
        } else {
            None
        }
    });

    let dir = match dir {
        Some(dir) => dir,
        None => return,
    };

    let dir_path = PathBuf::from(&dir);
    if let Err(err) = std::fs::create_dir_all(&dir_path) {
        tracing::warn!("failed to create ACP heap profile dir {dir}: {err:#}");
        return;
    }

    let prefix = dir_path.join(format!("{provider_id}-heap"));
    let mut conf_parts = vec![
        "prof:true".to_string(),
        "prof_active:true".to_string(),
        "prof_final:true".to_string(),
        format!("prof_prefix={}", prefix.to_string_lossy()),
    ];

    if let Ok(value) = std::env::var("CTX_ACP_HEAP_PROFILE_LG_INTERVAL") {
        conf_parts.push(format!("lg_prof_interval={value}"));
    }
    if let Ok(value) = std::env::var("CTX_ACP_HEAP_PROFILE_LG_SAMPLE") {
        conf_parts.push(format!("lg_prof_sample={value}"));
    }

    let extra = conf_parts.join(",");
    let merged = merge_malloc_conf(env.get("MALLOC_CONF").map(String::as_str), &extra);
    env.insert("MALLOC_CONF".to_string(), merged);
    env.insert("CTX_ACP_HEAP_PROFILE_DIR".to_string(), dir);
}
