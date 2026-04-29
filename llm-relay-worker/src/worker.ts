import { verifyGrantEnvelope } from "./grant";
import { jsonError, RelayHttpError, requireString } from "./http";
import { validateResponsesRequest } from "./request-shape";
import type {
  AuthorityReserveResponse,
  Env,
  ExecutionContextLike,
  RelayDelegation,
  RunGrant,
  ValidatedResponsesRequest,
} from "./types";

export default {
  async fetch(request: Request, env: Env, ctx: ExecutionContextLike): Promise<Response> {
    try {
      return await handleFetch(request, env, ctx);
    } catch (error) {
      return jsonError(error);
    }
  },
};

async function handleFetch(request: Request, env: Env, ctx: ExecutionContextLike): Promise<Response> {
  const url = new URL(request.url);
  if (request.method !== "POST" || url.pathname !== "/v1/responses") {
    throw new RelayHttpError(404, "not_found", "route not found");
  }

  const { delegation, grant, delegationJws, grantJws } = await verifyGrantEnvelope(request, env);
  const rawBody = await request.json();
  const validated = validateResponsesRequest(rawBody);
  ensureGrantCoversRequest(grant, validated);
  const providerBody = providerRequestBody(rawBody);

  const authority = new AuthorityClient(env);
  await authority.reserve(delegation, grant, validated, delegationJws, grantJws);
  await authority.providerStarted(grant.request_id, undefined);
  let providerResponse: Response;
  try {
    providerResponse = await callProvider(env, providerBody);
  } catch {
    ctx.waitUntil(
      authority
        .finalize(grant.request_id, {
          billable_cents: grant.max_estimated_cents,
          usage_unknown: true,
        })
        .catch(logSettlementFailure),
    );
    throw new RelayHttpError(502, "provider_unavailable", "provider request failed");
  }

  if (!providerResponse.ok || providerResponse.body == null) {
    ctx.waitUntil(
      authority
        .finalize(grant.request_id, {
          billable_cents: grant.max_estimated_cents,
          usage_unknown: true,
        })
        .catch(logSettlementFailure),
    );
    return new Response(providerResponse.body, {
      status: providerResponse.status,
      headers: safeProviderHeaders(providerResponse.headers),
    });
  }

  const providerRequestId =
    providerResponse.headers.get("x-request-id") ??
    providerResponse.headers.get("openai-request-id") ??
    undefined;
  ctx.waitUntil(authority.providerStarted(grant.request_id, providerRequestId).catch(logSettlementFailure));
  const stream = streamWithSettlement(providerResponse.body, async (settlement) => {
    await authority.finalize(grant.request_id, {
      billable_cents: grant.max_estimated_cents,
      usage_unknown: settlement.usageUnknown,
      actual_input_tokens: settlement.actualInputTokens,
      actual_output_tokens: settlement.actualOutputTokens,
      provider_request_id: providerRequestId,
    });
  });
  return new Response(stream, {
    status: providerResponse.status,
    headers: safeProviderHeaders(providerResponse.headers),
  });
}

function ensureGrantCoversRequest(grant: RunGrant, request: ValidatedResponsesRequest): void {
  if (grant.model_id !== request.model) {
    throw new RelayHttpError(403, "model_mismatch", "request model does not match grant");
  }
  if (grant.max_output_tokens != null && request.maxOutputTokens > grant.max_output_tokens) {
    throw new RelayHttpError(403, "grant_limit_exceeded", "request exceeds grant output token limit");
  }
  if (grant.max_input_tokens != null) {
    const estimatedTokens = Math.ceil(request.estimatedInputBytes / 4);
    if (estimatedTokens > grant.max_input_tokens) {
      throw new RelayHttpError(403, "grant_limit_exceeded", "request exceeds grant input token limit");
    }
  }
}

class AuthorityClient {
  private readonly baseUrl: string;
  private readonly bearerToken?: string;

  constructor(env: Env) {
    this.baseUrl = requireString(env.AUTHORITY_BASE_URL, "AUTHORITY_BASE_URL").replace(/\/$/, "");
    this.bearerToken = env.AUTHORITY_BEARER_TOKEN?.trim() || undefined;
    const environment = env.ENVIRONMENT ?? "dev";
    if (environment !== "dev" && environment !== "test" && this.bearerToken == null) {
      throw new RelayHttpError(
        500,
        "invalid_config",
        "AUTHORITY_BEARER_TOKEN is required outside dev/test",
      );
    }
  }

  async reserve(
    delegation: RelayDelegation,
    grant: RunGrant,
    request: ValidatedResponsesRequest,
    delegationJws: string | undefined,
    grantJws: string | undefined,
  ): Promise<AuthorityReserveResponse> {
    return this.post<AuthorityReserveResponse>("/v1/relay/reservations", {
      delegation,
      grant,
      delegation_jws: delegationJws,
      grant_jws: grantJws,
      estimated_input_tokens: Math.ceil(request.estimatedInputBytes / 4),
      estimated_output_tokens: request.maxOutputTokens,
    });
  }

  async providerStarted(requestId: string, providerRequestId: string | undefined): Promise<void> {
    await this.post("/v1/relay/provider-started", {
      request_id: requestId,
      provider_request_id: providerRequestId,
    });
  }

  async finalize(
    requestId: string,
    body: {
      billable_cents: number;
      usage_unknown: boolean;
      actual_input_tokens?: number;
      actual_output_tokens?: number;
      provider_request_id?: string;
    },
  ): Promise<void> {
    await this.post("/v1/relay/finalize", {
      request_id: requestId,
      ...body,
    });
  }

  async void(requestId: string, reason: string): Promise<void> {
    await this.post("/v1/relay/void", {
      request_id: requestId,
      reason,
    });
  }

