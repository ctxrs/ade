import { afterEach, describe, expect, test, vi } from "vitest";

import { base64UrlEncode, jwkThumbprint } from "../src/grant";
import worker from "../src/worker";
import type { Env, ExecutionContextLike, RelayDelegation, RunGrant } from "../src/types";

const authorityBaseUrl = "https://authority.test";
const providerUrl = "https://provider.test/v1/responses";

afterEach(() => {
  vi.restoreAllMocks();
});

describe("llm relay worker", () => {
  test("rejects unsupported request shape before authority call", async () => {
    const fetchMock = vi.spyOn(globalThis, "fetch");
    const response = await worker.fetch(
      buildRelayRequest({
        input: "hello",
        tools: [{ type: "web_search_preview" }],
        max_output_tokens: 32,
      }),
      testEnv(),
      testCtx(),
    );

    expect(response.status).toBe(400);
    expect(fetchMock).not.toHaveBeenCalled();
  });

  test("reservation denial blocks provider call", async () => {
    const calls: CapturedCall[] = [];
    vi.spyOn(globalThis, "fetch").mockImplementation(fakeFetch(calls, { reserveStatus: 402 }));

    const response = await worker.fetch(
      buildRelayRequest({ input: "hello", max_output_tokens: 32 }),
      testEnv(),
      testCtx(),
    );

    expect(response.status).toBe(402);
    expect(calls.map((call) => call.url)).toEqual([`${authorityBaseUrl}/v1/relay/reservations`]);
  });

  test("streams provider bytes and finalizes metadata-only", async () => {
    const calls: CapturedCall[] = [];
    vi.spyOn(globalThis, "fetch").mockImplementation(fakeFetch(calls));

    const response = await worker.fetch(
      buildRelayRequest({ input: "do not log this prompt", max_output_tokens: 32 }),
      testEnv(),
      testCtx(),
    );

    expect(response.status).toBe(200);
    expect(await response.text()).toContain("response.completed");
    expect(calls.map((call) => call.url)).toEqual([
      `${authorityBaseUrl}/v1/relay/reservations`,
      `${authorityBaseUrl}/v1/relay/provider-started`,
      providerUrl,
      `${authorityBaseUrl}/v1/relay/provider-started`,
      `${authorityBaseUrl}/v1/relay/finalize`,
    ]);
    for (const call of calls.filter((entry) => entry.url.startsWith(authorityBaseUrl))) {
      expect(JSON.stringify(call.body)).not.toContain("do not log this prompt");
      expect(JSON.stringify(call.body)).not.toContain("hello from model");
    }
    const finalize = calls.find((call) => call.url.endsWith("/v1/relay/finalize"));
    expect(finalize?.body).toMatchObject({
      usage_unknown: false,
      actual_input_tokens: 12,
      actual_output_tokens: 7,
    });
    const provider = calls.find((call) => call.url === providerUrl);
    expect(provider?.body).toMatchObject({ stream: true, store: false });
  });

  test("forces streaming and provider storage off", async () => {
    const calls: CapturedCall[] = [];
    vi.spyOn(globalThis, "fetch").mockImplementation(fakeFetch(calls));

    const response = await worker.fetch(
      buildRelayRequest({ input: "hello", stream: false, store: false, max_output_tokens: 32 }),
      testEnv(),
      testCtx(),
    );

    expect(response.status).toBe(200);
    const provider = calls.find((call) => call.url === providerUrl);
    expect(provider?.body).toMatchObject({ stream: true, store: false });
  });

  test("accepts signed delegation and daemon grant chain", async () => {
    const calls: CapturedCall[] = [];
    vi.spyOn(globalThis, "fetch").mockImplementation(fakeFetch(calls));
    const signed = await buildSignedRelayRequest({ input: "hello", max_output_tokens: 32 });

    const response = await worker.fetch(
      signed.request,
      {
        ...testEnv(),
        GRANT_VERIFICATION_MODE: "signed_chain",
        CONTROL_PLANE_JWKS: JSON.stringify({ keys: [signed.controlPublicJwk] }),
      },
      testCtx(),
    );

    expect(response.status).toBe(200);
    expect(calls.map((call) => call.url)).toContain(providerUrl);
    const reservation = calls.find((call) => call.url.endsWith("/v1/relay/reservations"));
    expect(reservation?.body).toMatchObject({
      delegation_jws: expect.any(String),
      grant_jws: expect.any(String),
    });
  });

  test("rejects hmac grants in prod", async () => {
    const fetchMock = vi.spyOn(globalThis, "fetch");
    const response = await worker.fetch(
      buildRelayRequest({ input: "hello", max_output_tokens: 32 }),
      {
        ...testEnv(),
        ENVIRONMENT: "prod",
        GRANT_VERIFICATION_MODE: "hmac",
        GRANT_HMAC_SECRET: "dev-secret",
      },
      testCtx(),
    );

    expect(response.status).toBe(500);
    expect(fetchMock).not.toHaveBeenCalled();
  });

  test("rejects unsigned grants for production environment aliases", async () => {
    const fetchMock = vi.spyOn(globalThis, "fetch");
    const response = await worker.fetch(
      buildRelayRequest({ input: "hello", max_output_tokens: 32 }),
      {
        ...testEnv(),
        ENVIRONMENT: "production",
        GRANT_VERIFICATION_MODE: "test_unsigned",
      },
      testCtx(),
    );

    expect(response.status).toBe(500);
    expect(fetchMock).not.toHaveBeenCalled();
  });

  test("requires authority bearer token outside dev and test", async () => {
    const fetchMock = vi.spyOn(globalThis, "fetch");
    const response = await worker.fetch(
      buildRelayRequest({ input: "hello", max_output_tokens: 32 }),
      { ...testEnv(), ENVIRONMENT: "staging" },
      testCtx(),
    );

    expect(response.status).toBe(500);
    expect(fetchMock).not.toHaveBeenCalled();
  });

  test("provider stream break finalizes as unknown usage", async () => {
    const calls: CapturedCall[] = [];
    vi.spyOn(globalThis, "fetch").mockImplementation(fakeFetch(calls, { breakStream: true }));

    const response = await worker.fetch(
      buildRelayRequest({ input: "hello", max_output_tokens: 32 }),
      testEnv(),
      testCtx(),
    );

    await expect(response.text()).rejects.toThrow();
    const finalize = calls.find((call) => call.url.endsWith("/v1/relay/finalize"));
    expect(finalize?.body).toMatchObject({ usage_unknown: true });
  });

  test("completed stream without usage metadata finalizes as unknown usage", async () => {
    const calls: CapturedCall[] = [];
    vi.spyOn(globalThis, "fetch").mockImplementation(fakeFetch(calls, { omitUsage: true }));

    const response = await worker.fetch(
      buildRelayRequest({ input: "hello", max_output_tokens: 32 }),
      testEnv(),
      testCtx(),
    );

    expect(response.status).toBe(200);
    await response.text();
    const finalize = calls.find((call) => call.url.endsWith("/v1/relay/finalize"));
    expect(finalize?.body).toMatchObject({ usage_unknown: true });
  });

  test("crlf-delimited stream extracts usage metadata", async () => {
    const calls: CapturedCall[] = [];
    vi.spyOn(globalThis, "fetch").mockImplementation(fakeFetch(calls, { crlfFrames: true }));

    const response = await worker.fetch(
      buildRelayRequest({ input: "hello", max_output_tokens: 32 }),
      testEnv(),
      testCtx(),
    );

    expect(response.status).toBe(200);
    await response.text();
    const finalize = calls.find((call) => call.url.endsWith("/v1/relay/finalize"));
    expect(finalize?.body).toMatchObject({
      usage_unknown: false,
      actual_input_tokens: 12,
      actual_output_tokens: 7,
    });
  });

  test("provider network failure consumes reservation for reconciliation", async () => {
    const calls: CapturedCall[] = [];
    vi.spyOn(globalThis, "fetch").mockImplementation(fakeFetch(calls, { providerThrows: true }));

    const response = await worker.fetch(
      buildRelayRequest({ input: "hello", max_output_tokens: 32 }),
      testEnv(),
      testCtx(),
    );

    expect(response.status).toBe(502);
    expect(calls.map((call) => call.url)).toEqual([
      `${authorityBaseUrl}/v1/relay/reservations`,
      `${authorityBaseUrl}/v1/relay/provider-started`,
      providerUrl,
      `${authorityBaseUrl}/v1/relay/finalize`,
    ]);
    const finalize = calls.find((call) => call.url.endsWith("/v1/relay/finalize"));
    expect(finalize?.body).toMatchObject({ usage_unknown: true });
  });
});

