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
  assert.match(output, /helpers\/container-stack\.tar\.gz/);
  assert.match(output, /container_stack_version=v2\.2\.1/);
  assert.match(output, /container_stack_mode=curated-nerdctl-subset/);
  assert.match(output, /container_stack_inventory=.*bin\/buildctl.*bin\/buildkitd.*bin\/containerd.*libexec\/cni\/bridge/);
  assert.match(output, /nerdctl-full-2\.2\.1-linux-arm64\.tar\.gz/);
  assert.match(output, /container_stack_sha256_url=.*\/SHA256SUMS/);
  assert.match(output, /rootfs_sha256_url=.*\/SHA256SUMS/);
  assert.match(output, /unpacked_sha256_url=.*\/unpacked\/SHA256SUMS/);
  assert.match(output, /runtime_version_strategy=artifact-digests/);
  assert.match(
    output,
    /runtime_version_inputs=rootfs,kernel,initrd,guest-agent,egress-proxy,container-stack,kernel-cmdline/,
  );

  fs.rmSync(tmpRoot, { recursive: true, force: true });
});

test("prepare_avf_linux_guest_runtime.sh auto-builds guest helpers into the resolved target root", () => {
  const tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-avf-runtime-autobuild-"));
  const runtimeDir = path.join(tmpRoot, "runtime");
  const targetRoot = path.join(tmpRoot, "target-root");
  const buildScript = path.join(tmpRoot, "fake-build.sh");
  fs.writeFileSync(
    buildScript,
    [
      "#!/bin/sh",
      "set -eu",
      "target=\"${CTX_AVF_GUEST_HELPER_TARGETS:?}\"",
      "root=\"${CTX_AVF_GUEST_HELPER_TARGET_ROOT:?}\"",
      "mkdir -p \"$root/$target/release\"",
      "printf '#!/bin/sh\\nexit 0\\n' > \"$root/$target/release/ctx-avf-linux-guest-agent\"",
      "printf '#!/bin/sh\\nexit 0\\n' > \"$root/$target/release/ctx-egress-proxy\"",
      "chmod 755 \"$root/$target/release/ctx-avf-linux-guest-agent\" \"$root/$target/release/ctx-egress-proxy\"",
    ].join("\n"),
    { mode: 0o755 },
  );

  const output = execFileSync(
    "bash",
    [
      scriptPath,
      "--dry-run",
      "--output-dir",
      runtimeDir,
      "--arch",
      "arm64",
    ],
    {
      encoding: "utf8",
      env: {
        ...process.env,
        CTX_AVF_GUEST_HELPER_TARGET_ROOT: targetRoot,
        CTX_AVF_BUILD_AVF_LINUX_GUEST_HELPERS_SCRIPT: buildScript,
      },
    },
  );

  assert.match(output, new RegExp(`target_root=${targetRoot.replace(/[.*+?^${}()|[\\]\\\\]/g, "\\$&")}`));
  assert.match(output, /auto_built_guest_helpers=1/);
  assert.match(output, /guest_agent=.*ctx-avf-linux-guest-agent/);
  assert.match(output, /egress_proxy=.*ctx-egress-proxy/);
  assert.ok(
    fs.existsSync(path.join(targetRoot, "aarch64-unknown-linux-gnu", "release", "ctx-avf-linux-guest-agent")),
  );
  assert.ok(fs.existsSync(path.join(targetRoot, "aarch64-unknown-linux-gnu", "release", "ctx-egress-proxy")));

  fs.rmSync(tmpRoot, { recursive: true, force: true });
});
