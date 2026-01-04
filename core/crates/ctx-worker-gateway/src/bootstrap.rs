use ctx_worker_protocol::RepoSpec;

pub struct BootstrapSpec<'a> {
    pub worker_id: &'a str,
    pub gateway_url: &'a str,
    pub gateway_token: Option<&'a str>,
    pub base_commit: &'a str,
    pub diff_debounce_ms: u64,
    pub repo: &'a RepoSpec,
    pub shim_url: &'a str,
    pub workdir: &'a str,
    pub mount_path: &'a str,
    pub mount_device_candidates: Vec<String>,
}

pub fn render_bootstrap_script(spec: &BootstrapSpec<'_>) -> String {
    let mut script = String::new();
    script.push_str("#!/usr/bin/env bash\n");
    script.push_str("set -euo pipefail\n\n");

    script.push_str(&format!(
        "CTX_WORKER_ID={}\n",
        shell_quote(spec.worker_id)
    ));
    script.push_str(&format!(
        "CTX_GATEWAY_URL={}\n",
        shell_quote(spec.gateway_url)
    ));
    if let Some(token) = spec.gateway_token {
        script.push_str(&format!(
            "CTX_WORKER_GATEWAY_TOKEN={}\n",
            shell_quote(token)
        ));
    }
    script.push_str(&format!(
        "CTX_BASE_COMMIT={}\n",
        shell_quote(spec.base_commit)
    ));
    script.push_str(&format!(
        "CTX_DIFF_DEBOUNCE_MS={}\n",
        shell_quote(&spec.diff_debounce_ms.to_string())
    ));
    script.push_str(&format!(
        "CTX_SHIM_URL={}\n",
        shell_quote(spec.shim_url)
    ));
    script.push_str(&format!(
        "CTX_WORKDIR={}\n",
        shell_quote(spec.workdir)
    ));
    script.push_str(&format!(
        "CTX_MOUNT_PATH={}\n",
        shell_quote(spec.mount_path)
    ));

    match spec.repo {
        RepoSpec::Git { url, reference } => {
            script.push_str(&format!("CTX_REPO_TYPE={}\n", shell_quote("git")));
            script.push_str(&format!("CTX_REPO_URL={}\n", shell_quote(url)));
            script.push_str(&format!(
                "CTX_REPO_REF={}\n",
                shell_quote(reference)
            ));
        }
        RepoSpec::Archive { url } => {
            script.push_str(&format!("CTX_REPO_TYPE={}\n", shell_quote("archive")));
            script.push_str(&format!(
                "CTX_REPO_ARCHIVE_URL={}\n",
                shell_quote(url)
            ));
        }
        RepoSpec::Local { .. } => {
            script.push_str(&format!("CTX_REPO_TYPE={}\n", shell_quote("local")));
        }
    }

    script.push_str("\nlog() { echo \"[ctx-worker] $*\"; }\n\n");
    script.push_str("install_deps() {\n");
    script.push_str("  if command -v apt-get >/dev/null 2>&1; then\n");
    script.push_str("    apt-get update -y >/dev/null 2>&1 || true\n");
    script.push_str("    apt-get install -y git curl tar >/dev/null 2>&1 || true\n");
    script.push_str("  elif command -v dnf >/dev/null 2>&1; then\n");
    script.push_str("    dnf install -y git curl tar >/dev/null 2>&1 || true\n");
    script.push_str("  elif command -v yum >/dev/null 2>&1; then\n");
    script.push_str("    yum install -y git curl tar >/dev/null 2>&1 || true\n");
    script.push_str("  fi\n");
    script.push_str("}\n\n");

    if !spec.mount_device_candidates.is_empty() {
        script.push_str("resolve_mount_device() {\n");
        script.push_str("  local candidates=(\n");
        for candidate in &spec.mount_device_candidates {
            script.push_str(&format!("    {}\n", shell_quote(candidate)));
        }
        script.push_str("  )\n");
        script.push_str("  for i in $(seq 1 60); do\n");
        script.push_str("    for candidate in \"${candidates[@]}\"; do\n");
        script.push_str("      if [ -b \"$candidate\" ]; then\n");
        script.push_str("        echo \"$candidate\"\n");
        script.push_str("        return 0\n");
        script.push_str("      fi\n");
        script.push_str("    done\n");
        script.push_str("    sleep 1\n");
        script.push_str("  done\n");
        script.push_str("  return 1\n");
        script.push_str("}\n\n");

        script.push_str("mount_session_disk() {\n");
        script.push_str("  local device\n");
        script.push_str("  device=$(resolve_mount_device || true)\n");
        script.push_str("  if [ -z \"$device\" ]; then\n");
        script.push_str("    log \"no session disk detected; continuing without mount\"\n");
        script.push_str("    return 0\n");
        script.push_str("  fi\n");
        script.push_str("  if ! blkid \"$device\" >/dev/null 2>&1; then\n");
        script.push_str("    log \"formatting session disk $device\"\n");
        script.push_str("    mkfs.ext4 -F \"$device\" >/dev/null 2>&1\n");
        script.push_str("  fi\n");
        script.push_str("  mkdir -p \"$CTX_MOUNT_PATH\"\n");
        script.push_str("  mount \"$device\" \"$CTX_MOUNT_PATH\" || mount -o rw \"$device\" \"$CTX_MOUNT_PATH\"\n");
        script.push_str("}\n\n");
    } else {
        script.push_str("mount_session_disk() { :; }\n\n");
    }

    script.push_str("hydrate_repo() {\n");
    script.push_str("  mkdir -p \"$CTX_WORKDIR\"\n");
    script.push_str("  case \"$CTX_REPO_TYPE\" in\n");
    script.push_str("    git)\n");
    script.push_str("      if [ ! -d \"$CTX_WORKDIR/.git\" ]; then\n");
    script.push_str("        git clone \"$CTX_REPO_URL\" \"$CTX_WORKDIR\"\n");
    script.push_str("      fi\n");
    script.push_str("      cd \"$CTX_WORKDIR\"\n");
    script.push_str("      if [ -n \"${CTX_REPO_REF:-}\" ]; then\n");
    script.push_str("        git fetch --all --tags --prune >/dev/null 2>&1 || true\n");
    script.push_str("        git checkout \"$CTX_REPO_REF\" >/dev/null 2>&1 || git checkout -B \"$CTX_REPO_REF\" \"origin/$CTX_REPO_REF\" >/dev/null 2>&1 || true\n");
    script.push_str("      fi\n");
    script.push_str("      ;;\n");
    script.push_str("    archive)\n");
    script.push_str("      if [ ! -f \"$CTX_WORKDIR/.ctx_archive_done\" ]; then\n");
    script.push_str("        curl -fsSL \"$CTX_REPO_ARCHIVE_URL\" -o /tmp/ctx-repo.tgz\n");
    script.push_str("        tar -xzf /tmp/ctx-repo.tgz -C \"$CTX_WORKDIR\" --strip-components=1\n");
    script.push_str("        touch \"$CTX_WORKDIR/.ctx_archive_done\"\n");
    script.push_str("      fi\n");
    script.push_str("      ;;\n");
    script.push_str("    local)\n");
    script.push_str("      log \"local repo spec is not supported on cloud workers\"\n");
    script.push_str("      exit 1\n");
    script.push_str("      ;;\n");
    script.push_str("    *)\n");
    script.push_str("      log \"unknown repo type $CTX_REPO_TYPE\"\n");
    script.push_str("      exit 1\n");
    script.push_str("      ;;\n");
    script.push_str("  esac\n");
    script.push_str("}\n\n");

    script.push_str("install_shim() {\n");
    script.push_str("  if [ ! -x /usr/local/bin/ctx-worker-shim ]; then\n");
    script.push_str("    curl -fsSL \"$CTX_SHIM_URL\" -o /usr/local/bin/ctx-worker-shim\n");
    script.push_str("    chmod +x /usr/local/bin/ctx-worker-shim\n");
    script.push_str("  fi\n");
    script.push_str("}\n\n");

    script.push_str("start_shim() {\n");
    script.push_str("  log \"starting ctx-worker-shim\"\n");
    script.push_str("  export CTX_WORKER_ID\n");
    script.push_str("  export CTX_GATEWAY_URL\n");
    script.push_str("  if [ -n \"${CTX_WORKER_GATEWAY_TOKEN:-}\" ]; then export CTX_WORKER_GATEWAY_TOKEN; fi\n");
    script.push_str("  export CTX_BASE_COMMIT\n");
    script.push_str("  export CTX_DIFF_DEBOUNCE_MS\n");
    script.push_str("  export CTX_WORKDIR\n");
    script.push_str("  nohup /usr/local/bin/ctx-worker-shim \\\n");
    script.push_str("    --gateway-url \"$CTX_GATEWAY_URL\" \\\n");
    script.push_str("    --worker-id \"$CTX_WORKER_ID\" \\\n");
    script.push_str("    --workdir \"$CTX_WORKDIR\" \\\n");
    script.push_str("    --base-commit \"$CTX_BASE_COMMIT\" \\\n");
    script.push_str("    --diff-debounce-ms \"$CTX_DIFF_DEBOUNCE_MS\" \\\n");
    script.push_str("    >/var/log/ctx-worker-shim.log 2>&1 &\n");
    script.push_str("}\n\n");

    script.push_str("main() {\n");
    script.push_str("  install_deps\n");
    script.push_str("  mount_session_disk\n");
    script.push_str("  hydrate_repo\n");
    script.push_str("  install_shim\n");
    script.push_str("  start_shim\n");
    script.push_str("}\n\n");
    script.push_str("main \"$@\"\n");

    script
}

fn shell_quote(value: &str) -> String {
    let mut out = String::from("'");
    for ch in value.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}
