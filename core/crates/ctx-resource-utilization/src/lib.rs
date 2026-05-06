use std::collections::HashMap;
#[cfg(target_os = "linux")]
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::Utc;
use serde::Serialize;
use sysinfo::{Disk, Disks, System};

#[cfg(not(target_os = "linux"))]
use sysinfo::Pid;

use ctx_core::ids::WorkspaceId;
use ctx_core::models::{Workspace, Worktree};
use ctx_providers::adapters::ProviderProcessInfo;

mod process;
pub mod process_limits;

const SYSTEM_CACHE_TTL: Duration = Duration::from_millis(750);
const DISK_CACHE_TTL: Duration = Duration::from_secs(30);
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

#[derive(Debug, Clone)]
pub struct ProviderMemorySample {
    pub provider_id: String,
    pub label: String,
    pub pid: u32,
    pub memory_bytes: u64,
    pub tool_memory_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderMemoryRollup {
    pub provider_id: String,
    pub label: String,
    pub pid: u32,
    pub read_ok: bool,
    pub rss_bytes: Option<u64>,
    pub rss_anon_bytes: Option<u64>,
    pub rss_file_bytes: Option<u64>,
    pub rss_shmem_bytes: Option<u64>,
    pub vm_hwm_bytes: Option<u64>,
    pub vm_size_bytes: Option<u64>,
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

#[cfg(target_os = "linux")]
#[derive(Debug, Clone)]
struct ProcCpuSample {
    total_ticks: u64,
    at: Instant,
}

pub struct ResourceSampler {
    system: System,
    disks: Disks,
    last_refresh: Option<Instant>,
    disk_cache: HashMap<WorkspaceId, WorkspaceDiskCache>,
    #[cfg(target_os = "linux")]
    proc_cpu: HashMap<u32, ProcCpuSample>,
    #[cfg(target_os = "linux")]
    clock_ticks: u64,
}

#[derive(Debug, Default, Clone)]
struct ProcMemoryRollup {
    rss_bytes: Option<u64>,
    rss_anon_bytes: Option<u64>,
    rss_file_bytes: Option<u64>,
    rss_shmem_bytes: Option<u64>,
    vm_hwm_bytes: Option<u64>,
    vm_size_bytes: Option<u64>,
}

impl ProcMemoryRollup {
    #[cfg(target_os = "linux")]
    fn is_empty(&self) -> bool {
        self.rss_bytes.is_none()
            && self.rss_anon_bytes.is_none()
            && self.rss_file_bytes.is_none()
            && self.rss_shmem_bytes.is_none()
            && self.vm_hwm_bytes.is_none()
            && self.vm_size_bytes.is_none()
    }
}

impl ResourceSampler {
    pub fn new() -> Self {
        let system = System::new();
        let mut disks = Disks::new_with_refreshed_list();
        disks.refresh();
        #[cfg(target_os = "linux")]
        let clock_ticks = process::clock_ticks_per_second();
        Self {
            system,
            disks,
            last_refresh: None,
            disk_cache: HashMap::new(),
            #[cfg(target_os = "linux")]
            proc_cpu: HashMap::new(),
            #[cfg(target_os = "linux")]
            clock_ticks,
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
        &mut self,
        daemon_pid: u32,
        providers: &[ProviderProcessInfo],
    ) -> ResourceProcesses {
        self.processes_snapshot_light(daemon_pid, providers)
    }

    pub fn processes_snapshot_light(
        &mut self,
        daemon_pid: u32,
        providers: &[ProviderProcessInfo],
    ) -> ResourceProcesses {
        #[cfg(target_os = "linux")]
        {
            let now = Instant::now();
            let mut seen = HashSet::new();
            let daemon = process::proc_snapshot_from_proc(
                daemon_pid,
                "ctx daemon",
                now,
                &mut self.proc_cpu,
                self.clock_ticks,
                &mut seen,
            );
            let providers = providers
                .iter()
                .filter_map(|p| {
                    let label = p.label.clone().unwrap_or_else(|| p.provider_id.clone());
                    process::proc_snapshot_from_proc(
                        p.pid,
                        &label,
                        now,
                        &mut self.proc_cpu,
                        self.clock_ticks,
                        &mut seen,
                    )
                })
                .collect();

            self.proc_cpu.retain(|pid, _| seen.contains(pid));

            ResourceProcesses { daemon, providers }
        }

        #[cfg(not(target_os = "linux"))]
        {
            let mut pids = Vec::with_capacity(1 + providers.len());
            pids.push(Pid::from_u32(daemon_pid));
            for provider in providers {
                pids.push(Pid::from_u32(provider.pid));
            }
            for pid in pids {
                let _ = self
                    .system
                    .refresh_process_specifics(pid, process::process_refresh_kind());
            }

            let daemon = process::aggregate_process_sysinfo(&self.system, daemon_pid, "ctx daemon");
            let providers = providers
                .iter()
                .filter_map(|p| {
                    let label = p.label.clone().unwrap_or_else(|| p.provider_id.clone());
                    process::aggregate_process_sysinfo(&self.system, p.pid, &label)
                })
                .collect();

            ResourceProcesses { daemon, providers }
        }
    }

    pub fn provider_memory_snapshot(
        &mut self,
        providers: &[ProviderProcessInfo],
    ) -> Vec<ProviderMemorySample> {
        #[cfg(target_os = "linux")]
        {
            providers
                .iter()
                .filter_map(|p| {
                    let label = p.label.clone().unwrap_or_else(|| p.provider_id.clone());
                    let rollup = process::read_proc_memory_rollup(p.pid)?;
                    Some(ProviderMemorySample {
                        provider_id: p.provider_id.clone(),
                        label,
                        pid: p.pid,
                        memory_bytes: rollup.rss_bytes.or(rollup.vm_hwm_bytes).unwrap_or(0),
                        tool_memory_bytes: 0,
                    })
                })
                .collect()
        }

        #[cfg(not(target_os = "linux"))]
        {
            for provider in providers {
                let _ = self.system.refresh_process_specifics(
                    Pid::from_u32(provider.pid),
                    process::memory_refresh_kind(),
                );
            }
            providers
                .iter()
                .filter_map(|p| {
                    let label = p.label.clone().unwrap_or_else(|| p.provider_id.clone());
                    let proc = self.system.process(Pid::from_u32(p.pid))?;
                    Some(ProviderMemorySample {
                        provider_id: p.provider_id.clone(),
                        label,
                        pid: p.pid,
                        memory_bytes: proc.memory(),
                        tool_memory_bytes: 0,
                    })
                })
                .collect()
        }
    }

    pub fn provider_memory_rollups(
        &self,
        providers: &[ProviderProcessInfo],
    ) -> Vec<ProviderMemoryRollup> {
        providers
            .iter()
            .map(|p| {
                let label = p.label.clone().unwrap_or_else(|| p.provider_id.clone());
                let rollup = process::read_proc_memory_rollup(p.pid);
                ProviderMemoryRollup {
                    provider_id: p.provider_id.clone(),
                    label,
                    pid: p.pid,
                    read_ok: rollup.is_some(),
                    rss_bytes: rollup.as_ref().and_then(|r| r.rss_bytes),
                    rss_anon_bytes: rollup.as_ref().and_then(|r| r.rss_anon_bytes),
                    rss_file_bytes: rollup.as_ref().and_then(|r| r.rss_file_bytes),
                    rss_shmem_bytes: rollup.as_ref().and_then(|r| r.rss_shmem_bytes),
                    vm_hwm_bytes: rollup.as_ref().and_then(|r| r.vm_hwm_bytes),
                    vm_size_bytes: rollup.as_ref().and_then(|r| r.vm_size_bytes),
                }
            })
            .collect()
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
