import { randomUUID } from "node:crypto";
import {
  Agent,
  AgentSideConnection,
  AuthenticateRequest,
  CancelNotification,
  InitializeRequest,
  InitializeResponse,
  NewSessionRequest,
  NewSessionResponse,
  PromptRequest,
  PromptResponse,
} from "@agentclientprotocol/sdk";
import packageJson from "../package.json" with { type: "json" };
import { contentBlocksToText } from "./mapping.js";
import { requestAssistantCompletion } from "./openai.js";

type OpenHandsSession = {
  sessionId: string;
  cancelRequested: boolean;
  inFlight: AbortController | null;
};

const firstText = (...values: unknown[]): string | null => {
  for (const value of values) {
    if (typeof value !== "string") continue;
    const trimmed = value.trim();
    if (trimmed.length > 0) return trimmed;
  }
  return null;
};

function extractModelHint(request: PromptRequest): string | undefined {
  const candidate = (request as { model?: unknown }).model;

  const direct = firstText(candidate);
  if (direct) return direct;

  if (!candidate || typeof candidate !== "object" || Array.isArray(candidate)) {
    return undefined;
  }

  const modelObject = candidate as Record<string, unknown>;
  return (
    firstText(
      modelObject.id,
      modelObject.model_id,
      modelObject.modelId,
      modelObject.name,
    ) ?? undefined
  );
}

export class OpenHandsAcpAgent implements Agent {
  private sessions = new Map<string, OpenHandsSession>();

  constructor(private client: AgentSideConnection) {}

  async initialize(_request: InitializeRequest): Promise<InitializeResponse> {
    return {
      protocolVersion: 1,
      agentCapabilities: {
        loadSession: false,
        promptCapabilities: {
          image: false,
          audio: false,
          embeddedContext: true,
        },
        mcpCapabilities: {
          http: false,
          sse: false,
        },
      },
      agentInfo: {
        name: packageJson.name,
        title: "OpenHands ACP",
        version: packageJson.version,
      },
      authMethods: [],
    };
  }

  async newSession({ mcpServers }: NewSessionRequest): Promise<NewSessionResponse> {
    // ctx forwards MCP server metadata by default; this adapter ignores it.
    void mcpServers;

    const sessionId = randomUUID();
    this.sessions.set(sessionId, {
      sessionId,
      cancelRequested: false,
      inFlight: null,
    });

    return { sessionId };
  }

  async prompt(params: PromptRequest): Promise<PromptResponse> {
    const session = this.sessions.get(params.sessionId);
    if (!session) {
      throw new Error("session not found");
    }

    session.cancelRequested = false;
    const controller = new AbortController();
    session.inFlight = controller;

    const promptText = contentBlocksToText(params.prompt);
    const modelHint = extractModelHint(params);

    let assistantText = "";
    try {
      assistantText = await requestAssistantCompletion({
        promptText,
        modelHint,
        signal: controller.signal,
      });
    } finally {
      if (session.inFlight === controller) {
        session.inFlight = null;
      }
    }

    if (session.cancelRequested) {
      return { stopReason: "cancelled" };
    }

    const trimmed = assistantText.trim();
    if (trimmed.length > 0) {
      await this.client.sessionUpdate({
        sessionId: session.sessionId,
        update: {
          sessionUpdate: "agent_message_chunk",
          content: {
            type: "text",
            text: trimmed,
          },
        },
      });
    }

    return { stopReason: "end_turn" };
  }

  async cancel(params: CancelNotification): Promise<void> {
    const session = this.sessions.get(params.sessionId);
    if (!session) {
      throw new Error("session not found");
    }

    session.cancelRequested = true;
    session.inFlight?.abort();
    session.inFlight = null;
  }

  async authenticate(_params: AuthenticateRequest): Promise<void> {
    // Endpoint auth is supplied via env vars and is validated at request-time.
  }
}
