import { test } from "node:test";
import assert from "node:assert/strict";
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import { pathToFileURL } from "node:url";

const rootDir = path.resolve(import.meta.dirname, "..");

async function loadRuntimeModule() {
  return import(pathToFileURL(path.join(rootDir, "dist", "runtime.js")).href);
}

test("resolvePromptInput keeps text-only items on the flat prompt path", async () => {
  const { resolvePromptInput } = await loadRuntimeModule();

  const resolved = resolvePromptInput(
    {
      items: [
        { type: "text", text: "describe the screenshot" },
        "and note anything unusual",
      ],
    },
    {
      sessionId: "session-1",
      cwd: rootDir,
      env: {},
    },
  );

  assert.deepEqual(resolved, {
    kind: "text",
    prompt: "describe the screenshot\nand note anything unusual",
  });
});

test("resolvePromptInput turns image_ref items into structured Claude messages", async () => {
  const { resolvePromptInput } = await loadRuntimeModule();

  const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), "claude-crp-image-ref-"));
  const blobDir = path.join(tempRoot, "blobs");
  const bytes = Buffer.from([1, 2, 3, 4, 5]);
  await fs.mkdir(blobDir, { recursive: true });
  await fs.writeFile(path.join(blobDir, "blob-1"), bytes);

  const resolved = resolvePromptInput(
    {
      items: [
        { type: "text", text: "describe this image" },
        { type: "image_ref", blob_id: "blob-1", mime_type: "image/png" },
      ],
    },
    {
      sessionId: "session-1",
      cwd: rootDir,
      env: { CTX_DATA_ROOT_HOST: tempRoot },
    },
  );

  assert.equal(resolved?.kind, "structured");
  assert.equal(resolved.messages.length, 1);
  assert.equal(resolved.messages[0].session_id, "session-1");
  assert.equal(resolved.messages[0].parent_tool_use_id, null);
  assert.deepEqual(resolved.messages[0].message, {
    role: "user",
    content: [
      { type: "text", text: "describe this image" },
      {
        type: "image",
        source: {
          type: "base64",
          media_type: "image/png",
          data: bytes.toString("base64"),
        },
      },
    ],
  });
});

test("resolvePromptInput keeps explicit local_image compatibility by reading the file", async () => {
  const { resolvePromptInput } = await loadRuntimeModule();

  const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), "claude-crp-local-image-"));
  const imagePath = path.join(tempRoot, "example.png");
  const bytes = Buffer.from([9, 8, 7, 6]);
  await fs.writeFile(imagePath, bytes);

  const resolved = resolvePromptInput(
    {
      items: [{ type: "local_image", path: imagePath }],
    },
    {
      sessionId: "session-2",
      cwd: rootDir,
      env: {},
    },
  );

  assert.equal(resolved?.kind, "structured");
  assert.deepEqual(resolved.messages[0].message, {
    role: "user",
    content: [
      {
        type: "image",
        source: {
          type: "base64",
          media_type: "image/png",
          data: bytes.toString("base64"),
        },
      },
    ],
  });
});

test("resolvePromptInput fails explicitly on Claude-unsupported image mime types", async () => {
  const { resolvePromptInput } = await loadRuntimeModule();

  const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), "claude-crp-unsupported-image-"));
  const blobDir = path.join(tempRoot, "blobs");
  await fs.mkdir(blobDir, { recursive: true });
  await fs.writeFile(path.join(blobDir, "blob-unsupported"), Buffer.from([1, 2, 3]));

  assert.throws(
    () =>
      resolvePromptInput(
        {
          items: [
            {
              type: "image_ref",
              blob_id: "blob-unsupported",
              mime_type: "image/svg+xml",
            },
          ],
        },
        {
          sessionId: "session-3",
          cwd: rootDir,
          env: { CTX_DATA_ROOT_HOST: tempRoot },
        },
      ),
    /unsupported image mime type/i,
  );
});
