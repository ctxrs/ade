const test = require("node:test");
const assert = require("node:assert/strict");

const { expectedRemoteRootPrefix } = require("./helpers/remote_container_contract_paths.cjs");

test("sandbox new workspaces resolve under the managed remote staging prefix", () => {
  assert.equal(
    expectedRemoteRootPrefix({
      remoteBase: "/tmp/ctx-remote-contract-123",
      remoteDataDir: "/tmp/ctx-fixture/daemon-sandbox",
      container: "sandbox",
      sourceKind: "new",
    }),
    "/tmp/ctx-fixture/daemon-sandbox/workspaces/staging/",
  );
});

test("host workspaces continue to resolve under the explicit remote base", () => {
  assert.equal(
    expectedRemoteRootPrefix({
      remoteBase: "/tmp/ctx-remote-contract-456",
      remoteDataDir: "/tmp/ctx-fixture/daemon-host",
      container: "host",
      sourceKind: "new",
    }),
    "/tmp/ctx-remote-contract-456/",
  );
});
