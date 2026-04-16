const DEFAULT_GITHUB_API_BASE_URL = "https://api.github.com";
const DEFAULT_REQUIRED_GITHUB_STATUS_CONTEXTS = Object.freeze(["buildkite/ctx-main"]);

function defaultSleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function normalizeGitHubStatusContext(value) {
  return String(value || "").trim();
}

function normalizeGitHubRepoUrl(repoUrl) {
  const normalized = String(repoUrl || "").trim();
  if (!normalized) {
    throw new Error("GitHub repo URL is required");
  }
  const url = new URL(normalized);
  if (url.hostname !== "github.com") {
    throw new Error(`unsupported GitHub repo URL '${repoUrl}'`);
  }
  const pathParts = url.pathname
    .replace(/^\/+/, "")
    .replace(/\/+$/, "")
    .replace(/\.git$/, "")
    .split("/")
    .filter(Boolean);
  if (pathParts.length !== 2) {
    throw new Error(`unsupported GitHub repo URL '${repoUrl}'`);
  }
  return {
    owner: pathParts[0],
    repo: pathParts[1],
  };
}

function buildGitHubCommitStatusUrl({
  apiBaseUrl = DEFAULT_GITHUB_API_BASE_URL,
  commitSha,
  owner,
  repo,
}) {
  return new URL(`/repos/${owner}/${repo}/commits/${commitSha}/status`, apiBaseUrl).toString();
}

async function fetchGitHubCommitStatus({
  apiBaseUrl = DEFAULT_GITHUB_API_BASE_URL,
  commitSha,
  fetchImpl = globalThis.fetch,
  repoUrl,
  token = "",
}) {
  if (typeof fetchImpl !== "function") {
    throw new Error("global fetch is unavailable; provide fetchImpl explicitly");
  }
  const normalizedCommit = String(commitSha || "").trim();
  if (!normalizedCommit) {
    throw new Error("commitSha is required");
  }
  const { owner, repo } = normalizeGitHubRepoUrl(repoUrl);
  const headers = {
    accept: "application/vnd.github+json",
    "user-agent": "ctx-buildbuddy-release",
  };
  const normalizedToken = String(token || "").trim();
  if (normalizedToken) {
    headers.authorization = `Bearer ${normalizedToken}`;
  }
  const response = await fetchImpl(
    buildGitHubCommitStatusUrl({
      apiBaseUrl,
      commitSha: normalizedCommit,
      owner,
      repo,
    }),
    { headers },
  );
  if (!response.ok) {
    throw new Error(`GitHub commit status request failed (${response.status} ${response.statusText})`);
  }
  return response.json();
}

function getGitHubCommitStatusTimestamp(status) {
  const raw = String(status?.updated_at || status?.created_at || "").trim();
  const parsed = Date.parse(raw);
  if (Number.isFinite(parsed)) {
    return parsed;
  }
  const numericId = Number(status?.id || 0);
  return Number.isFinite(numericId) ? numericId : 0;
}

function getLatestGitHubCommitStatusForContext({ context, statuses }) {
  const normalizedContext = normalizeGitHubStatusContext(context);
  if (!normalizedContext) {
    return null;
  }
  return [...(Array.isArray(statuses) ? statuses : [])]
    .filter((status) => normalizeGitHubStatusContext(status?.context) === normalizedContext)
    .sort((left, right) => getGitHubCommitStatusTimestamp(right) - getGitHubCommitStatusTimestamp(left))[0] || null;
}

function classifyGitHubCommitStatus(status) {
  if (!status) {
    return "missing";
  }
  const normalizedState = String(status?.state || "").trim().toLowerCase();
  if (normalizedState === "success") {
    return "passed";
  }
  if (normalizedState === "pending") {
    return "pending";
  }
  return "failed";
}

function buildGitHubStatusCheckRecord({
  context,
  initialState,
  resolution,
  status,
}) {
  return {
    action_name: normalizeGitHubStatusContext(context),
    initial_state: initialState,
    resolution,
    invocation_id: "",
    invocation_url: String(status?.target_url || status?.url || "").trim(),
    source: "github_commit_status",
  };
}

function buildGitHubStatusFailureMessage({
  commitSha,
  context,
  status,
}) {
  const lines = [
    `required GitHub commit status '${context}' failed for commit ${commitSha}`,
  ];
  const description = String(status?.description || "").trim();
  if (description) {
    lines.push(description);
  }
  const targetUrl = String(status?.target_url || status?.url || "").trim();
  if (targetUrl) {
    lines.push(targetUrl);
  }
  return lines.join("\n");
}

async function ensureGitHubCommitStatusesPassed({
  apiBaseUrl = DEFAULT_GITHUB_API_BASE_URL,
  commitSha,
  contexts = DEFAULT_REQUIRED_GITHUB_STATUS_CONTEXTS,
  fetchImpl = globalThis.fetch,
  maxWaitMs = 240 * 60 * 1000,
  pollMs = 5000,
  repoUrl,
  sleepImpl = defaultSleep,
  token = "",
}) {
  const normalizedContexts = [...new Set((Array.isArray(contexts) ? contexts : [])
    .map((value) => normalizeGitHubStatusContext(value))
    .filter(Boolean))];
  const initialStates = new Map();
  const startedAt = Date.now();

  while (true) {
    const payload = await fetchGitHubCommitStatus({
      apiBaseUrl,
      commitSha,
      fetchImpl,
      repoUrl,
      token,
    });
    const statuses = Array.isArray(payload?.statuses) ? payload.statuses : [];
    const checks = normalizedContexts.map((context) => {
      const latest = getLatestGitHubCommitStatusForContext({ context, statuses });
      const state = classifyGitHubCommitStatus(latest);
      if (!initialStates.has(context)) {
        initialStates.set(context, state);
      }
      return {
        context,
        state,
        latest,
      };
    });

    const failed = checks.find((entry) => entry.state === "failed");
    if (failed) {
      throw new Error(
        buildGitHubStatusFailureMessage({
          commitSha,
          context: failed.context,
          status: failed.latest,
        }),
      );
    }

    if (checks.every((entry) => entry.state === "passed")) {
      return {
        required_actions: normalizedContexts,
        checks: checks.map((entry) => buildGitHubStatusCheckRecord({
          context: entry.context,
          initialState: initialStates.get(entry.context) || entry.state,
          resolution: (initialStates.get(entry.context) || entry.state) === "passed"
            ? "already_satisfied"
            : "waited_for_github_status",
          status: entry.latest,
        })),
      };
    }

    if (Date.now() - startedAt > maxWaitMs) {
      const pending = checks.find((entry) => entry.state !== "passed");
      throw new Error(
        `required GitHub commit status '${pending?.context || normalizedContexts[0] || "unknown"}' did not reach success for commit ${commitSha} within ${maxWaitMs}ms`,
      );
    }

    await sleepImpl(pollMs);
  }
}

module.exports = {
  DEFAULT_GITHUB_API_BASE_URL,
  DEFAULT_REQUIRED_GITHUB_STATUS_CONTEXTS,
  buildGitHubCommitStatusUrl,
  classifyGitHubCommitStatus,
  ensureGitHubCommitStatusesPassed,
  fetchGitHubCommitStatus,
  getGitHubCommitStatusTimestamp,
  getLatestGitHubCommitStatusForContext,
  normalizeGitHubRepoUrl,
  normalizeGitHubStatusContext,
};
