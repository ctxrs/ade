const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("fs");
const os = require("os");
const path = require("path");
const { execFileSync } = require("child_process");

const scriptPath = path.join(__dirname, "prepare_avf_linux_guest_runtime.sh");

test("prepare_avf_linux_guest_runtime.sh resolves official Ubuntu inputs in dry-run mode", () => {
  const tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-avf-runtime-"));
  const runtimeDir = path.join(tmpRoot, "runtime");
  const guestAgent = path.join(tmpRoot, "ctx-avf-linux-guest-agent");
  const egressProxy = path.join(tmpRoot, "ctx-egress-proxy");
  fs.writeFileSync(guestAgent, "#!/bin/sh\nexit 0\n", { mode: 0o755 });
  fs.writeFileSync(egressProxy, "#!/bin/sh\nexit 0\n", { mode: 0o755 });

  const output = execFileSync(
    "bash",
    [
      scriptPath,
      "--dry-run",
      "--output-dir",
      runtimeDir,
      "--arch",
      "arm64",
      "--guest-agent",
      guestAgent,
      "--egress-proxy",
      egressProxy,
    ],
    {
      encoding: "utf8",
      env: {
        ...process.env,
        PATH: `${process.env.HOME}/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin`,
      },
    },
  );

  assert.match(output, /runtime_dir=.*\/runtime/);
  assert.match(output, /arch=arm64/);
  assert.match(output, /ubuntu_arch=arm64/);
  assert.match(output, /guest_agent=.*ctx-avf-linux-guest-agent/);
  assert.match(output, /ubuntu-24\.04-server-cloudimg-arm64\.img/);
  assert.match(output, /ubuntu-24\.04-server-cloudimg-arm64-vmlinuz-generic/);
  assert.match(output, /ubuntu-24\.04-server-cloudimg-arm64-initrd-generic/);
  assert.match(output, /rootfs\.raw/);
  assert.match(output, /helpers\/kernel-cmdline/);
  assert.match(output, /cloudimg-rootfs/);
  assert.match(output, /helpers\/guest-agent/);
  assert.match(output, /helpers\/egress-proxy/);
  assert.match(output, /rootfs_sha256_url=.*\/SHA256SUMS/);
  assert.match(output, /unpacked_sha256_url=.*\/unpacked\/SHA256SUMS/);

  fs.rmSync(tmpRoot, { recursive: true, force: true });
});
