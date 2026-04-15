import test from "node:test";
import assert from "node:assert/strict";
import worker from "./index.js";

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

test("GET /install returns the install shell script with normalized attribution defaults", async () => {
  const response = await worker.fetch(
    new Request(
      "https://ctx.rs/install?ctx_download_id=ctx-download-123&utm_source=hello world&utm_medium=Email&utm_campaign=Spring Sale&referrer_domain=https://Example.COM/path",
    ),
  );
  const body = await response.text();

  assert.equal(response.status, 200);
  assert.equal(response.headers.get("content-type"), "text/x-shellscript; charset=utf-8");
  assert.match(body, /^#!\/bin\/sh/m);
  assert.match(body, /download_id="\$\{CTX_DOWNLOAD_ID:-ctx-download-123\}"/);
  assert.match(body, /referrer_domain="\$\{CTX_INSTALL_REFERRER_DOMAIN:-example\.com\}"/);
  assert.match(body, /utm_source="\$\{CTX_INSTALL_UTM_SOURCE:-hello_world\}"/);
  assert.match(body, /utm_medium="\$\{CTX_INSTALL_UTM_MEDIUM:-Email\}"/);
  assert.match(body, /utm_campaign="\$\{CTX_INSTALL_UTM_CAMPAIGN:-Spring_Sale\}"/);
});

test("GET /install.sh falls back to a normalized Referer hostname and generated download id", async () => {
  const response = await worker.fetch(
    new Request("https://ctx.rs/install.sh?utm_source=desktop-launch", {
      headers: {
        referer: "https://Docs.Ctx.rs/guides/install?from=nav",
      },
    }),
  );
  const body = await response.text();

  assert.equal(response.status, 200);
  assert.equal(response.headers.get("content-type"), "text/x-shellscript; charset=utf-8");
  assert.match(body, /download_id="\$\{CTX_DOWNLOAD_ID:-[0-9a-f-]{36}\}"/);
  assert.match(body, /referrer_domain="\$\{CTX_INSTALL_REFERRER_DOMAIN:-docs\.ctx\.rs\}"/);
  assert.match(body, /utm_source="\$\{CTX_INSTALL_UTM_SOURCE:-desktop-launch\}"/);
});

test("GET / root advertises install and uninstall commands", async () => {
  const response = await worker.fetch(new Request("https://ctx.rs/"));
  const body = await response.text();

  assert.equal(response.status, 200);
  assert.match(body, /curl -fsSL https:\/\/ctx\.rs\/install \| sh/);
  assert.match(body, /curl -fsSL https:\/\/ctx\.rs\/uninstall \| sh/);
});
