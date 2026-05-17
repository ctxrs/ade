const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { execFileSync } = require("node:child_process");

const REMOTE_HOST = String(
  process.env.CTX_AUTOMATION_REMOTE_HOST || process.env.CTX_UPDATER_E2E_REMOTE_HOST || "",
).trim();
const REMOTE_USER = String(process.env.CTX_AUTOMATION_REMOTE_USER || process.env.CTX_UPDATER_E2E_REMOTE_USER || "root").trim() || "root";
const REMOTE_PORT = Number.parseInt(String(process.env.CTX_AUTOMATION_REMOTE_PORT || "44099"), 10) || 44099;
const REMOTE_DATA_DIR = String(process.env.CTX_AUTOMATION_REMOTE_DATA_DIR || "").trim() || "/tmp/ctx-updater-proof/daemon";
const REMOTE_CTX_BIN = String(process.env.CTX_AUTOMATION_REMOTE_CTX_BIN || process.env.CTX_UPDATER_E2E_REMOTE_CTX_BIN || "$HOME/.ctx/bin/ctx").trim();
const REMOTE_MANAGED_CTX_BIN = "$HOME/.ctx/bin/ctx";
const SSH_KEY_PATH = String(
  process.env.CTX_UPDATER_E2E_SSH_KEY_PATH || process.env.CTX_AUTOMATION_REMOTE_SSH_KEY_PATH || "",
).trim();
const SSH_CONFIG_PATH = String(
  process.env.CTX_DESKTOP_SSH_CONFIG_PATH || process.env.CTX_AUTOMATION_REMOTE_FIXTURE_SSH_CONFIG || "",
).trim();
const SSH_PORT = Number.parseInt(String(process.env.CTX_AUTOMATION_REMOTE_SSH_PORT || "0"), 10) || 0;
const DOWNLOAD_BASE_URL = String(
  process.env.CTX_UPDATER_E2E_DOWNLOAD_BASE_URL || process.env.SUPABASE_FUNCTIONS_URL || "https://api.ctx.rs/functions/v1",
).trim().replace(/\/+$/u, "");
const BOOTSTRAP_CHANNEL = String(process.env.CTX_UPDATER_E2E_BOOTSTRAP_CHANNEL || "stable").trim() || "stable";
const TARGET_CHANNEL = String(
  process.env.CTX_UPDATER_E2E_RELEASE_CHANNEL
  || process.env.RELEASE_STORAGE_CHANNEL
  || process.env.RELEASE_CHANNEL
  || "stable",
).trim() || "stable";
const REMOTE_ARCH = String(process.env.CTX_UPDATER_E2E_REMOTE_ARCH || "").trim() || "linux-x64";

const remoteBundleDir = `${REMOTE_DATA_DIR.replace(/\/+$/u, "")}/bundles`;
const remoteDaemonLog = `${REMOTE_DATA_DIR.replace(/\/+$/u, "")}/daemon.log`;
const remoteDaemonPidFile = `${REMOTE_DATA_DIR.replace(/\/+$/u, "")}/daemon.pid`;
const remoteAuthFile = `${REMOTE_DATA_DIR.replace(/\/+$/u, "")}/daemon_auth.json`;
const remoteProofRoot = `${REMOTE_DATA_DIR.replace(/\/+$/u, "")}/updater-proof`;

const trimText = (value) => String(value || "").trim();

class BootstrapDaemonUnsupportedError extends Error {
  constructor(message, details = {}) {
    super(message);
    this.name = "BootstrapDaemonUnsupportedError";
    this.details = details;
  }
}

const isBootstrapDaemonUnsupportedError = (error) =>
  error instanceof BootstrapDaemonUnsupportedError || String(error?.name || "") === "BootstrapDaemonUnsupportedError";

