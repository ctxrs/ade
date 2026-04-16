const http = require("node:http");
const https = require("node:https");

const DEFAULT_BUILDBUDDY_API_BASE_URL = "https://app.buildbuddy.io";
const RETRYABLE_TRANSPORT_ERROR_CODES = new Set([
  "EPIPE",
  "ECONNRESET",
  "ETIMEDOUT",
  "ECONNABORTED",
  "EAI_AGAIN",
]);

function buildInvocationUrl(invocationId, apiBaseUrl = DEFAULT_BUILDBUDDY_API_BASE_URL) {
  const base = String(apiBaseUrl || DEFAULT_BUILDBUDDY_API_BASE_URL).replace(/\/+$/, "");
  return `${base}/invocation/${invocationId}`;
}

function buildHeaders({ apiKey, body, expectBinary = false }) {
  const headers = {
    "x-buildbuddy-api-key": apiKey,
  };
  if (!expectBinary) {
    headers.accept = "application/json";
  }
  if (body) {
    headers["content-type"] = "application/json";
    headers["content-length"] = Buffer.byteLength(body);
  }
  return headers;
}

function isRetryableTransportError(error) {
  const code = String(error?.code || "").trim().toUpperCase();
  return RETRYABLE_TRANSPORT_ERROR_CODES.has(code);
}

function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function apiRequest({
  apiBaseUrl = DEFAULT_BUILDBUDDY_API_BASE_URL,
  apiKey,
  expectBinary = false,
  pathName,
  payload,
  retryAttempts = 3,
  retryDelayMs = 250,
}) {
  if (!String(apiKey || "").trim()) {
    throw new Error("BuildBuddy API key is required");
  }
  const body = payload === undefined ? "" : JSON.stringify(payload);
  const url = new URL(pathName, apiBaseUrl);
  const transport = url.protocol === "http:" ? http : https;
  let attempt = 1;
  while (attempt <= retryAttempts) {
    try {
      return await new Promise((resolve, reject) => {
        const request = transport.request(
          url,
          {
            method: "POST",
            headers: buildHeaders({ apiKey, body, expectBinary }),
          },
          (response) => {
            const chunks = [];
            response.on("data", (chunk) => {
              chunks.push(Buffer.from(chunk));
            });
            response.on("end", () => {
              const buffer = Buffer.concat(chunks);
              if ((response.statusCode || 0) >= 400) {
                const detail = buffer.toString("utf8");
                reject(new Error(`BuildBuddy API ${pathName} failed (${response.statusCode}): ${detail}`));
                return;
              }
              if (expectBinary) {
                resolve(buffer);
                return;
              }
              try {
                resolve(buffer.length === 0 ? {} : JSON.parse(buffer.toString("utf8")));
              } catch (error) {
                reject(error);
              }
            });
          },
        );
        request.on("error", reject);
        if (body) {
          request.write(body);
        }
        request.end();
      });
    } catch (error) {
      if (!isRetryableTransportError(error) || attempt >= retryAttempts) {
        throw error;
      }
      await delay(retryDelayMs * attempt);
      attempt += 1;
    }
  }
}

async function executeWorkflow({ apiBaseUrl, apiKey, payload, retryAttempts, retryDelayMs }) {
  const response = await apiRequest({
    apiBaseUrl,
    apiKey,
    pathName: "/api/v1/ExecuteWorkflow",
    payload,
    retryAttempts,
    retryDelayMs,
  });
  const invocationId = String(response.invocation_id || "").trim();
  if (invocationId) {
    return response;
  }
  const actionStatuses = Array.isArray(response.actionStatuses) ? response.actionStatuses : [];
  if (actionStatuses.length === 0) {
    return response;
  }
  const failedStatus = actionStatuses.find((status) => Number(status?.status?.code || 0) !== 0);
  if (failedStatus) {
    const code = Number(failedStatus?.status?.code || 0);
    const message = String(failedStatus?.status?.message || "").trim() || "unknown error";
    const actionName = String(failedStatus?.actionName || "").trim() || "workflow action";
    throw new Error(`BuildBuddy ExecuteWorkflow ${actionName} failed (${code}): ${message}`);
  }
  const normalizedInvocationId = String(
    actionStatuses[0]?.invocationId || actionStatuses[0]?.invocation_id || "",
  ).trim();
  if (!normalizedInvocationId) {
    return response;
  }
  return {
    ...response,
    invocation_id: normalizedInvocationId,
  };
}

async function getInvocation({
  apiBaseUrl,
  apiKey,
  includeArtifacts = false,
  includeChildInvocations = false,
  includeMetadata = false,
  invocationId,
  commitSha = "",
}) {
  const invocations = await getInvocations({
    apiBaseUrl,
    apiKey,
    includeArtifacts,
    includeChildInvocations,
    includeMetadata,
    invocationId,
    commitSha,
  });
  return invocations[0] || null;
}

async function getInvocations({
  apiBaseUrl,
  apiKey,
  includeArtifacts = false,
  includeChildInvocations = false,
  includeMetadata = false,
  invocationId = "",
  commitSha = "",
  pageToken = "",
}) {
  const selector = {};
  if (String(invocationId || "").trim()) {
    selector.invocation_id = String(invocationId).trim();
  }
  if (String(commitSha || "").trim()) {
    selector.commit_sha = String(commitSha).trim();
  }
  if (Object.keys(selector).length === 0) {
    throw new Error("BuildBuddy invocation selector requires invocationId or commitSha");
  }
  const explicitPageToken = String(pageToken || "").trim();
  let nextPageToken = explicitPageToken;
  const invocations = [];
  while (true) {
    const response = await apiRequest({
      apiBaseUrl,
      apiKey,
      pathName: "/api/v1/GetInvocation",
      payload: {
        selector,
        include_artifacts: includeArtifacts,
        include_child_invocations: includeChildInvocations,
        include_metadata: includeMetadata,
        ...(nextPageToken ? { page_token: nextPageToken } : {}),
      },
    });
    if (Array.isArray(response.invocation) && response.invocation.length > 0) {
      invocations.push(...response.invocation);
    }
    const responseNextPageToken = String(response.nextPageToken || response.next_page_token || "").trim();
    if (explicitPageToken || !responseNextPageToken) {
      break;
    }
    nextPageToken = responseNextPageToken;
  }
  return invocations;
}

async function getInvocationLogPage({ apiBaseUrl, apiKey, invocationId, pageToken = "" }) {
  return apiRequest({
    apiBaseUrl,
    apiKey,
    pathName: "/api/v1/GetLog",
    payload: {
      selector: { invocation_id: invocationId },
      ...(pageToken ? { page_token: pageToken } : {}),
    },
  });
}

async function getFile({ apiBaseUrl, apiKey, uri }) {
  return apiRequest({
    apiBaseUrl,
    apiKey,
    expectBinary: true,
    pathName: "/api/v1/GetFile",
    payload: { uri },
  });
}

module.exports = {
  DEFAULT_BUILDBUDDY_API_BASE_URL,
  apiRequest,
  buildInvocationUrl,
  executeWorkflow,
  getFile,
  getInvocation,
  getInvocations,
  getInvocationLogPage,
};