interface CapturedCall {
  url: string;
  body: unknown;
}

function testEnv(): Env {
  return {
    ENVIRONMENT: "test",
    GRANT_VERIFICATION_MODE: "test_unsigned",
    AUTHORITY_BASE_URL: authorityBaseUrl,
    OPENAI_RESPONSES_URL: providerUrl,
    OPENAI_API_KEY: "provider-key",
  };
}

function testCtx(): ExecutionContextLike {
  return {
    waitUntil(promise: Promise<unknown>) {
      void promise;
    },
  };
}

function buildRelayRequest(body: unknown): Request {
  const delegation = baseDelegation();
  const grant = baseGrant();
  return new Request("https://relay.test/v1/responses", {
    method: "POST",
    headers: {
      "content-type": "application/json",
      "x-ctx-relay-delegation": encodeJson(delegation),
      "x-ctx-relay-grant": encodeJson(grant),
    },
    body: JSON.stringify({ model: "gpt-5", stream: true, ...asRecord(body) }),
  });
}

async function buildSignedRelayRequest(body: unknown): Promise<{
  request: Request;
  controlPublicJwk: JsonWebKey;
}> {
  const controlKey = await generateEs256KeyPair("control_key_1");
  const daemonKey = await generateEs256KeyPair("daemon_key_1");
  const delegation = baseDelegation();
  delegation.daemon_public_key_jwk = daemonKey.publicJwk;
  delegation.daemon_public_key_thumbprint = await jwkThumbprint(daemonKey.publicJwk);
  const grant = baseGrant();
  return {
    request: new Request("https://relay.test/v1/responses", {
      method: "POST",
      headers: {
        "content-type": "application/json",
        "x-ctx-relay-delegation-jws": await signEs256Jws(delegation, controlKey.privateKey, "control_key_1"),
        "x-ctx-relay-grant-jws": await signEs256Jws(grant, daemonKey.privateKey, "daemon_key_1"),
      },
      body: JSON.stringify({ model: "gpt-5", stream: true, ...asRecord(body) }),
    }),
    controlPublicJwk: controlKey.publicJwk,
  };
}

