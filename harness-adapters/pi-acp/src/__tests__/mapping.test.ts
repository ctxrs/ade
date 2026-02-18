import test from "node:test";
import assert from "node:assert/strict";
import { contentBlocksToText } from "../mapping.js";

test("contentBlocksToText joins text and resource links", () => {
  const text = contentBlocksToText([
    { type: "text", text: "hello" },
    {
      type: "resource_link",
      uri: "file:///tmp/demo.txt",
      name: "demo.txt",
      mimeType: "text/plain",
    },
  ]);

  assert.equal(text, "hello\n\ndemo.txt (file:///tmp/demo.txt)");
});