const assertRemoteConfigured = () => {
  if (!REMOTE_HOST) {
    throw new Error("remote updater proof requires CTX_AUTOMATION_REMOTE_HOST / CTX_UPDATER_E2E_REMOTE_HOST");
  }
  if (!SSH_KEY_PATH) {
    throw new Error("remote updater proof requires CTX_UPDATER_E2E_SSH_KEY_PATH / CTX_AUTOMATION_REMOTE_SSH_KEY_PATH");
  }
};

const sshArgsForTarget = (targetHost = REMOTE_HOST) => {
  const args = [
    "-o",
    "BatchMode=yes",
    "-o",
    "StrictHostKeyChecking=no",
    "-o",
    "UserKnownHostsFile=/dev/null",
    "-o",
    "ConnectTimeout=12",
  ];
  if (SSH_CONFIG_PATH) {
    args.unshift(SSH_CONFIG_PATH);
    args.unshift("-F");
  } else {
    args.unshift("/dev/null");
    args.unshift("-F");
  }
  if (SSH_PORT > 0) {
    args.push("-p", String(SSH_PORT));
  }
  if (SSH_KEY_PATH) {
    args.unshift(SSH_KEY_PATH);
    args.unshift("-i");
  }
  args.push(`${REMOTE_USER}@${targetHost}`);
  return args;
};

const formatRemoteSshError = (error) => {
  const stderr = String(error?.stderr || "").trim();
  const stdout = String(error?.stdout || "").trim();
  const details = [];
  if (error?.status !== undefined && error.status !== null) {
    details.push(`status=${error.status}`);
  }
  if (error?.signal) {
    details.push(`signal=${error.signal}`);
  }
  const summary = details.length > 0 ? `ssh exited unsuccessfully (${details.join(", ")})` : "ssh command failed";
  return [stderr, stdout, summary].filter(Boolean).join("\n");
};

const remoteSsh = (command, { host = REMOTE_HOST, allowFailure = false, input = undefined } = {}) => {
  assertRemoteConfigured();
  const args = sshArgsForTarget(host);
  args.push(command);
  try {
    return String(execFileSync("ssh", args, {
      encoding: "utf8",
      input,
      stdio: [typeof input === "undefined" ? "ignore" : "pipe", "pipe", "pipe"],
    }) || "");
  } catch (error) {
    if (allowFailure) {
      return String(error.stdout || "");
    }
    throw new Error(formatRemoteSshError(error));
  }
};

