import test from "node:test";
import assert from "node:assert/strict";
import { AgentSideConnection, NewSessionRequest } from "@agentclientprotocol/sdk";
import { OpenHandsAcpAgent } from "../agent.js";

test("newSession tolerates forwarded MCP stdio servers", async () => {
  const agent = new OpenHandsAcpAgent({} as AgentSideConnection);
  const req: NewSessionRequest = {
    cwd: process.cwd(),
    mcpServers: [
      {
        name: "ctx",
        command: "ctx",
        args: ["mcp", "serve"],
        env: [],
      },
    ],
  };
  const response = await agent.newSession(req);
  assert.ok(response.sessionId.length > 0);
});