interface Es256KeyPair {
  privateKey: CryptoKey;
  publicJwk: JsonWebKey;
}

type TestPublicJwk = JsonWebKey & { kid?: string };

async function generateEs256KeyPair(kid: string): Promise<Es256KeyPair> {
  const pair = (await crypto.subtle.generateKey(
    { name: "ECDSA", namedCurve: "P-256" },
    true,
    ["sign", "verify"],
  )) as CryptoKeyPair;
  const publicJwk = (await crypto.subtle.exportKey("jwk", pair.publicKey)) as TestPublicJwk;
  publicJwk.kid = kid;
  publicJwk.alg = "ES256";
  publicJwk.use = "sig";
  return { privateKey: pair.privateKey, publicJwk };
}

async function signEs256Jws(payload: unknown, privateKey: CryptoKey, kid: string): Promise<string> {
  const header = encodeJson({ alg: "ES256", typ: "JWT", kid });
  const body = encodeJson(payload);
  const signingInput = `${header}.${body}`;
  const signature = await crypto.subtle.sign(
    { name: "ECDSA", hash: "SHA-256" },
    privateKey,
    new TextEncoder().encode(signingInput),
  );
  return `${signingInput}.${base64UrlEncode(new Uint8Array(signature))}`;
}

function encodeJson(value: unknown): string {
  return base64UrlEncode(new TextEncoder().encode(JSON.stringify(value)));
}

function asRecord(value: unknown): Record<string, unknown> {
  if (typeof value === "object" && value !== null && !Array.isArray(value)) {
    return value as Record<string, unknown>;
  }
  return {};
}