const shellQuote = (value) => `'${String(value || "").replace(/'/g, `'\\''`)}'`;

const remoteSh = (script, options = {}) =>
  remoteSsh("bash -s", { ...options, input: script });

const remotePathPrelude = `
normalize_remote_path() {
  local input="$1"
  case "$input" in
    "~")
      printf '%s\\n' "$HOME"
      ;;
    "~/"*)
      printf '%s/%s\\n' "$HOME" "\${input#~/}"
      ;;
    "\\$HOME")
      printf '%s\\n' "$HOME"
      ;;
    "\\$HOME/"*)
      printf '%s/%s\\n' "$HOME" "\${input#\\$HOME/}"
      ;;
    *)
      printf '%s\\n' "$input"
      ;;
  esac
}
`;

const remotePlatformKey = () => {
  if (REMOTE_ARCH === "linux-arm64") return "linux-arm64";
  return "linux-x64";
};

const fetchManifest = async (channel) => {
  const response = await fetch(`${DOWNLOAD_BASE_URL}/releases/${encodeURIComponent(channel)}/latest.json`);
  if (!response.ok) {
    throw new Error(`manifest fetch failed for ${channel}: HTTP ${response.status}`);
  }
  return await response.json();
};

const resolvePlatformEntry = (manifest, channel) => {
  const platform = remotePlatformKey();
  const platformEntry = manifest?.platforms?.[platform];
  if (!platformEntry?.daemon?.url_path || !platformEntry?.daemon?.sha256) {
    throw new Error(`manifest ${channel} missing daemon artifact for ${platform}`);
  }
  if (!platformEntry?.appimage?.url_path || !platformEntry?.appimage?.sha256) {
    throw new Error(`manifest ${channel} missing appimage artifact for ${platform}`);
  }
  return {
    platform,
    daemon: platformEntry.daemon,
    appimage: platformEntry.appimage,
    version: String(manifest?.latest_version || "").trim(),
  };
};

const remoteHttpJson = (method, apiPath, body, { token } = {}) => {
  const authToken = trimText(token);
  if (!authToken) {
    throw new Error(`remote HTTP ${method} ${apiPath} requires auth token`);
  }
  const bodyJson = typeof body === "undefined" ? "" : JSON.stringify(body);
  const bodyB64 = Buffer.from(bodyJson, "utf8").toString("base64");
  const authHeaderB64 = Buffer.from(`authorization: Bearer ${authToken}`, "utf8").toString("base64");
  const contentTypeB64 = Buffer.from("content-type: application/json", "utf8").toString("base64");
  const url = `http://127.0.0.1:${REMOTE_PORT}${apiPath}`;
  const script = `
set -euo pipefail
tmp_response="$(mktemp)"
tmp_request_body="$(mktemp)"
tmp_curl_config="$(mktemp)"
cleanup() { rm -f "$tmp_response" "$tmp_request_body" "$tmp_curl_config"; }
trap cleanup EXIT
base64 -d >"$tmp_request_body" <<'__CTX_REMOTE_HTTP_BODY_B64__'
${bodyB64}
__CTX_REMOTE_HTTP_BODY_B64__
auth_header="$(base64 -d <<'__CTX_REMOTE_HTTP_AUTH_B64__'
${authHeaderB64}
__CTX_REMOTE_HTTP_AUTH_B64__
)"
content_type_header="$(base64 -d <<'__CTX_REMOTE_HTTP_CONTENT_TYPE_B64__'
${contentTypeB64}
__CTX_REMOTE_HTTP_CONTENT_TYPE_B64__
)"
{
  printf 'silent\\n'
  printf 'show-error\\n'
  printf 'request = "%s"\\n' ${shellQuote(method)}
  printf 'url = "%s"\\n' ${shellQuote(url)}
  printf 'output = "%s"\\n' "$tmp_response"
  printf 'write-out = "%%{http_code}"\\n'
  printf 'header = "%s"\\n' "$auth_header"
  printf 'header = "%s"\\n' "$content_type_header"
  ${typeof body === "undefined" ? "" : "printf 'data-binary = \"@%s\"\\n' \"$tmp_request_body\""}
} >"$tmp_curl_config"
status="$(curl --config "$tmp_curl_config")"
cat "$tmp_response"
printf '\\n__HTTP_STATUS__:%s\\n' "$status"
`;
  const raw = remoteSh(script);
  const marker = "\n__HTTP_STATUS__:";
  const idx = raw.lastIndexOf(marker);
  if (idx < 0) {
    throw new Error(`remote HTTP response missing status marker: ${raw}`);
  }
  const payloadText = raw.slice(0, idx).trim();
  const status = Number.parseInt(raw.slice(idx + marker.length).trim(), 10);
  let payload = {};
  if (payloadText) {
    try {
      payload = JSON.parse(payloadText);
    } catch {
      payload = { raw: payloadText };
    }
  }
  return { status, payload };
};

const readRemoteAuthToken = () => {
  const raw = remoteSh(`cat -- ${shellQuote(remoteAuthFile)}`);
  const parsed = JSON.parse(String(raw || "").trim());
  const token = trimText(parsed?.token);
  if (!token) {
    throw new Error(`remote daemon auth file ${remoteAuthFile} missing token`);
  }
  return token;
};

const readRemoteHealth = () => {
  const raw = remoteSh(`curl -fsS http://127.0.0.1:${REMOTE_PORT}/api/health`);
  return JSON.parse(String(raw || "").trim());
};

const readRemoteVersion = () => trimText(readRemoteHealth()?.daemon_version);

