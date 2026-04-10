import { test } from "node:test";
import assert from "node:assert/strict";
import * as path from "node:path";
import { pathToFileURL } from "node:url";

const rootDir = path.resolve(import.meta.dirname, "..");

async function loadRuntimeModule() {
  return import(pathToFileURL(path.join(rootDir, "dist", "runtime.js")).href);
}

test("buildSessionOpenedMetadataEnvelope preserves supported command metadata", async () => {
  const { buildSessionOpenedMetadataEnvelope } = await loadRuntimeModule();

  const envelope = buildSessionOpenedMetadataEnvelope({
    sessionId: "session-1",
    providerSessionId: "provider-session-1",
    initializationResult: {
      commands: [
        {
          name: "compact",
          description: "Summarize conversation to save context",
          argumentHint: "<focus>",
        },
      ],
      agents: [
        {
          name: "Explore",
          description: "Research the repo",
          model: "sonnet",
        },
      ],
      output_style: "default",
      available_output_styles: ["default", "brief"],
      models: [
        {
          id: "sonnet",
          name: "Sonnet",
          description: "Claude Sonnet",
        },
      ],
      account: {
        email: "dev@example.com",
        organization: "ctx",
        subscriptionType: "max",
      },
      fast_mode_state: "off",
    },
    systemInit: {
      slash_commands: ["compact", "review"],
      skills: ["simplify"],
      tools: ["Read", "Write"],
      plugins: [{ name: "plugin-a", path: "/tmp/plugin-a" }],
      mcp_servers: [{ name: "github", status: "connected" }],
      model: "sonnet",
      permissionMode: "default",
    },
  });

  assert.equal(envelope.type, "session.opened");
  assert.equal(envelope.session_id, "session-1");
  assert.equal(envelope.provider_session_id, "provider-session-1");
  assert.equal(envelope.supports_session_status, true);
  assert.deepEqual(envelope.commands, [
    {
      name: "compact",
      description: "Summarize conversation to save context",
      argument_hint: "<focus>",
    },
  ]);
  assert.deepEqual(envelope.slash_commands, ["compact", "review"]);
  assert.deepEqual(envelope.skills, ["simplify"]);
  assert.deepEqual(envelope.tools, ["Read", "Write"]);
  assert.equal(envelope.current_model_id, "sonnet");
  assert.equal(envelope.permission_mode, "default");
  assert.equal(envelope.output_style, "default");
  assert.deepEqual(envelope.available_output_styles, ["default", "brief"]);
  assert.deepEqual(envelope.models, [
    {
      id: "sonnet",
      name: "Sonnet",
      description: "Claude Sonnet",
    },
  ]);
  assert.deepEqual(envelope.agents, [
    {
      name: "Explore",
      description: "Research the repo",
      model: "sonnet",
    },
  ]);
  assert.deepEqual(envelope.account, {
    email: "dev@example.com",
    organization: "ctx",
    subscription_type: "max",
  });
  assert.equal(envelope.fast_mode_state, "off");
});