function baseDelegation(): RelayDelegation {
  const issuedAt = new Date(Date.now() - 1_000).toISOString();
  const expiresAt = new Date(Date.now() + 60_000).toISOString();
  return {
    jti: "delegation_1",
    access_context_kind: "org",
    billing_subject_id: "bill_org_1",
    ctx_user_id: "user_1",
    ctx_account_id: "account_1",
    ctx_org_id: "org_1",
    ctx_membership_id: "membership_1",
    daemon_id: "daemon_1",
    daemon_public_key_thumbprint: "thumb_1",
    allowed_route_ids: ["route_ctx"],
    allowed_provider_model_pairs: [{ provider_id: "openai", model_id: "gpt-5" }],
    allowed_auth_methods: ["ctx_provider_key"],
    policy_version: "policy_v1",
    pricing_version: "pricing_v1",
    max_per_request_cents: 100,
    max_input_tokens: 8_000,
    max_output_tokens: 1_000,
    issued_at: issuedAt,
    expires_at: expiresAt,
    issuer: "ctx-control-plane",
    audience: "ctx-llm-relay",
  };
}

function baseGrant(): RunGrant {
  const issuedAt = new Date(Date.now() - 500).toISOString();
  const expiresAt = new Date(Date.now() + 30_000).toISOString();
  return {
    jti: "grant_1",
    request_id: "request_1",
    access_context_kind: "org",
    billing_subject_id: "bill_org_1",
    ctx_user_id: "user_1",
    ctx_account_id: "account_1",
    ctx_org_id: "org_1",
    ctx_membership_id: "membership_1",
    daemon_id: "daemon_1",
    route_id: "route_ctx",
    route_type: "ctx_managed",
    provider_id: "openai",
    model_id: "gpt-5",
    policy_version: "policy_v1",
    pricing_version: "pricing_v1",
    max_estimated_cents: 25,
    max_input_tokens: 8_000,
    max_output_tokens: 1_000,
    delegation_jti: "delegation_1",
    delegation_hash: "hash",
    issued_at: issuedAt,
    expires_at: expiresAt,
    issuer: "daemon_1",
    audience: "ctx-llm-relay",
  };
}

function fakeFetch(
  calls: CapturedCall[],
  options: {
    reserveStatus?: number;
    breakStream?: boolean;
    providerThrows?: boolean;
    omitUsage?: boolean;
    crlfFrames?: boolean;
  } = {},
): typeof fetch {
  return (async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const url = String(input);
    const body = init?.body == null ? undefined : JSON.parse(String(init.body));
    calls.push({ url, body });
    if (url.endsWith("/v1/relay/reservations")) {
      return Response.json(
        {
          request_id: "request_1",
          reservation_id: "reservation_1",
          state: "reserved",
          reserved_cents: 25,
          idempotent: false,
        },
        { status: options.reserveStatus ?? 200 },
      );
    }
    if (url.endsWith("/v1/relay/provider-started") || url.endsWith("/v1/relay/finalize") || url.endsWith("/v1/relay/void")) {
      return Response.json({ ok: true });
    }
    if (url === providerUrl) {
      if (options.providerThrows === true) {
        throw new Error("provider offline");
      }
      return new Response(
        options.breakStream
          ? brokenStream()
          : providerStream({ omitUsage: options.omitUsage, crlfFrames: options.crlfFrames }),
        {
        status: 200,
        headers: {
          "content-type": "text/event-stream",
          "x-request-id": "provider_req_1",
        },
        },
      );
    }
    return Response.json({ error: "unexpected fetch" }, { status: 500 });
  }) as typeof fetch;
}

function providerStream(options: { omitUsage?: boolean; crlfFrames?: boolean } = {}): ReadableStream<Uint8Array> {
  const encoder = new TextEncoder();
  return new ReadableStream<Uint8Array>({
    start(controller) {
      const newline = options.crlfFrames === true ? "\r\n" : "\n";
      const boundary = `${newline}${newline}`;
      controller.enqueue(
        encoder.encode(`event: response.output_text.delta${newline}data: {"delta":"hello from model"}${boundary}`),
      );
      if (options.omitUsage !== true) {
        controller.enqueue(
          encoder.encode(
            `event: response.completed${newline}data: {"response":{"usage":{"input_tokens":12,"output_tokens":7}}}${boundary}`,
          ),
        );
      }
      controller.close();
    },
  });
}

function brokenStream(): ReadableStream<Uint8Array> {
  const encoder = new TextEncoder();
  return new ReadableStream<Uint8Array>({
    start(controller) {
      controller.enqueue(encoder.encode("event: response.output_text.delta\ndata: {}\n\n"));
      controller.error(new Error("stream broke"));
    },
  });
}