const readRemoteDaemonDiagnostics = () =>
  remoteSh(`
set +e
echo "__daemon_log__"
tail -200 ${shellQuote(remoteDaemonLog)} 2>&1
echo "__daemon_pid__"
cat ${shellQuote(remoteDaemonPidFile)} 2>&1
echo "__daemon_process__"
pid="$(cat ${shellQuote(remoteDaemonPidFile)} 2>/dev/null || true)"
if [[ -n "$pid" ]]; then
  ps -fp "$pid" 2>&1
fi
echo "__listeners__"
if command -v ss >/dev/null 2>&1; then
  ss -ltnp 2>&1 | grep -E ':${REMOTE_PORT}\\b|State' || true
elif command -v lsof >/dev/null 2>&1; then
  lsof -iTCP:${REMOTE_PORT} -sTCP:LISTEN 2>&1 || true
fi
echo "__data_dir__"
find ${shellQuote(REMOTE_DATA_DIR)} -maxdepth 3 -mindepth 1 -print 2>&1 | sort | head -200
`, { allowFailure: true }).trim();

const waitForRemoteHealth = async ({ timeoutMs = 120000, intervalMs = 1000 } = {}) => {
  const started = Date.now();
  let lastError = null;
  while (Date.now() - started < timeoutMs) {
    try {
      const health = readRemoteHealth();
      if (trimText(health?.daemon_version)) return health;
    } catch (error) {
      lastError = error;
    }
    await browser.pause(intervalMs);
  }
  throw lastError || new Error(`remote daemon health timed out after ${timeoutMs}ms`);
};

const waitForRemoteVersionChange = async (
  previousVersion,
  {
    timeoutMs = 180000,
    intervalMs = 1000,
  } = {},
) => {
  const started = Date.now();
  let currentVersion = "";
  while (Date.now() - started < timeoutMs) {
    try {
      currentVersion = readRemoteVersion();
      if (currentVersion && currentVersion !== previousVersion) {
        return currentVersion;
      }
    } catch {
      // replacement may briefly flap health while restarting
    }
    await browser.pause(intervalMs);
  }
  throw new Error(`remote daemon version did not change from '${previousVersion}' within ${timeoutMs}ms (last='${currentVersion}')`);
};

