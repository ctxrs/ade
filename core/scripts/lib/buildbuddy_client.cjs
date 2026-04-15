const http = require("node:http");
const https = require("node:https");

const DEFAULT_BUILDBUDDY_API_BASE_URL = "https://app.buildbuddy.io";

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

function apiRequest({
  apiBaseUrl = DEFAULT_BUILDBUDDY_API_BASE_URL,
  apiKey,
  expectBinary = false,
  pathName,
  payload,
}) {
  if (!String(apiKey || "").trim()) {
    throw new Error("BuildBuddy API key is required");
  }
  const body = payload === undefined ? "" : JSON.stringify(payload);
  const url = new URL(pathName, apiBaseUrl);
  const transport = url.protocol === "http:" ? http : https;
  return new Promise((resolve, reject) => {
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
            const detail = expectBinary ? buffer.toString("utf8") : buffer.toString("utf8");
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
}

async function executeWorkflow({ apiBaseUrl, apiKey, payload }) {
  return apiRequest({
    apiBaseUrl,
    apiKey,
    pathName: "/api/v1/ExecuteWorkflow",
    payload,
  });
}

async function getInvocation({
  apiBaseUrl,
  apiKey,
  includeArtifacts = false,
  includeChildInvocations = false,
  includeMetadata = false,
  invocationId,
}) {
  const response = await apiRequest({
    apiBaseUrl,
    apiKey,
    pathName: "/api/v1/GetInvocation",
    payload: {
      selector: { invocation_id: invocationId },
      include_artifacts: includeArtifacts,
      include_child_invocations: includeChildInvocations,
      include_metadata: includeMetadata,
    },
  });
  return Array.isArray(response.invocation) ? response.invocation[0] || null : null;
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
  getInvocationLogPage,
};
