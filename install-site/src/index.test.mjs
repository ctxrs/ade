import test from "node:test";
import assert from "node:assert/strict";
import worker from "./index.ts";

test("GET /uninstall returns the uninstall shell script", async () => {
  const response = await worker.fetch(new Request("https://ctx.rs/uninstall"));
  const body = await response.text();

  assert.equal(response.status, 200);
  assert.equal(response.headers.get("content-type"), "text/x-shellscript; charset=utf-8");
  assert.match(body, /^#!\/bin\/sh/m);
  assert.match(body, /ctx uninstall complete\./);
});

test("GET /uninstall.sh returns the uninstall shell script", async () => {
  const response = await worker.fetch(new Request("https://ctx.rs/uninstall.sh"));
  const body = await response.text();

  assert.equal(response.status, 200);
  assert.equal(response.headers.get("content-type"), "text/x-shellscript; charset=utf-8");
  assert.match(body, /confirm_uninstall/);
});

test("GET / root advertises install and uninstall commands", async () => {
  const response = await worker.fetch(new Request("https://ctx.rs/"));
  const body = await response.text();

  assert.equal(response.status, 200);
  assert.match(body, /curl -fsSL https:\/\/ctx\.rs\/install \| sh/);
  assert.match(body, /curl -fsSL https:\/\/ctx\.rs\/uninstall \| sh/);
});