const bootstrapRemoteDaemon = async ({
  bootstrapChannel = BOOTSTRAP_CHANNEL,
  updateChannel = TARGET_CHANNEL,
  autoUpdateIntervalSecs = 60,
} = {}) => {
  const manifest = await fetchManifest(bootstrapChannel);
  const platform = resolvePlatformEntry(manifest, bootstrapChannel);
  const daemonUrl = `${DOWNLOAD_BASE_URL}${platform.daemon.url_path}`;
  const appImageUrl = `${DOWNLOAD_BASE_URL}${platform.appimage.url_path}`;
  const bootstrapVersion = platform.version;
  const bootstrapRunId = `${Date.now()}-${process.pid}`;
  const script = `
set -euo pipefail
${remotePathPrelude}
data_dir=${shellQuote(REMOTE_DATA_DIR)}
ctx_bin="$(normalize_remote_path ${shellQuote(REMOTE_MANAGED_CTX_BIN)})"
bundle_dir=${shellQuote(remoteBundleDir)}
proof_root=${shellQuote(remoteProofRoot)}
pid_file=${shellQuote(remoteDaemonPidFile)}
log_file=${shellQuote(remoteDaemonLog)}
mkdir -p "$proof_root"
tmp_dir="$proof_root/bootstrap-${bootstrapRunId}"
rm -rf "$tmp_dir"
mkdir -p "$tmp_dir"
cleanup() { rm -rf "$tmp_dir"; }
trap cleanup EXIT
download_checked() {
  local label="$1"
  local url="$2"
  local dest="$3"
  local sha256="$4"
  echo "[remote-updater-proof] downloading $label to $dest" >&2
  mkdir -p "$(dirname "$dest")"
  df -h "$(dirname "$dest")" >&2 || true
  rm -f "$dest.partial"
  curl --fail --show-error --location --retry 3 --retry-delay 2 --connect-timeout 20 --max-time 300 -o "$dest.partial" "$url"
  printf '%s  %s\n' "$sha256" "$dest.partial" | sha256sum -c - >/dev/null
  mv -f "$dest.partial" "$dest"
}
if [[ -f "$pid_file" ]]; then
  pid="$(cat "$pid_file" || true)"
  if [[ -n "$pid" ]] && kill -0 "$pid" >/dev/null 2>&1; then
    kill "$pid" >/dev/null 2>&1 || true
    sleep 1
    kill -9 "$pid" >/dev/null 2>&1 || true
  fi
  rm -f "$pid_file"
fi
if command -v lsof >/dev/null 2>&1; then
  listener_pid="$(lsof -tiTCP:${REMOTE_PORT} -sTCP:LISTEN 2>/dev/null | head -n 1 || true)"
  if [[ -n "$listener_pid" ]]; then
    kill "$listener_pid" >/dev/null 2>&1 || true
    sleep 1
    kill -9 "$listener_pid" >/dev/null 2>&1 || true
  fi
fi
rm -rf "$data_dir" "$bundle_dir"
mkdir -p "$(dirname "$ctx_bin")" "$data_dir" "$bundle_dir"
download_checked daemon ${shellQuote(daemonUrl)} "$tmp_dir/ctx" ${shellQuote(platform.daemon.sha256)}
if ! "$tmp_dir/ctx" serve --help >/dev/null 2>"$tmp_dir/ctx-serve-help.err"; then
  install -m 755 "$tmp_dir/ctx" "$ctx_bin"
  echo "__CTX_BOOTSTRAP_DAEMON_UNSUPPORTED__" >&2
  echo "bootstrap daemon artifact for ${shellQuote(bootstrapChannel)} is not a headless ctx daemon" >&2
  cat "$tmp_dir/ctx-serve-help.err" >&2 || true
  exit 86
fi
install -m 755 "$tmp_dir/ctx" "$ctx_bin"
download_checked appimage ${shellQuote(appImageUrl)} "$tmp_dir/ctx.AppImage" ${shellQuote(platform.appimage.sha256)}
chmod +x "$tmp_dir/ctx.AppImage"
(
  cd "$tmp_dir"
  "$tmp_dir/ctx.AppImage" --appimage-extract >/dev/null
)
bundles_src="$(find "$tmp_dir/squashfs-root" -type d -path '*/bundles' -print -quit)"
if [[ -z "$bundles_src" ]]; then
  echo "remote bootstrap failed: bundles directory missing from AppImage" >&2
  exit 1
fi
cp -R "$bundles_src/." "$bundle_dir/"
nohup env \
  CTX_BUNDLE_DIR="$bundle_dir" \
  CTX_MANAGED_DAEMON_AUTO_UPDATE=1 \
  CTX_DAEMON_UPDATE_CHANNEL=${shellQuote(updateChannel)} \
  CTX_DAEMON_UPDATE_BASE_URL=${shellQuote(DOWNLOAD_BASE_URL)} \
  CTX_MANAGED_DAEMON_AUTO_UPDATE_INTERVAL_SECS=${shellQuote(String(autoUpdateIntervalSecs))} \
  "$ctx_bin" serve --bind 127.0.0.1:${REMOTE_PORT} --data-dir "$data_dir" >"$log_file" 2>&1 < /dev/null &
echo $! >"$pid_file"
`;
  try {
    remoteSh(script);
  } catch (error) {
    if (String(error?.message || error).includes("__CTX_BOOTSTRAP_DAEMON_UNSUPPORTED__")) {
      throw new BootstrapDaemonUnsupportedError(
        `bootstrap daemon artifact for ${bootstrapChannel} cannot run ctx serve`,
        {
          bootstrap_channel: bootstrapChannel,
          target_channel: updateChannel,
          bootstrap_version: bootstrapVersion,
          platform: platform.platform,
        },
      );
    }
    throw error;
  }
  try {
    await waitForRemoteHealth({ timeoutMs: 120000 });
  } catch (error) {
    const diagnostics = readRemoteDaemonDiagnostics();
    throw new Error(`${String(error?.message || error)}\nremote daemon bootstrap diagnostics:\n${diagnostics}`);
  }
  return {
    bootstrap_channel: bootstrapChannel,
    target_channel: updateChannel,
    bootstrap_version: bootstrapVersion,
  };
};

