use super::*;

pub(super) fn prepare_guest_worktree(
    data_root: &Path,
    workspace_id: &str,
    worktree_id: &str,
    host_workspace_root: &Path,
    base_commit_sha: &str,
    branch_name: &str,
) -> Result<AvfLinuxGuestWorktreeResponse> {
    let shared_vm = shared_vm_state(data_root)?;
    if !matches!(shared_vm.state, AvfLinuxSharedVmLifecycleState::Running) {
        bail!(
            "shared AVF Linux VM must be running before preparing guest worktrees (state={:?})",
            shared_vm.state
        );
    }
    if !host_workspace_root.is_dir() {
        bail!(
            "host workspace root does not exist: {}",
            host_workspace_root.display()
        );
    }

    let guest_root = guest_worktree_root(worktree_id);
    let guest_user = guest_workspace_user(workspace_id);
    let host_shadow_root = shared_vm_worktree_shadow_root(data_root, workspace_id, worktree_id);
    let metadata_path = shared_vm_worktree_metadata_path(data_root, workspace_id, worktree_id);

    if let Some(existing) = load_guest_worktree_state(&metadata_path)? {
        let existing_guest_user = if existing.guest_user.trim().is_empty() {
            guest_user.clone()
        } else {
            existing.guest_user.clone()
        };
        let matches_request = existing.workspace_id == workspace_id
            && existing.worktree_id == worktree_id
            && existing.host_workspace_root == host_workspace_root
            && existing.base_commit_sha == base_commit_sha
            && existing.branch_name == branch_name
            && existing.host_shadow_root == host_shadow_root
            && existing.guest_root == guest_root
            && host_shadow_root.join(".git").exists();
        if matches_request {
            if !shared_vm.simulated {
                ensure_guest_workspace_user(data_root, &existing_guest_user)?;
                if guest_directory_exists(data_root, &guest_root)? {
                    finalize_guest_worktree_permissions(
                        data_root,
                        &guest_root,
                        &existing_guest_user,
                    )?;
                    return Ok(map_guest_worktree_response(
                        workspace_id,
                        worktree_id,
                        guest_root,
                        existing_guest_user,
                        host_shadow_root,
                        metadata_path,
                        AvfLinuxGuestWorktreeStatus::AlreadyPresent,
                        existing.simulated,
                        existing.notes,
                    ));
                }

                materialize_guest_worktree_from_shadow_root(
                    data_root,
                    &host_shadow_root,
                    &guest_root,
                )?;
                finalize_guest_worktree_permissions(data_root, &guest_root, &existing_guest_user)?;

                let mut notes = existing.notes;
                notes.push(format!(
                    "guest worktree was rematerialized at {} because the prior guest path was missing after VM restart",
                    guest_root.display()
                ));
                let persisted = PersistedGuestWorktreeState {
                    workspace_id: workspace_id.to_string(),
                    worktree_id: worktree_id.to_string(),
                    host_workspace_root: host_workspace_root.to_path_buf(),
                    guest_root: guest_root.clone(),
                    guest_user: existing_guest_user.clone(),
                    host_shadow_root: host_shadow_root.clone(),
                    base_commit_sha: base_commit_sha.to_string(),
                    branch_name: branch_name.to_string(),
                    updated_at: now_timestamp_string(),
                    simulated: false,
                    notes: notes.clone(),
                };
                persist_guest_worktree_state(&metadata_path, &persisted)?;

                return Ok(map_guest_worktree_response(
                    workspace_id,
                    worktree_id,
                    guest_root,
                    existing_guest_user,
                    host_shadow_root,
                    metadata_path,
                    AvfLinuxGuestWorktreeStatus::Prepared,
                    false,
                    notes,
                ));
            }
            return Ok(map_guest_worktree_response(
                workspace_id,
                worktree_id,
                guest_root,
                existing_guest_user,
                host_shadow_root,
                metadata_path,
                AvfLinuxGuestWorktreeStatus::AlreadyPresent,
                existing.simulated,
                existing.notes,
            ));
        }
    }

    best_effort_remove_git_worktree(host_workspace_root, &host_shadow_root);
    if host_shadow_root.exists() {
        fs::remove_dir_all(&host_shadow_root)
            .with_context(|| format!("removing {}", host_shadow_root.display()))?;
    }
    if let Some(parent) = host_shadow_root.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    run_git_worktree_add(
        host_workspace_root,
        &host_shadow_root,
        base_commit_sha,
        branch_name,
    )?;

    let (simulated, notes) = if shared_vm.simulated {
        (
            true,
            vec![
                "guest worktree is staged through the helper-owned shadow root until full AVF guest filesystem import lands".to_string(),
                format!("guest root planned at {}", guest_root.display()),
                format!("guest worktree user reserved as {guest_user}"),
            ],
        )
    } else {
        ensure_guest_workspace_user(data_root, &guest_user)?;
        materialize_guest_worktree_from_shadow_root(data_root, &host_shadow_root, &guest_root)?;
        finalize_guest_worktree_permissions(data_root, &guest_root, &guest_user)?;
        (
            false,
            vec![
                format!(
                    "guest worktree imported from helper shadow root {}",
                    host_shadow_root.display()
                ),
                format!("guest root materialized at {}", guest_root.display()),
                format!("guest worktree user ensured as {guest_user}"),
            ],
        )
    };
    let persisted = PersistedGuestWorktreeState {
        workspace_id: workspace_id.to_string(),
        worktree_id: worktree_id.to_string(),
        host_workspace_root: host_workspace_root.to_path_buf(),
        guest_root: guest_root.clone(),
        guest_user: guest_user.clone(),
        host_shadow_root: host_shadow_root.clone(),
        base_commit_sha: base_commit_sha.to_string(),
        branch_name: branch_name.to_string(),
        updated_at: now_timestamp_string(),
        simulated,
        notes: notes.clone(),
    };
    persist_guest_worktree_state(&metadata_path, &persisted)?;

    Ok(map_guest_worktree_response(
        workspace_id,
        worktree_id,
        guest_root,
        guest_user,
        host_shadow_root,
        metadata_path,
        AvfLinuxGuestWorktreeStatus::Prepared,
        simulated,
        notes,
    ))
}

