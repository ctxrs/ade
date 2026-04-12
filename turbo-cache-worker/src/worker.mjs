const HASH_PATTERN = /^[a-fA-F0-9]+$/;
const MAX_TAG_LENGTH = 600;

function json(value, init = {}) {
  const headers = new Headers(init.headers || {});
  if (!headers.has("content-type")) {
    headers.set("content-type", "application/json; charset=utf-8");
  }
  return new Response(`${JSON.stringify(value)}\n`, {
    ...init,
    headers,
  });
}

function errorResponse(status, code, message) {
  return json({ code, message }, { status });
}

function parseBearerToken(request) {
  const header = request.headers.get("authorization") || "";
  const match = header.match(/^Bearer\s+(.+)$/i);
  return match ? match[1].trim() : "";
}

function parseTokenSet(rawValue) {
  return new Set(
    String(rawValue || "")
      .split(",")
      .map((entry) => entry.trim())
      .filter((entry) => entry.length > 0),
  );
}

function requireAuth(request, env) {
  const expectedTokens = parseTokenSet(env.TURBO_TOKEN || env.TURBO_TOKENS);
  if (expectedTokens.size === 0) {
    return {
      ok: false,
      response: errorResponse(500, "cache_auth_unconfigured", "Remote cache token is not configured"),
    };
  }
  const provided = parseBearerToken(request);
  if (!provided) {
    return {
      ok: false,
      response: errorResponse(401, "cache_auth_missing", "Missing bearer token"),
    };
  }
  if (!expectedTokens.has(provided)) {
    return {
      ok: false,
      response: errorResponse(403, "cache_auth_invalid", "Invalid bearer token"),
    };
  }
  return { ok: true };
}

function sanitizeScopePart(value) {
  return String(value || "")
    .trim()
    .replace(/[^A-Za-z0-9._-]+/g, "-")
    .replace(/^-+|-+$/g, "");
}

function resolveScope(url, env) {
  const teamId = sanitizeScopePart(url.searchParams.get("teamId"));
  const slug = sanitizeScopePart(url.searchParams.get("slug"));
  const configuredTeam = sanitizeScopePart(env.TURBO_TEAM);
  const configuredTeamId = sanitizeScopePart(env.TURBO_TEAM_ID);

  if (configuredTeam && slug && slug !== configuredTeam) {
    return {
      ok: false,
      response: errorResponse(403, "cache_scope_mismatch", `Slug ${slug} is not allowed for this cache`),
    };
  }
  if (configuredTeamId && teamId && teamId !== configuredTeamId) {
    return {
      ok: false,
      response: errorResponse(403, "cache_scope_mismatch", `Team id ${teamId} is not allowed for this cache`),
    };
  }
  // When only TURBO_TEAM is configured (no TURBO_TEAM_ID), any caller-supplied teamId is
  // unverifiable and must be rejected to prevent scope bypass via the teamId path.
  if (configuredTeam && !configuredTeamId && teamId) {
    return {
      ok: false,
      response: errorResponse(
        403,
        "cache_scope_mismatch",
        "teamId selection is not permitted when only TURBO_TEAM is configured",
      ),
    };
  }
  // When only TURBO_TEAM_ID is configured (no TURBO_TEAM), any caller-supplied slug is
  // unverifiable and must be rejected to prevent scope bypass via the slug path.
  if (configuredTeamId && !configuredTeam && slug) {
    return {
      ok: false,
      response: errorResponse(
        403,
        "cache_scope_mismatch",
        "slug selection is not permitted when only TURBO_TEAM_ID is configured",
      ),
    };
  }

  const scope = slug || teamId || configuredTeam || configuredTeamId || "default";
  return { ok: true, scope };
}

function normalizeHash(hash) {
  const value = String(hash || "").trim();
  if (!HASH_PATTERN.test(value)) {
    return "";
  }
  return value.toLowerCase();
}

function buildArtifactKey(env, scope, hash) {
  const prefix = sanitizeScopePart(env.CACHE_OBJECT_PREFIX) || "turbo";
  return `${prefix}/${scope}/${hash}`;
}