const createRemoteWorkspaceRepo = (label) => {
  const remoteRoot = `${remoteProofRoot}/${label}-workspace`;
  const script = `
set -euo pipefail
root=${shellQuote(remoteRoot)}
rm -rf "$root"
mkdir -p "$root"
cd "$root"
git init >/dev/null 2>&1
git config user.email updater-proof@example.com
git config user.name UpdaterProof
printf '%s\n' 'remote updater proof' > README.md
git add README.md >/dev/null 2>&1
git commit -m init >/dev/null 2>&1
`;
  remoteSh(script);
  return remoteRoot;
};

const remoteProviderInstallAndConfigure = ({
  workspaceId,
  token,
  providerId = "codex",
  endpointName,
  installTarget = "host",
}) => {
  const apiKey = trimText(process.env.OPENROUTER_API_KEY);
  if (!apiKey) {
    throw new Error("OPENROUTER_API_KEY is required for remote updater proof");
  }
  const baseUrl = trimText(process.env.OPENROUTER_BASE_URL) || "https://openrouter.ai/api/v1";
  const modelOverride = trimText(process.env.CTX_E2E_OPENROUTER_MODEL_OVERRIDE) || (providerId === "qwen"
    ? "google/gemini-2.5-flash"
    : providerId === "pi"
      ? "google/gemini-3-flash-preview"
      : "openai/gpt-4o-mini");

  const startInstall = remoteHttpJson("POST", `/api/providers/${providerId}/install?target=${installTarget}`, {}, { token });
  if (startInstall.status !== 200) {
    throw new Error(`remote provider install start failed (${startInstall.status}): ${JSON.stringify(startInstall.payload || null)}`);
  }
  const installId = trimText(startInstall.payload?.install_id);
  if (!installId) {
    throw new Error(`remote provider install missing install_id: ${JSON.stringify(startInstall.payload || null)}`);
  }
  return { installId, baseUrl, apiKey, modelOverride, endpointName };
};

const waitRemoteProviderInstalled = async ({ token, providerId, installId, timeoutMs = 180000, pollMs = 2000 }) => {
  const started = Date.now();
  while (Date.now() - started < timeoutMs) {
    const poll = remoteHttpJson("GET", `/api/providers/install/${installId}`, undefined, { token });
    if (poll.status !== 200) {
      throw new Error(`remote provider install poll failed (${poll.status})`);
    }
    const state = trimText(poll.payload?.state).toLowerCase();
    if (state === "succeeded") return poll.payload;
    if (state === "failed" || state === "cancelled") {
      throw new Error(`remote provider install ${state}: ${JSON.stringify(poll.payload || null)}`);
    }
    await browser.pause(pollMs);
  }
  throw new Error(`remote provider install timed out for ${providerId}`);
};