pub(super) fn stage_guest_worktree_archive_path(worktree_id: &str) -> PathBuf {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    std::env::temp_dir().join(format!(
        "ctx-avf-linux-worktree-{worktree_id}-{}-{millis}.tar",
        std::process::id()
    ))
}

pub(super) fn materialize_guest_worktree_from_shadow_root(
    data_root: &Path,
    host_shadow_root: &Path,
    guest_root: &Path,
) -> Result<()> {
    let control_socket = shared_vm_control_socket_path(data_root);
    let guest_root_parent = guest_root.parent().ok_or_else(|| {
        anyhow::anyhow!(
            "guest worktree root has no parent: {}",
            guest_root.display()
        )
    })?;

    ensure_guest_exec_success(
        &format!(
            "creating guest worktree parent {}",
            guest_root_parent.display()
        ),
        run_guest_exec_capture(
            &control_socket,
            Path::new("/"),
            "/bin/mkdir",
            &[String::from("-p"), guest_root_parent.display().to_string()],
            None,
            HashMap::new(),
            None,
        )?,
    )?;
    ensure_guest_exec_success(
        &format!("clearing guest worktree root {}", guest_root.display()),
        run_guest_exec_capture(
            &control_socket,
            Path::new("/"),
            "/bin/rm",
            &[String::from("-rf"), guest_root.display().to_string()],
            None,
            HashMap::new(),
            None,
        )?,
    )?;
    ensure_guest_exec_success(
        &format!("creating guest worktree root {}", guest_root.display()),
        run_guest_exec_capture(
            &control_socket,
            Path::new("/"),
            "/bin/mkdir",
            &[String::from("-p"), guest_root.display().to_string()],
            None,
            HashMap::new(),
            None,
        )?,
    )?;
    if !guest_directory_exists(data_root, guest_root)? {
        bail!(
            "guest worktree root {} is still missing immediately after creation",
            guest_root.display()
        );
    }

    let archive_path = stage_guest_worktree_archive_path(
        guest_root
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("worktree"),
    );
    let archive_status = Command::new("tar")
        .env("COPYFILE_DISABLE", "1")
        .arg("-C")
        .arg(host_shadow_root)
        .arg("-cf")
        .arg(&archive_path)
        .arg(".")
        .status()
        .with_context(|| {
            format!(
                "creating guest worktree archive from {}",
                host_shadow_root.display()
            )
        })?;
    if !archive_status.success() {
        bail!(
            "creating guest worktree archive from {} failed with status {}",
            host_shadow_root.display(),
            archive_status
        );
    }

    let import_result = (|| -> Result<GuestExecCaptureResult> {
        let mut archive_file = std::fs::File::open(&archive_path)
            .with_context(|| format!("opening {}", archive_path.display()))?;
        run_guest_exec_capture(
            &control_socket,
            Path::new("/"),
            "/usr/bin/tar",
            &[
                String::from("-xpf"),
                String::from("-"),
                String::from("-C"),
                guest_root.display().to_string(),
            ],
            None,
            HashMap::new(),
            Some(&mut archive_file),
        )
    })();
    let _ = fs::remove_file(&archive_path);
    ensure_guest_exec_success(
        &format!(
            "importing staged worktree {} into guest root {}",
            host_shadow_root.display(),
            guest_root.display()
        ),
        import_result?,
    )?;
    if !guest_directory_exists(data_root, guest_root)? {
        bail!(
            "guest worktree root {} disappeared after importing staged worktree {}",
            guest_root.display(),
            host_shadow_root.display()
        );
    }
    Ok(())
}

pub(super) fn best_effort_remove_git_worktree(host_workspace_root: &Path, host_shadow_root: &Path) {
    let _ = Command::new("git")
        .arg("-C")
        .arg(host_workspace_root)
        .arg("worktree")
        .arg("remove")
        .arg("--force")
        .arg(host_shadow_root)
        .output();
    let _ = Command::new("git")
        .arg("-C")
        .arg(host_workspace_root)
        .arg("worktree")
        .arg("prune")
        .output();
}

pub(super) fn run_git_worktree_add(
    host_workspace_root: &Path,
    host_shadow_root: &Path,
    base_commit_sha: &str,
    branch_name: &str,
) -> Result<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(host_workspace_root)
        .arg("worktree")
        .arg("add")
        .arg("--force")
        .arg("-B")
        .arg(branch_name)
        .arg(host_shadow_root)
        .arg(base_commit_sha)
        .output()
        .with_context(|| {
            format!(
                "spawning git worktree add for {} -> {}",
                host_workspace_root.display(),
                host_shadow_root.display()
            )
        })?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let combined = format!("{stderr}\n{stdout}").trim().to_string();
    if combined.is_empty() {
        bail!(
            "git worktree add failed for {} (status: {})",
            host_shadow_root.display(),
            output.status
        );
    }
    bail!(
        "git worktree add failed for {}: {}",
        host_shadow_root.display(),
        combined
    )
}
