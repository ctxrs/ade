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
import { PiRpcClient } from "./rpc.js";

type PiSession = {
  sessionId: string;
  rpc: PiRpcClient;
  cancelRequested: boolean;
};

export class PiAcpAgent implements Agent {
  private sessions = new Map<string, PiSession>();

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
        title: "Pi ACP",
        version: packageJson.version,
      },
      authMethods: [],
    };
  }

  async newSession({ cwd, mcpServers }: NewSessionRequest): Promise<NewSessionResponse> {
    // ctx forwards MCP servers by default on session open. Pi RPC mode does not currently
    // consume forwarded MCP configuration, so we tolerate and ignore these entries.
    void mcpServers;

    const sessionId = randomUUID();
    const rpc = new PiRpcClient(cwd);

    this.sessions.set(sessionId, {
      sessionId,
      rpc,
      cancelRequested: false,
    });

    return { sessionId };
  }

  async prompt(params: PromptRequest): Promise<PromptResponse> {
    const session = this.sessions.get(params.sessionId);
    if (!session) {
      throw new Error("Session not found");
    }

    session.cancelRequested = false;
    const promptText = contentBlocksToText(params.prompt);

    await session.rpc.prompt(promptText);
    await session.rpc.waitForIdle();

    const assistantText = await session.rpc.getLastAssistantText();
    if (assistantText && assistantText.trim()) {
      await this.client.sessionUpdate({
        sessionId: session.sessionId,
        update: {
          sessionUpdate: "agent_message_chunk",
          content: {
            type: "text",
            text: assistantText,
          },
        },
      });
    }

    if (session.cancelRequested) {
      return { stopReason: "cancelled" };
    }

    return { stopReason: "end_turn" };
  }

  async cancel(params: CancelNotification): Promise<void> {
    const session = this.sessions.get(params.sessionId);
    if (!session) {
      throw new Error("Session not found");
    }
    session.cancelRequested = true;
    await session.rpc.abort();
  }

  async authenticate(_params: AuthenticateRequest): Promise<void> {
    // Pi itself handles provider auth/session state. We do not perform adapter-side auth checks.
  }
}