const configureRemoteProvider = ({ token, workspaceId, providerId, endpointName, baseUrl, apiKey, modelOverride }) => {
  const endpointResp = remoteHttpJson("POST", `/api/providers/${providerId}/harness_config/endpoints`, {
    name: endpointName,
    base_url: baseUrl,
    auth_type: "api_key",
    api_key: apiKey,
    model_override: modelOverride,
  }, { token });
  if (endpointResp.status !== 200) {
    throw new Error(`remote endpoint upsert failed (${endpointResp.status}): ${JSON.stringify(endpointResp.payload || null)}`);
  }
  const endpoints = Array.isArray(endpointResp.payload?.endpoints) ? endpointResp.payload.endpoints : [];
  const selected = endpoints.find((entry) => trimText(entry?.name) === endpointName) || {};
  const endpointId = trimText(selected.id || endpointResp.payload?.selected_endpoint_id);
  if (!endpointId) {
    throw new Error(`remote endpoint upsert returned no id: ${JSON.stringify(endpointResp.payload || null)}`);
  }
  const selectResp = remoteHttpJson("POST", `/api/providers/${providerId}/harness_config/select`, {
    source_kind: "endpoint",
    endpoint_id: endpointId,
  }, { token });
  if (selectResp.status !== 200) {
    throw new Error(`remote endpoint select failed (${selectResp.status}): ${JSON.stringify(selectResp.payload || null)}`);
  }
  const verifyResp = remoteHttpJson("POST", `/api/workspaces/${workspaceId}/providers/${providerId}/verify`, {}, { token });
  if (verifyResp.status !== 200 || trimText(verifyResp.payload?.status).toLowerCase() !== "ok") {
    throw new Error(`remote provider verify failed (${verifyResp.status}): ${JSON.stringify(verifyResp.payload || null)}`);
  }
  const optionsResp = remoteHttpJson("GET", `/api/workspaces/${workspaceId}/providers/${providerId}/options`, undefined, { token });
  if (optionsResp.status !== 200) {
    throw new Error(`remote provider options failed (${optionsResp.status}): ${JSON.stringify(optionsResp.payload || null)}`);
  }
  const currentModelId = trimText(optionsResp.payload?.models?.current_model_id);
  const firstModelId = Array.isArray(optionsResp.payload?.models?.models)
    ? trimText(optionsResp.payload.models.models[0]?.id || optionsResp.payload.models.models[0]?.model_id)
    : "";
  const modelId = currentModelId || firstModelId;
  if (!modelId) {
    throw new Error(`remote provider options missing model id: ${JSON.stringify(optionsResp.payload || null)}`);
  }
  return {
    endpoint_id: endpointId,
    model_id: modelId,
    verify_response: verifyResp.payload || null,
  };
};

const createBusyRemoteTurn = async ({ label, providerId = "codex" } = {}) => {
  const token = readRemoteAuthToken();
  const remoteRoot = createRemoteWorkspaceRepo(label);
  const workspaceResp = remoteHttpJson("POST", "/api/workspaces", {
    root_path: remoteRoot,
    name: `updater-proof-${label}`,
  }, { token });
  if (workspaceResp.status !== 200) {
    throw new Error(`remote workspace create failed (${workspaceResp.status}): ${JSON.stringify(workspaceResp.payload || null)}`);
  }
  const workspaceId = trimText(workspaceResp.payload?.id);
  if (!workspaceId) {
    throw new Error(`remote workspace create missing id: ${JSON.stringify(workspaceResp.payload || null)}`);
  }
  const execResp = remoteHttpJson("POST", `/api/workspaces/${workspaceId}/execution_config`, {
    environment: "host",
  }, { token });
  if (execResp.status !== 200) {
    throw new Error(`remote execution config failed (${execResp.status}): ${JSON.stringify(execResp.payload || null)}`);
  }
  const install = remoteProviderInstallAndConfigure({
    workspaceId,
    token,
    providerId,
    endpointName: `updater-proof-${label}-${providerId}`,
  });
  await waitRemoteProviderInstalled({
    token,
    providerId,
    installId: install.installId,
  });
  const configured = configureRemoteProvider({
    token,
    workspaceId,
    providerId,
    endpointName: install.endpointName,
    baseUrl: install.baseUrl,
    apiKey: install.apiKey,
    modelOverride: install.modelOverride,
  });
  const taskResp = remoteHttpJson("POST", `/api/workspaces/${workspaceId}/tasks`, {
    title: `updater-proof-${label}`,
    description: "Remote updater proof long-running task",
    default_session: {
      provider_id: providerId,
      model_id: configured.model_id,
      execution_environment: "host",
    },
  }, { token });
  if (taskResp.status !== 200) {
    throw new Error(`remote task create failed (${taskResp.status}): ${JSON.stringify(taskResp.payload || null)}`);
  }
  const taskId = trimText(taskResp.payload?.id);
  if (!taskId) {
    throw new Error(`remote task create missing id: ${JSON.stringify(taskResp.payload || null)}`);
  }
  const sessionId = trimText(taskResp.payload?.primary_session_id);
  if (!sessionId) {
    throw new Error(`remote task create missing primary_session_id: ${JSON.stringify(taskResp.payload || null)}`);
  }
  const prompt = "Write 350 numbered one-line status bullets about release update verification. Do not stop early.";
  const postResp = remoteHttpJson("POST", `/api/sessions/${sessionId}/messages`, {
    content: prompt,
    delivery: "immediate",
    attachments: [],
  }, { token });
  if (postResp.status !== 200) {
    throw new Error(`remote message post failed (${postResp.status}): ${JSON.stringify(postResp.payload || null)}`);
  }

  const started = Date.now();
  while (Date.now() - started < 120000) {
    const history = remoteHttpJson("GET", `/api/sessions/${sessionId}/history?limit=50`, undefined, { token });
    if (history.status === 200) {
      const turns = Array.isArray(history.payload?.turns) ? history.payload.turns : [];
      const latestTurn = turns.length > 0 ? turns[turns.length - 1] : null;
      const latestStatus = trimText(latestTurn?.status).toLowerCase();
      if (latestStatus === "queued" || latestStatus === "running") {
        return {
          token,
          workspace_id: workspaceId,
          task_id: taskId,
          session_id: sessionId,
          provider_id: providerId,
          model_id: configured.model_id,
        };
      }
      if (latestStatus === "failed" || latestStatus === "cancelled" || latestStatus === "completed") {
        throw new Error(`remote busy turn did not stay active long enough: ${JSON.stringify(history.payload || null)}`);
      }
    }
    await browser.pause(1000);
  }
  throw new Error(`remote busy turn did not enter queued/running state in time`);
};