  private async post<T = unknown>(path: string, body: unknown): Promise<T> {
    const headers = new Headers({ "content-type": "application/json" });
    if (this.bearerToken != null) {
      headers.set("authorization", `Bearer ${this.bearerToken}`);
    }
    const response = await fetch(`${this.baseUrl}${path}`, {
      method: "POST",
      headers,
      body: JSON.stringify(body),
    });
    if (!response.ok) {
      const text = await response.text();
      throw new RelayHttpError(response.status, "authority_rejected", text || "authority rejected request");
    }
    return (await response.json()) as T;
  }
}

async function callProvider(env: Env, body: unknown): Promise<Response> {
  const providerUrl = requireString(env.OPENAI_RESPONSES_URL, "OPENAI_RESPONSES_URL");
  const providerKey = requireString(env.OPENAI_API_KEY, "OPENAI_API_KEY");
  return fetch(providerUrl, {
    method: "POST",
    headers: {
      authorization: `Bearer ${providerKey}`,
      "content-type": "application/json",
    },
    body: JSON.stringify(body),
  });
}

function providerRequestBody(body: unknown): Record<string, unknown> {
  if (!isRecord(body)) {
    throw new RelayHttpError(500, "invalid_state", "validated provider request body must be an object");
  }
  return { ...body, stream: true, store: false };
}

function streamWithSettlement(
  body: ReadableStream<Uint8Array>,
  settle: (settlement: StreamSettlement) => Promise<void>,
): ReadableStream<Uint8Array> {
  return new ReadableStream<Uint8Array>({
    async start(controller) {
      const reader = body.getReader();
      const usageTracker = new OpenAiResponsesUsageTracker();
      let streamCompleted = false;
      try {
        while (true) {
          const { done, value } = await reader.read();
          if (done) {
            streamCompleted = true;
            break;
          }
          usageTracker.ingest(value);
          controller.enqueue(value);
        }
        controller.close();
      } catch (error) {
        controller.error(error);
      } finally {
        const usage = usageTracker.finish();
        await settle({
          usageUnknown: !streamCompleted || usage == null,
          actualInputTokens: usage?.inputTokens,
          actualOutputTokens: usage?.outputTokens,
        }).catch(logSettlementFailure);
      }
    },
  });
}

interface StreamSettlement {
  usageUnknown: boolean;
  actualInputTokens?: number;
  actualOutputTokens?: number;
}

interface ResponseUsage {
  inputTokens: number;
  outputTokens: number;
}

class OpenAiResponsesUsageTracker {
  private readonly decoder = new TextDecoder();
  private buffer = "";
  private latestUsage?: ResponseUsage;

  ingest(chunk: Uint8Array): void {
    this.buffer += this.decoder.decode(chunk, { stream: true });
    this.consumeFrames(false);
  }

  finish(): ResponseUsage | undefined {
    this.buffer += this.decoder.decode();
    this.consumeFrames(true);
    return this.latestUsage;
  }

  private consumeFrames(flush: boolean): void {
    while (true) {
      const boundary = findSseFrameBoundary(this.buffer);
      if (boundary == null) {
        break;
      }
      const frame = this.buffer.slice(0, boundary.index);
      this.buffer = this.buffer.slice(boundary.index + boundary.length);
      this.parseFrame(frame);
    }
    if (flush && this.buffer.trim() !== "") {
      this.parseFrame(this.buffer);
      this.buffer = "";
    }
  }

  private parseFrame(frame: string): void {
    let eventName = "";
    const dataLines: string[] = [];
    for (const line of frame.split(/\r?\n/u)) {
      if (line.startsWith("event:")) {
        eventName = line.slice("event:".length).trim();
      } else if (line.startsWith("data:")) {
        dataLines.push(line.slice("data:".length).trimStart());
      }
    }
    if (eventName !== "response.completed" || dataLines.length === 0) {
      return;
    }
    try {
      const parsed = JSON.parse(dataLines.join("\n")) as unknown;
      const usage = extractOpenAiUsage(parsed);
      if (usage != null) {
        this.latestUsage = usage;
      }
    } catch {
      return;
    }
  }
}

function findSseFrameBoundary(buffer: string): { index: number; length: number } | undefined {
  const lf = buffer.indexOf("\n\n");
  const crlf = buffer.indexOf("\r\n\r\n");
  if (lf < 0 && crlf < 0) {
    return undefined;
  }
  if (lf >= 0 && (crlf < 0 || lf < crlf)) {
    return { index: lf, length: 2 };
  }
  return { index: crlf, length: 4 };
}

function extractOpenAiUsage(value: unknown): ResponseUsage | undefined {
  if (!isRecord(value)) {
    return undefined;
  }
  const response = isRecord(value.response) ? value.response : value;
  if (!isRecord(response.usage)) {
    return undefined;
  }
  const inputTokens = readNonNegativeInteger(response.usage.input_tokens);
  const outputTokens = readNonNegativeInteger(response.usage.output_tokens);
  if (inputTokens == null || outputTokens == null) {
    return undefined;
  }
  return { inputTokens, outputTokens };
}

function readNonNegativeInteger(value: unknown): number | undefined {
  if (typeof value !== "number" || !Number.isInteger(value) || value < 0) {
    return undefined;
  }
  return value;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function logSettlementFailure(error: unknown): void {
  console.error("relay settlement failed", error);
}

function safeProviderHeaders(headers: Headers): Headers {
  const output = new Headers();
  const contentType = headers.get("content-type");
  if (contentType != null) {
    output.set("content-type", contentType);
  }
  const requestId = headers.get("x-request-id") ?? headers.get("openai-request-id");
  if (requestId != null) {
    output.set("x-provider-request-id", requestId);
  }
  return output;
}
