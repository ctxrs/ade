#!/usr/bin/env node
import { existsSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";

export function resolvePathOrNull(value) {
  return value ? resolve(value) : null;
}

export function readJson(path) {
  return JSON.parse(readFileSync(path, "utf8"));
}

export function sleep(ms) {
  return new Promise((resolveSleep) => setTimeout(resolveSleep, ms));
}

export async function waitFor(check, { timeoutMs = 20_000, intervalMs = 200, label = "condition" } = {}) {
  const startedAt = Date.now();
  while (Date.now() - startedAt <= timeoutMs) {
    const value = await check();
    if (value) {
      return value;
    }
    await sleep(intervalMs);
  }
  throw new Error(`Timed out waiting for ${label}`);
}

export async function waitForFile(path, options = {}) {
  return waitFor(() => (existsSync(path) ? path : null), {
    ...options,
    label: options.label || path,
  });
}

export async function waitForHttpOk(url, options = {}) {
  return waitFor(async () => {
    try {
      const resp = await fetch(url);
      return resp.ok ? true : null;
    } catch {
      return null;
    }
  }, {
    ...options,
    label: options.label || url,
  });
}

export function readDaemonAuth(options) {
  if (options.daemonUrl && options.authToken) {
    return { daemonUrl: options.daemonUrl, authToken: options.authToken };
  }
  if (!options.dataDir) {
    throw new Error("daemon auth not provided; use --daemon-url/--auth-token or --data-dir");
  }
  const authPath = join(options.dataDir, "daemon_auth.json");
  const auth = readJson(authPath);
  return {
    daemonUrl: options.daemonUrl || auth.daemon_url,
    authToken: options.authToken || auth.token,
  };
}

export async function api(baseUrl, token, method, path, body) {
  const resp = await fetch(`${baseUrl}${path}`, {
    method,
    headers: {
      authorization: `Bearer ${token}`,
      "content-type": "application/json",
    },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await resp.text();
  const parsed = text ? JSON.parse(text) : null;
  if (!resp.ok) {
    throw new Error(`${method} ${path} failed (${resp.status}): ${text}`);
  }
  return parsed;
}

export async function getProviderStatus(baseUrl, token, providerId, target = "host") {
  return api(baseUrl, token, "GET", `/api/providers/${encodeURIComponent(providerId)}?target=${encodeURIComponent(target)}`);
}

export async function installProviderAndWait(
  baseUrl,
  token,
  providerId,
  {
    target = "host",
    timeoutMs = 10 * 60_000,
    pollMs = 2_000,
  } = {},
) {
  const started = await api(
    baseUrl,
    token,
    "POST",
    `/api/providers/${encodeURIComponent(providerId)}/install?target=${encodeURIComponent(target)}`,
    {},
  );
  const installId = String(started.install_id || "").trim();
  if (!installId) {
    throw new Error(`provider install response missing install_id for ${providerId}`);
  }
  return waitFor(async () => {
    const poll = await api(baseUrl, token, "GET", `/api/providers/install/${encodeURIComponent(installId)}`);
    const state = String(poll.state || "").toLowerCase();
    if (state === "succeeded") {
      return poll;
    }
    if (state === "failed" || state === "cancelled") {
      const detail = JSON.stringify(poll.last_event || poll.error || poll);
      throw new Error(`provider install ${state} for ${providerId}: ${detail}`);
    }
    return null;
  }, {
    timeoutMs,
    intervalMs: pollMs,
    label: `${providerId} install`,
  });
}
