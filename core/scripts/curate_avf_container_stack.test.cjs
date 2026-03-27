const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { execFileSync } = require("node:child_process");

const scriptPath = path.join(__dirname, "curate_avf_container_stack.sh");

test("curate_avf_container_stack.sh keeps the required guest stack subset only", () => {
  const tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-avf-curate-stack-"));
  const sourceRoot = path.join(tmpRoot, "source");
  const inputArchive = path.join(tmpRoot, "nerdctl-full.tar.gz");
  const outputArchive = path.join(tmpRoot, "container-stack.tar.gz");
  const requiredEntries = [
    "bin/buildctl",
    "bin/buildkitd",
    "bin/containerd",
    "bin/containerd-shim-runc-v2",
    "bin/ctr",
    "bin/nerdctl",
    "bin/runc",
    "libexec/cni/bridge",
    "libexec/cni/firewall",
    "libexec/cni/host-local",
    "libexec/cni/loopback",
    "libexec/cni/portmap",
    "libexec/cni/tuning",
  ];
  const extraEntries = [
    "bin/buildg",
    "bin/containerd-stargz-grpc",
    "bin/ctr-enc",
    "bin/ctr-remote",
    "bin/ctd-decoder",
    "bin/nerdctl.gomodjail",
    "libexec/cni/dnsname",
    "share/doc/README.md",
  ];

  for (const entry of [...requiredEntries, ...extraEntries]) {
    const fullPath = path.join(sourceRoot, entry);
    fs.mkdirSync(path.dirname(fullPath), { recursive: true });
    fs.writeFileSync(fullPath, `${entry}\n`, { mode: 0o755 });
  }

  execFileSync("tar", ["-czf", inputArchive, "-C", sourceRoot, "."], {
    stdio: "inherit",
  });
  execFileSync("bash", [scriptPath, "--input", inputArchive, "--output", outputArchive], {
    stdio: "inherit",
  });

  const listing = execFileSync("tar", ["-tzf", outputArchive], {
    encoding: "utf8",
  })
    .trim()
    .split("\n")
    .filter(Boolean)
    .sort();

  for (const entry of requiredEntries) {
    assert.ok(
      listing.includes(`./${entry}`),
      `expected curated archive to keep ${entry}`,
    );
  }
  for (const entry of extraEntries) {
    assert.ok(
      !listing.includes(`./${entry}`),
      `expected curated archive to drop ${entry}`,
    );
  }

  fs.rmSync(tmpRoot, { recursive: true, force: true });
});
