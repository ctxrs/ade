import test from "node:test";
import assert from "node:assert/strict";
import { AgentSideConnection, NewSessionRequest } from "@agentclientprotocol/sdk";
import { PiAcpAgent } from "../agent.js";

test("newSession tolerates forwarded MCP stdio servers", async () => {
  const originalCommand = process.env.PI_ACP_PI_COMMAND;
  process.env.PI_ACP_PI_COMMAND = "/usr/bin/true";

  try {
    const agent = new PiAcpAgent({} as AgentSideConnection);
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
  } finally {
    if (originalCommand === undefined) {
      delete process.env.PI_ACP_PI_COMMAND;
    } else {
      process.env.PI_ACP_PI_COMMAND = originalCommand;
    }
  }
});