const waitForRemoteTurnTerminal = async ({ token, sessionId, timeoutMs = 300000, pollMs = 2000 } = {}) => {
  const started = Date.now();
  while (Date.now() - started < timeoutMs) {
    const history = remoteHttpJson("GET", `/api/sessions/${sessionId}/history?limit=200`, undefined, { token });
    if (history.status === 200) {
      const turns = Array.isArray(history.payload?.turns) ? history.payload.turns : [];
      const latestTurn = turns.length > 0 ? turns[turns.length - 1] : null;
      const latestStatus = trimText(latestTurn?.status).toLowerCase();
      if (latestStatus === "completed" || latestStatus === "failed" || latestStatus === "cancelled") {
        return {
          status: latestStatus,
          history: history.payload || null,
        };
      }
    }
    await browser.pause(pollMs);
  }
  throw new Error(`remote turn ${sessionId} did not reach a terminal state in time`);
};

const readRemoteAutoUpdateStatus = () => {
  try {
    const raw = remoteSh(`cat -- ${shellQuote(`${REMOTE_DATA_DIR.replace(/\/+$/u, "")}/updates/managed-daemon-auto-update/status.json`)}`);
    return JSON.parse(String(raw || "").trim());
  } catch {
    return null;
  }
};

module.exports = {
  REMOTE_HOST,
  REMOTE_USER,
  REMOTE_PORT,
  REMOTE_DATA_DIR,
  REMOTE_CTX_BIN,
  REMOTE_MANAGED_CTX_BIN,
  BOOTSTRAP_CHANNEL,
  TARGET_CHANNEL,
  BootstrapDaemonUnsupportedError,
  isBootstrapDaemonUnsupportedError,
  remoteBundleDir,
  remoteDaemonLog,
  shellQuote,
  formatRemoteSshError,
  remoteSh,
  readRemoteHealth,
  readRemoteVersion,
  readRemoteDaemonDiagnostics,
  waitForRemoteHealth,
  waitForRemoteVersionChange,
  bootstrapRemoteDaemon,
  createBusyRemoteTurn,
  waitForRemoteTurnTerminal,
  readRemoteAutoUpdateStatus,
};
