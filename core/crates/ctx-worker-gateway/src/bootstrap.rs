use std::collections::HashMap;

use ctx_worker_protocol::RepoSpec;

pub struct BootstrapSpec<'a> {
    pub worker_id: &'a str,
    pub gateway_url: &'a str,
    pub gateway_token: Option<&'a str>,
    pub base_commit: &'a str,
    pub diff_debounce_ms: u64,
    pub repo: &'a RepoSpec,
    pub provider_id: Option<&'a str>,
    pub env: &'a HashMap<String, String>,
    pub shim_url: &'a str,
    pub workdir: &'a str,
    pub mount_path: &'a str,
    pub mount_device_candidates: Vec<String>,
}

pub fn render_bootstrap_script(spec: &BootstrapSpec<'_>) -> String {
    let mut script = String::new();
    script.push_str("#!/usr/bin/env bash\n");
    script.push_str("set -euo pipefail\n\n");

    script.push_str(&format!("CTX_WORKER_ID={}\n", shell_quote(spec.worker_id)));
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
    script.push_str(&format!("CTX_SHIM_URL={}\n", shell_quote(spec.shim_url)));
    script.push_str(&format!("CTX_WORKDIR={}\n", shell_quote(spec.workdir)));
    script.push_str(&format!(
        "CTX_MOUNT_PATH={}\n",
        shell_quote(spec.mount_path)
    ));
    if let Some(provider_id) = spec.provider_id {
        script.push_str(&format!("CTX_PROVIDER_ID={}\n", shell_quote(provider_id)));
    }
    if !spec.env.is_empty() {
        let mut keys: Vec<&String> = spec.env.keys().collect();
        keys.sort();
        for key in keys {
            if !is_safe_env_key(key) {
                continue;
            }
            if let Some(value) = spec.env.get(key) {
                script.push_str(&format!("{}={}\n", key, shell_quote(value)));
            }
        }
    }

    match spec.repo {
        RepoSpec::Git { url, reference } => {
            script.push_str(&format!("CTX_REPO_TYPE={}\n", shell_quote("git")));
            script.push_str(&format!("CTX_REPO_URL={}\n", shell_quote(url)));
            script.push_str(&format!("CTX_REPO_REF={}\n", shell_quote(reference)));
        }
        RepoSpec::Archive { url } => {
            script.push_str(&format!("CTX_REPO_TYPE={}\n", shell_quote("archive")));
            script.push_str(&format!("CTX_REPO_ARCHIVE_URL={}\n", shell_quote(url)));
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

    script.push_str("install_gateway_ca() {\n");
    script.push_str("  if [ -z \"${CTX_GATEWAY_CA_B64:-}\" ]; then\n");
    script.push_str("    return 0\n");
    script.push_str("  fi\n");
    script.push_str("  local ca_path=\"/etc/ctx-gateway-ca.pem\"\n");
    script.push_str("  if base64 --help 2>&1 | grep -q -- '--decode'; then\n");
    script.push_str("    echo \"$CTX_GATEWAY_CA_B64\" | base64 --decode > \"$ca_path\"\n");
    script.push_str("  else\n");
    script.push_str("    echo \"$CTX_GATEWAY_CA_B64\" | base64 -d > \"$ca_path\"\n");
    script.push_str("  fi\n");
    script.push_str("  chmod 600 \"$ca_path\"\n");
    script.push_str("  export CTX_GATEWAY_CA_PATH=\"$ca_path\"\n");
    script.push_str("}\n\n");

    script.push_str("curl_gateway() {\n");
    script.push_str("  local url=\"$1\"\n");
    script.push_str("  local dest=\"$2\"\n");
    script.push_str(
        "  if [ -n \"${CTX_GATEWAY_CA_PATH:-}\" ] && [ -n \"${CTX_GATEWAY_URL:-}\" ]; then\n",
    );
    script.push_str("    case \"$url\" in\n");
    script.push_str("      \"${CTX_GATEWAY_URL%/}\"*)\n");
    script.push_str("        curl --cacert \"$CTX_GATEWAY_CA_PATH\" -fsSL \"$url\" -o \"$dest\"\n");
    script.push_str("        return $?\n");
    script.push_str("        ;;\n");
    script.push_str("    esac\n");
    script.push_str("  fi\n");
    script.push_str("  curl -fsSL \"$url\" -o \"$dest\"\n");
    script.push_str("}\n\n");

    script.push_str("install_codex_auth() {\n");
    script.push_str("  if [ -n \"${CTX_CODEX_AUTH_B64:-}\" ]; then\n");
    script.push_str("    mkdir -p /root/.codex\n");
    script.push_str("    if base64 --help 2>&1 | grep -q -- '--decode'; then\n");
    script.push_str(
        "      echo \"$CTX_CODEX_AUTH_B64\" | base64 --decode > /root/.codex/auth.json\n",
    );
    script.push_str("    else\n");
    script.push_str("      echo \"$CTX_CODEX_AUTH_B64\" | base64 -d > /root/.codex/auth.json\n");
    script.push_str("    fi\n");
    script.push_str("    chmod 600 /root/.codex/auth.json\n");
    script.push_str("    unset CTX_CODEX_AUTH_B64\n");
    script.push_str("  fi\n");
    script.push_str("}\n\n");

    script.push_str("install_codex_acp() {\n");
    script.push_str("  if command -v codex-acp >/dev/null 2>&1; then\n");
    script.push_str("    return 0\n");
    script.push_str("  fi\n");
    script.push_str("  local arch\n");
    script.push_str("  arch=$(uname -m || true)\n");
    script.push_str("  local url=\"\"\n");
    script.push_str("  case \"$arch\" in\n");
    script.push_str("    x86_64|amd64)\n");
    script.push_str("      url=\"https://github.com/ctxrs/codex-acp/releases/download/v0.7.4-ctx.4/codex-acp-0.7.4-ctx.4-x86_64-unknown-linux-gnu.tar.gz\"\n");
    script.push_str("      ;;\n");
    script.push_str("    aarch64|arm64)\n");
    script.push_str("      url=\"https://github.com/ctxrs/codex-acp/releases/download/v0.7.4-ctx.4/codex-acp-0.7.4-ctx.4-aarch64-unknown-linux-gnu.tar.gz\"\n");
    script.push_str("      ;;\n");
    script.push_str("    *)\n");
    script.push_str("      log \"unsupported arch for codex-acp: $arch\"\n");
    script.push_str("      return 0\n");
    script.push_str("      ;;\n");
    script.push_str("  esac\n");
    script.push_str("  curl -fsSL \"$url\" -o /tmp/ctx-codex-acp.tgz\n");
    script.push_str("  tar -xzf /tmp/ctx-codex-acp.tgz -C /tmp\n");
    script.push_str("  if [ -f /tmp/codex-acp ]; then\n");
    script.push_str("    mkdir -p /usr/local/bin\n");
    script.push_str("    chmod +x /tmp/codex-acp\n");
    script.push_str("    mv /tmp/codex-acp /usr/local/bin/codex-acp\n");
    script.push_str("  else\n");
    script.push_str("    log \"codex-acp binary missing after extract\"\n");
    script.push_str("  fi\n");
    script.push_str("}\n\n");

    script.push_str("install_node() {\n");
    script
        .push_str("  if command -v node >/dev/null 2>&1 && command -v npm >/dev/null 2>&1; then\n");
    script.push_str("    return 0\n");
    script.push_str("  fi\n");
    script.push_str("  if command -v apt-get >/dev/null 2>&1; then\n");
    script.push_str(
        "    curl -fsSL https://deb.nodesource.com/setup_20.x | bash - >/dev/null 2>&1 || true\n",
    );
    script.push_str("    apt-get install -y nodejs >/dev/null 2>&1 || apt-get install -y nodejs npm >/dev/null 2>&1 || true\n");
    script.push_str("  elif command -v dnf >/dev/null 2>&1; then\n");
    script.push_str("    dnf install -y nodejs npm >/dev/null 2>&1 || true\n");
    script.push_str("  elif command -v yum >/dev/null 2>&1; then\n");
    script.push_str("    yum install -y nodejs npm >/dev/null 2>&1 || true\n");
    script.push_str("  fi\n");
    script.push_str("}\n\n");

    script.push_str("install_claude_acp() {\n");
    script.push_str("  if command -v claude-code-acp >/dev/null 2>&1; then\n");
    script.push_str("    return 0\n");
    script.push_str("  fi\n");
    script.push_str("  install_node\n");
    script.push_str("  if command -v npm >/dev/null 2>&1; then\n");
    script.push_str("    npm install -g @zed-industries/claude-code-acp >/dev/null 2>&1 || true\n");
    script.push_str("  else\n");
    script.push_str("    log \"npm not available; skipping claude-code-acp install\"\n");
    script.push_str("  fi\n");
    script.push_str("}\n\n");

    script.push_str("install_providers() {\n");
    script.push_str("  case \"${CTX_PROVIDER_ID:-}\" in\n");
    script.push_str("    codex)\n");
    script.push_str("      install_codex_acp\n");
    script.push_str("      ;;\n");
    script.push_str("    claude)\n");
    script.push_str("      install_claude_acp\n");
    script.push_str("      ;;\n");
    script.push_str("    *)\n");
    script.push_str("      ;;\n");
    script.push_str("  esac\n");
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
    script
        .push_str("        tar -xzf /tmp/ctx-repo.tgz -C \"$CTX_WORKDIR\" --strip-components=1\n");
    script.push_str("        if [ ! -d \"$CTX_WORKDIR/.git\" ]; then\n");
    script.push_str("          git init >/dev/null 2>&1 || true\n");
    script.push_str(
        "          git config user.email \"ctx-worker@localhost\" >/dev/null 2>&1 || true\n",
    );
    script.push_str("          git config user.name \"ctx-worker\" >/dev/null 2>&1 || true\n");
    script.push_str("          git add . >/dev/null 2>&1 || true\n");
    script.push_str("          git commit -m \"ctx base\" >/dev/null 2>&1 || true\n");
    script.push_str("        fi\n");
    script.push_str("        touch \"$CTX_WORKDIR/.ctx_archive_done\"\n");
    script.push_str("      fi\n");
    script.push_str("      if [ -d \"$CTX_WORKDIR/.git\" ]; then\n");
    script.push_str("        CTX_BASE_COMMIT=$(git rev-parse HEAD 2>/dev/null || echo \"HEAD\")\n");
    script.push_str("        export CTX_BASE_COMMIT\n");
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
    script.push_str("    curl_gateway \"$CTX_SHIM_URL\" /usr/local/bin/ctx-worker-shim\n");
    script.push_str("    chmod +x /usr/local/bin/ctx-worker-shim\n");
    script.push_str("  fi\n");
    script.push_str("}\n\n");

    script.push_str("start_shim() {\n");
    script.push_str("  log \"starting ctx-worker-shim\"\n");
    script.push_str("  export CTX_WORKER_ID\n");
    script.push_str("  export CTX_GATEWAY_URL\n");
    script.push_str(
        "  if [ -n \"${CTX_WORKER_GATEWAY_TOKEN:-}\" ]; then export CTX_WORKER_GATEWAY_TOKEN; fi\n",
    );
    script
        .push_str("  if [ -n \"${CTX_GATEWAY_CA_B64:-}\" ]; then export CTX_GATEWAY_CA_B64; fi\n");
    script.push_str("  export CTX_BASE_COMMIT\n");
    script.push_str("  export CTX_DIFF_DEBOUNCE_MS\n");
    script.push_str("  export CTX_WORKDIR\n");
    script.push_str("  export RUST_LOG=${CTX_SHIM_LOG_LEVEL:-info}\n");
    script.push_str("  nohup /usr/local/bin/ctx-worker-shim \\\n");
    script.push_str("    --gateway-url \"$CTX_GATEWAY_URL\" \\\n");
    script.push_str("    --worker-id \"$CTX_WORKER_ID\" \\\n");
    script.push_str("    --workdir \"$CTX_WORKDIR\" \\\n");
    script.push_str("    --base-commit \"$CTX_BASE_COMMIT\" \\\n");
    script.push_str("    --diff-debounce-ms \"$CTX_DIFF_DEBOUNCE_MS\" \\\n");
    script.push_str("    2>&1 | tee -a /var/log/ctx-worker-shim.log &\n");
    script.push_str("}\n\n");

    script.push_str("main() {\n");
    script.push_str("  install_deps\n");
    script.push_str("  install_gateway_ca\n");
    script.push_str("  install_codex_auth\n");
    script.push_str("  install_providers\n");
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

fn is_safe_env_key(key: &str) -> bool {
    if key.is_empty() {
        return false;
    }
    key.chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}
