import test from "node:test";
import assert from "node:assert/strict";

import worker from "./worker.mjs";

async function readBody(body) {
  if (body == null) {
    return new Uint8Array();
  }
  if (body instanceof Uint8Array) {
    return body;
  }
  if (body instanceof ArrayBuffer) {
    return new Uint8Array(body);
  }
  if (typeof body.getReader === "function") {
    const reader = body.getReader();
    const chunks = [];
    let total = 0;
    while (true) {
      const { done, value } = await reader.read();
      if (done) {
        break;
      }
      chunks.push(value);
      total += value.length;
    }
    const output = new Uint8Array(total);
    let offset = 0;
    for (const chunk of chunks) {
      output.set(chunk, offset);
      offset += chunk.length;
    }
    return output;
  }
  return new TextEncoder().encode(String(body));
}

class FakeObject {
  constructor(body, customMetadata) {
    this.body = body;
    this.customMetadata = customMetadata;
    this.size = body.length;
  }
}

class FakeBucket {
  constructor() {
    this.objects = new Map();
  }

  async put(key, body, options = {}) {
    const bytes = await readBody(body);
    this.objects.set(key, new FakeObject(bytes, { ...(options.customMetadata || {}) }));
  }

  async head(key) {
    return this.objects.get(key) || null;
  }

  async get(key) {
    return this.objects.get(key) || null;
  }
}

function buildEnv(overrides = {}) {
  return {
    CACHE_BUCKET: new FakeBucket(),
    TURBO_TOKEN: "test-token",
    TURBO_TEAM: "ctx-sdlc",
    CACHE_OBJECT_PREFIX: "turbo",
    EVENTS_OBJECT_PREFIX: "turbo-events",
    ...overrides,
  };
}

function buildRequest(url, init = {}) {
  const headers = new Headers(init.headers || {});
  if (!headers.has("authorization")) {
    headers.set("authorization", "Bearer test-token");
  }
  if (!url.includes("slug=")) {
    url += url.includes("?") ? "&slug=ctx-sdlc" : "?slug=ctx-sdlc";
  }
  return new Request(url, { ...init, headers });
}

test("status endpoint returns enabled", async () => {
  const response = await worker.fetch(buildRequest("https://cache.example/artifacts/status"), buildEnv());
  assert.equal(response.status, 200);
  assert.deepEqual(await response.json(), { status: "enabled" });
});

test("put + head + get roundtrip works", async () => {
  const env = buildEnv();
  const body = new TextEncoder().encode("artifact-bytes");
  const putResponse = await worker.fetch(
    buildRequest("https://cache.example/artifacts/abc123", {
      method: "PUT",
      body,
      headers: {
        "content-length": String(body.length),
        "x-artifact-duration": "42",
        "x-artifact-tag": "tag-1",
        "x-artifact-sha": "sha-1",
        "x-artifact-dirty-hash": "dirty-1",
      },
    }),
    env,
  );
  assert.equal(putResponse.status, 202);

  const headResponse = await worker.fetch(
    buildRequest("https://cache.example/artifacts/abc123", { method: "HEAD" }),
    env,
  );
  assert.equal(headResponse.status, 200);
  assert.equal(headResponse.headers.get("content-length"), String(body.length));
  assert.equal(headResponse.headers.get("x-artifact-duration"), "42");
  assert.equal(headResponse.headers.get("x-artifact-sha"), "sha-1");

  const getResponse = await worker.fetch(buildRequest("https://cache.example/artifacts/abc123"), env);
  assert.equal(getResponse.status, 200);
  assert.equal(getResponse.headers.get("x-artifact-tag"), "tag-1");
  assert.deepEqual(new Uint8Array(await getResponse.arrayBuffer()), body);
});

test("batch query returns info for hits and null for misses", async () => {
  const env = buildEnv();
  const body = new TextEncoder().encode("artifact-bytes");
  await worker.fetch(
    buildRequest("https://cache.example/artifacts/abc123", {
      method: "PUT",
      body,
      headers: {
        "content-length": String(body.length),
        "x-artifact-duration": "55",
        "x-artifact-tag": "tag-2",
      },
    }),
    env,
  );

  const response = await worker.fetch(
    buildRequest("https://cache.example/artifacts", {
      method: "POST",
      body: JSON.stringify({ hashes: ["abc123", "def456"] }),
      headers: {
        "content-type": "application/json",
      },
    }),
    env,
  );
  assert.equal(response.status, 200);
  assert.deepEqual(await response.json(), {
    abc123: {
      size: body.length,
      taskDurationMs: 55,
      tag: "tag-2",
    },
    def456: null,
  });
});

test("events endpoint stores a batch object", async () => {
  const env = buildEnv();
  const response = await worker.fetch(
    buildRequest("https://cache.example/artifacts/events", {
      method: "POST",
      body: JSON.stringify([
        {
          sessionId: "5f5238fe-04d3-4787-97db-86443ea45d79",
          source: "REMOTE",
          event: "HIT",
          hash: "abc123",
          duration: 12,
        },
      ]),
      headers: {
        "content-type": "application/json",
      },
    }),
    env,
  );
  assert.equal(response.status, 200);
  const keys = [...env.CACHE_BUCKET.objects.keys()];
  assert.equal(keys.length, 1);
  assert.match(keys[0], /^turbo-events\/ctx-sdlc\//);
});

test("auth is required", async () => {
  const response = await worker.fetch(
    new Request("https://cache.example/artifacts/status?slug=ctx-sdlc"),
    buildEnv(),
  );
  assert.equal(response.status, 401);
});

test("scope mismatch is rejected", async () => {
  const response = await worker.fetch(
    buildRequest("https://cache.example/artifacts/status?slug=other-team"),
    buildEnv(),
  );
  assert.equal(response.status, 403);
});