function buildEventKey(env, scope) {
  const prefix = sanitizeScopePart(env.EVENTS_OBJECT_PREFIX) || "turbo-events";
  const now = new Date();
  const datePrefix = now.toISOString().slice(0, 10);
  return `${prefix}/${scope}/${datePrefix}/${now.toISOString()}-${crypto.randomUUID()}.json`;
}

function parseOptionalNonNegativeInteger(value, headerName) {
  if (value == null || value === "") {
    return { ok: true, value: undefined };
  }
  const parsed = Number.parseInt(String(value), 10);
  if (!Number.isFinite(parsed) || parsed < 0) {
    return {
      ok: false,
      response: errorResponse(400, "invalid_header", `${headerName} must be a non-negative integer`),
    };
  }
  return { ok: true, value: parsed };
}

function collectArtifactMetadata(request) {
  const contentLength = parseOptionalNonNegativeInteger(request.headers.get("content-length"), "Content-Length");
  if (!contentLength.ok || contentLength.value == null) {
    return contentLength.ok
      ? {
          ok: false,
          response: errorResponse(400, "missing_header", "Content-Length header is required"),
        }
      : contentLength;
  }

  const duration = parseOptionalNonNegativeInteger(
    request.headers.get("x-artifact-duration"),
    "x-artifact-duration",
  );
  if (!duration.ok) {
    return duration;
  }

  const tag = String(request.headers.get("x-artifact-tag") || "").trim();
  if (tag.length > MAX_TAG_LENGTH) {
    return {
      ok: false,
      response: errorResponse(400, "invalid_header", "x-artifact-tag exceeds max length"),
    };
  }

  const sha = String(request.headers.get("x-artifact-sha") || "").trim();
  const dirtyHash = String(request.headers.get("x-artifact-dirty-hash") || "").trim();
  const clientCi = String(request.headers.get("x-artifact-client-ci") || "").trim();
  const clientInteractive = String(request.headers.get("x-artifact-client-interactive") || "").trim();

  return {
    ok: true,
    metadata: {
      contentLength: String(contentLength.value),
      artifactDuration: duration.value == null ? "" : String(duration.value),
      artifactTag: tag,
      artifactSha: sha,
      artifactDirtyHash: dirtyHash,
      clientCi,
      clientInteractive,
      storedAt: new Date().toISOString(),
    },
  };
}

function applyArtifactHeaders(headers, object) {
  const custom = object.customMetadata || {};
  // Use the actual stored object size — not the client-supplied metadata — to prevent
  // a caller from advertising a content-length that differs from what was stored.
  headers.set("content-length", String(object.size));
  if (custom.artifactDuration) {
    headers.set("x-artifact-duration", custom.artifactDuration);
  }
  if (custom.artifactTag) {
    headers.set("x-artifact-tag", custom.artifactTag);
  }
  if (custom.artifactSha) {
    headers.set("x-artifact-sha", custom.artifactSha);
  }
  if (custom.artifactDirtyHash) {
    headers.set("x-artifact-dirty-hash", custom.artifactDirtyHash);
  }
}

async function handleStatus() {
  return json({ status: "enabled" });
}

async function handleHead(env, artifactKey) {
  const object = await env.CACHE_BUCKET.head(artifactKey);
  if (!object) {
    return errorResponse(404, "artifact_not_found", "Artifact not found");
  }
  const headers = new Headers();
  applyArtifactHeaders(headers, object);
  return new Response(null, { status: 200, headers });
}

async function handleGet(env, artifactKey) {
  const object = await env.CACHE_BUCKET.get(artifactKey);
  if (!object) {
    return errorResponse(404, "artifact_not_found", "Artifact not found");
  }
  const headers = new Headers({
    "content-type": "application/octet-stream",
  });
  applyArtifactHeaders(headers, object);
  return new Response(object.body, { status: 200, headers });
}

