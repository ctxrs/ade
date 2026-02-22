import test from "node:test";
import assert from "node:assert/strict";
import { extractAssistantText, resolveRuntimeConfig } from "../openai.js";

test("extractAssistantText reads string content from chat completion payload", () => {
  const payload = {
    choices: [
      {
        message: {
          content: "pong",
        },
      },
    ],
  };
  assert.equal(extractAssistantText(payload), "pong");
});

test("resolveRuntimeConfig picks OPENAI env by precedence", () => {
  const snapshot = {
    OPENAI_API_KEY: process.env.OPENAI_API_KEY,
    OPENAI_BASE_URL: process.env.OPENAI_BASE_URL,
    OPENAI_MODEL: process.env.OPENAI_MODEL,
  };

  process.env.OPENAI_API_KEY = "test-key";
  process.env.OPENAI_BASE_URL = "https://openrouter.ai/api/v1/";
  process.env.OPENAI_MODEL = "openai/gpt-5.2-codex";

  try {
    const cfg = resolveRuntimeConfig(undefined);
    assert.equal(cfg.apiKey, "test-key");
    assert.equal(cfg.baseUrl, "https://openrouter.ai/api/v1");
    assert.equal(cfg.model, "openai/gpt-5.2-codex");
  } finally {
    if (snapshot.OPENAI_API_KEY === undefined) {
      delete process.env.OPENAI_API_KEY;
    } else {
      process.env.OPENAI_API_KEY = snapshot.OPENAI_API_KEY;
    }

    if (snapshot.OPENAI_BASE_URL === undefined) {
      delete process.env.OPENAI_BASE_URL;
    } else {
      process.env.OPENAI_BASE_URL = snapshot.OPENAI_BASE_URL;
    }

    if (snapshot.OPENAI_MODEL === undefined) {
      delete process.env.OPENAI_MODEL;
    } else {
      process.env.OPENAI_MODEL = snapshot.OPENAI_MODEL;
    }
  }
});
