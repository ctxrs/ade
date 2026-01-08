use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::Utc;
use serde::Serialize;
use sysinfo::{Disk, Disks, Pid, System};

use ctx_core::ids::WorkspaceId;
use ctx_core::models::{Workspace, Worktree};
use ctx_providers::adapters::ProviderProcessInfo;

const SYSTEM_CACHE_TTL: Duration = Duration::from_millis(750);
const DISK_CACHE_TTL: Duration = Duration::from_secs(30);
const MAX_CHILD_PROCESSES: usize = 2000;

#[derive(Debug, Clone, Serialize)]
pub struct SystemSnapshot {
    pub cpu_pct: f32,
    pub memory_total_bytes: u64,
    pub memory_used_bytes: u64,
    pub swap_total_bytes: u64,
    pub swap_used_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiskSnapshot {
    pub name: String,
    pub mount_point: String,
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub file_system: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResourceChildProcess {
    pub pid: u32,
    pub parent_pid: Option<u32>,
    pub name: String,
    pub cmdline: Option<String>,
    pub cpu_pct: f32,
    pub memory_bytes: u64,
    pub virtual_memory_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResourceProcess {
    pub label: String,
    pub pid: u32,
    pub cpu_pct: f32,
    pub memory_bytes: u64,
    pub virtual_memory_bytes: u64,
    pub child_count: u64,
    pub children: Vec<ResourceChildProcess>,
    pub children_truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResourceProcesses {
    pub daemon: Option<ResourceProcess>,
    pub providers: Vec<ResourceProcess>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorktreeDiskSnapshot {
    pub worktree_id: String,
    pub root_path: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceDiskSnapshot {
    pub workspace_id: String,
    pub root_path: String,
    pub size_bytes: u64,
    pub size_collected_at: String,
    pub size_cache_age_ms: u64,
    pub disk: Option<DiskSnapshot>,
    pub worktrees: Vec<WorktreeDiskSnapshot>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResourceUtilizationSnapshot {
    pub collected_at: String,
    pub cache_age_ms: u64,
    pub system: SystemSnapshot,
    pub processes: ResourceProcesses,
    pub workspace: WorkspaceDiskSnapshot,
}

#[derive(Debug, Clone)]
pub struct WorkspaceDiskCache {
    pub collected_at: Instant,
    pub snapshot: WorkspaceDiskSnapshot,
}

pub struct ResourceSampler {
    system: System,
    disks: Disks,
    last_refresh: Option<Instant>,
    disk_cache: HashMap<WorkspaceId, WorkspaceDiskCache>,
}

impl ResourceSampler {
    pub fn new() -> Self {
        let mut system = System::new_all();
        system.refresh_all();
        let mut disks = Disks::new_with_refreshed_list();
        disks.refresh();
        Self {
            system,
            disks,
            last_refresh: None,
            disk_cache: HashMap::new(),
        }
    }

    pub fn system_snapshot(&mut self) -> (SystemSnapshot, Vec<DiskSnapshot>, u64) {
        let now = Instant::now();
        let should_refresh = self
            .last_refresh
            .map(|t| now.duration_since(t) > SYSTEM_CACHE_TTL)
            .unwrap_or(true);
        if should_refresh {
            self.system.refresh_cpu();
            self.system.refresh_memory();
            self.system.refresh_processes();
            if self.disks.list().is_empty() {
                self.disks.refresh_list();
            }
            self.disks.refresh();
            self.last_refresh = Some(now);
        }
        let cache_age_ms = self
            .last_refresh
            .map(|t| now.duration_since(t).as_millis() as u64)
            .unwrap_or(0);
        let system = SystemSnapshot {
            cpu_pct: self.system.global_cpu_info().cpu_usage(),
            // sysinfo reports memory values in bytes.
            memory_total_bytes: self.system.total_memory(),
            memory_used_bytes: self.system.used_memory(),
            swap_total_bytes: self.system.total_swap(),
            swap_used_bytes: self.system.used_swap(),
        };
        let disks = self.disks.iter().map(DiskSnapshot::from).collect();
        (system, disks, cache_age_ms)
    }

    pub fn processes_snapshot(
        &self,
        daemon_pid: u32,
        providers: &[ProviderProcessInfo],
    ) -> ResourceProcesses {
        let mut task_pids = HashSet::new();
        for (pid, process) in self.system.processes() {
            if let Some(tasks) = process.tasks() {
                for task_pid in tasks {
                    if task_pid != pid {
                        task_pids.insert(*task_pid);
                    }
                }
            }
        }
        let mut children: HashMap<Pid, Vec<Pid>> = HashMap::new();
        for (pid, process) in self.system.processes() {
            if task_pids.contains(pid) {
                continue;
            }
            if let Some(parent) = process.parent() {
                if task_pids.contains(&parent) {
                    continue;
                }
                children.entry(parent).or_default().push(*pid);
            }
        }

        let daemon = aggregate_process(&self.system, &children, daemon_pid, "ctx daemon");
        let providers = providers
            .iter()
            .filter_map(|p| {
                let label = p.label.clone().unwrap_or_else(|| p.provider_id.clone());
                aggregate_process(&self.system, &children, p.pid, &label)
            })
            .collect();

        ResourceProcesses { daemon, providers }
    }

    pub fn disk_cache_entry(&self, workspace_id: WorkspaceId) -> Option<WorkspaceDiskCache> {
        self.disk_cache.get(&workspace_id).cloned()
    }

    pub fn update_disk_cache(
        &mut self,
        workspace_id: WorkspaceId,
        collected_at: Instant,
        snapshot: WorkspaceDiskSnapshot,
    ) {
        self.disk_cache.insert(
            workspace_id,
            WorkspaceDiskCache {
                collected_at,
                snapshot,
            },
        );
    }
}

impl Default for ResourceSampler {
    fn default() -> Self {
        Self::new()
    }
}

pub fn disk_for_path(path: &Path, disks: &[DiskSnapshot]) -> Option<DiskSnapshot> {
    let mut best: Option<DiskSnapshot> = None;
    let mut best_len = 0usize;
    for disk in disks {
        let mount = Path::new(&disk.mount_point);
        if path.starts_with(mount) {
            let len = disk.mount_point.len();
            if len >= best_len {
                best_len = len;
                best = Some(disk.clone());
            }
        }
    }
    best
}

pub fn should_refresh_disk_cache(now: Instant, cache: Option<&WorkspaceDiskCache>) -> bool {
    match cache {
        Some(entry) => now.duration_since(entry.collected_at) > DISK_CACHE_TTL,
        None => true,
    }
}

pub fn disk_cache_age_ms(now: Instant, cache: Option<&WorkspaceDiskCache>) -> u64 {
    cache
        .map(|entry| now.duration_since(entry.collected_at).as_millis() as u64)
        .unwrap_or(0)
}

pub fn compute_workspace_disk_snapshot(
    workspace: Workspace,
    worktrees: Vec<Worktree>,
    disk: Option<DiskSnapshot>,
    size_cache_age_ms: u64,
) -> WorkspaceDiskSnapshot {
    let workspace_root = PathBuf::from(&workspace.root_path);
    let size_bytes = dir_size(&workspace_root);
    let mut worktree_snapshots = Vec::with_capacity(worktrees.len());
    for worktree in worktrees {
        let root = PathBuf::from(&worktree.root_path);
        worktree_snapshots.push(WorktreeDiskSnapshot {
            worktree_id: worktree.id.0.to_string(),
            root_path: worktree.root_path.clone(),
            size_bytes: dir_size(&root),
        });
    }
    WorkspaceDiskSnapshot {
        workspace_id: workspace.id.0.to_string(),
        root_path: workspace.root_path,
        size_bytes,
        size_collected_at: Utc::now().to_rfc3339(),
        size_cache_age_ms,
        disk,
        worktrees: worktree_snapshots,
    }
}

fn aggregate_process(
    system: &System,
    children: &HashMap<Pid, Vec<Pid>>,
    pid: u32,
    label: &str,
) -> Option<ResourceProcess> {
    let root = Pid::from_u32(pid);
    system.process(root)?;
    let mut stack = vec![root];
    let mut cpu_pct = 0.0f32;
    let mut memory_bytes = 0u64;
    let mut virtual_memory_bytes = 0u64;
    let mut child_count = 0u64;
    let mut child_processes = Vec::new();
    let mut children_truncated = false;
    while let Some(next) = stack.pop() {
        if let Some(proc) = system.process(next) {
            cpu_pct += proc.cpu_usage();
            // sysinfo reports process memory values in bytes.
            memory_bytes = memory_bytes.saturating_add(proc.memory());
            virtual_memory_bytes = virtual_memory_bytes.saturating_add(proc.virtual_memory());

            if next != root && !children_truncated {
                if child_processes.len() >= MAX_CHILD_PROCESSES {
                    children_truncated = true;
                } else {
                    let cmd = proc.cmd();
                    let cmdline = cmd.first().map(|first| {
                        let basename = Path::new(first)
                            .file_name()
                            .map(|name| name.to_string_lossy().to_string())
                            .unwrap_or_else(|| first.clone());
                        if cmd.len() > 1 {
                            format!("{basename} (args redacted)")
                        } else {
                            basename
                        }
                    });
                    child_processes.push(ResourceChildProcess {
                        pid: next.as_u32(),
                        parent_pid: proc.parent().map(|p| p.as_u32()),
                        name: proc.name().to_string(),
                        cmdline,
                        cpu_pct: proc.cpu_usage(),
                        memory_bytes: proc.memory(),
                        virtual_memory_bytes: proc.virtual_memory(),
                    });
                }
            }
        }
        if let Some(next_children) = children.get(&next) {
            child_count = child_count.saturating_add(next_children.len() as u64);
            stack.extend(next_children.iter().copied());
        }
    }

    child_processes.sort_by(|a, b| {
        b.memory_bytes
            .cmp(&a.memory_bytes)
            .then_with(|| b.cpu_pct.total_cmp(&a.cpu_pct))
            .then_with(|| a.pid.cmp(&b.pid))
    });
    Some(ResourceProcess {
        label: label.to_string(),
        pid,
        cpu_pct,
        memory_bytes,
        virtual_memory_bytes,
        child_count,
        children: child_processes,
        children_truncated,
    })
}

impl From<&Disk> for DiskSnapshot {
    fn from(disk: &Disk) -> Self {
        let file_system = disk.file_system().to_string_lossy().to_string();
        DiskSnapshot {
            name: disk.name().to_string_lossy().to_string(),
            mount_point: disk.mount_point().to_string_lossy().to_string(),
            total_bytes: disk.total_space(),
            available_bytes: disk.available_space(),
            file_system,
        }
    }
}

fn dir_size(root: &Path) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            let entries = match std::fs::read_dir(&path) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                stack.push(entry.path());
            }
        } else {
            total = total.saturating_add(metadata.len());
        }
    }
    total
}