async function handlePut(request, env, artifactKey) {
  if (!request.body) {
    return errorResponse(400, "missing_body", "Artifact body is required");
  }
  const metadataResult = collectArtifactMetadata(request);
  if (!metadataResult.ok) {
    return metadataResult.response;
  }

  await env.CACHE_BUCKET.put(artifactKey, request.body, {
    customMetadata: metadataResult.metadata,
  });

  return json({ urls: [artifactKey] }, { status: 202 });
}

async function handleBatchQuery(request, env, scope) {
  let payload;
  try {
    payload = await request.json();
  } catch {
    return errorResponse(400, "invalid_json", "Request body must be valid JSON");
  }

  const hashes = Array.isArray(payload?.hashes) ? payload.hashes : null;
  if (!hashes) {
    return errorResponse(400, "invalid_request", "hashes must be an array");
  }

  const responsePayload = {};
  for (const rawHash of hashes) {
    const normalizedHash = normalizeHash(rawHash);
    if (!normalizedHash) {
      responsePayload[String(rawHash)] = {
        error: {
          message: "Invalid artifact hash",
        },
      };
      continue;
    }
    const object = await env.CACHE_BUCKET.head(buildArtifactKey(env, scope, normalizedHash));
    if (!object) {
      responsePayload[normalizedHash] = null;
      continue;
    }
    const custom = object.customMetadata || {};
    responsePayload[normalizedHash] = {
      // Use actual stored object size, not client-supplied metadata.
      size: object.size,
      taskDurationMs: Number(custom.artifactDuration || 0),
      ...(custom.artifactTag ? { tag: custom.artifactTag } : {}),
    };
  }

  return json(responsePayload);
}

async function handleEvents(request, env, scope) {
  let payload;
  try {
    payload = await request.json();
  } catch {
    return errorResponse(400, "invalid_json", "Request body must be valid JSON");
  }

  if (!Array.isArray(payload)) {
    return errorResponse(400, "invalid_request", "Events payload must be an array");
  }

  await env.CACHE_BUCKET.put(
    buildEventKey(env, scope),
    JSON.stringify({
      receivedAt: new Date().toISOString(),
      scope,
      events: payload,
    }),
    {
      customMetadata: {
        eventCount: String(payload.length),
      },
    },
  );

  return new Response(null, { status: 200 });
}

async function routeRequest(request, env) {
  const auth = requireAuth(request, env);
  if (!auth.ok) {
    return auth.response;
  }

  const url = new URL(request.url);
  const scopeResult = resolveScope(url, env);
  if (!scopeResult.ok) {
    return scopeResult.response;
  }
  const scope = scopeResult.scope;

  if (request.method === "GET" && url.pathname === "/artifacts/status") {
    return handleStatus();
  }
  if (request.method === "POST" && url.pathname === "/artifacts") {
    return handleBatchQuery(request, env, scope);
  }
  if (request.method === "POST" && url.pathname === "/artifacts/events") {
    return handleEvents(request, env, scope);
  }

  const artifactMatch = url.pathname.match(/^\/artifacts\/([A-Za-z0-9]+)$/);
  if (artifactMatch) {
    const hash = normalizeHash(artifactMatch[1]);
    if (!hash) {
      return errorResponse(400, "invalid_hash", "Artifact hash must be hexadecimal");
    }
    const artifactKey = buildArtifactKey(env, scope, hash);
    if (request.method === "HEAD") {
      return handleHead(env, artifactKey);
    }
    if (request.method === "GET") {
      return handleGet(env, artifactKey);
    }
    if (request.method === "PUT") {
      return handlePut(request, env, artifactKey);
    }
  }

  return errorResponse(404, "not_found", "Route not found");
}

const worker = {
  fetch(request, env) {
    return routeRequest(request, env);
  },
};

export {
  applyArtifactHeaders,
  buildArtifactKey,
  buildEventKey,
  handleBatchQuery,
  handleEvents,
  handleGet,
  handleHead,
  handlePut,
  handleStatus,
  normalizeHash,
  requireAuth,
  resolveScope,
  routeRequest,
};

export default worker;
