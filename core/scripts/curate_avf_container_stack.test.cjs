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
    .filter((entry) => !entry.endsWith("/"))
    .sort();

  assert.deepEqual(
    listing,
    requiredEntries.map((entry) => `./${entry}`).sort(),
    "expected curated archive to contain exactly the required guest stack inventory",
  );

  fs.rmSync(tmpRoot, { recursive: true, force: true });
});

test("curate_avf_container_stack.sh fails closed when a required entry is missing", () => {
  const tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-avf-curate-stack-missing-"));
  const sourceRoot = path.join(tmpRoot, "source");
  const inputArchive = path.join(tmpRoot, "nerdctl-full.tar.gz");
  const outputArchive = path.join(tmpRoot, "container-stack.tar.gz");
  const presentEntries = [
    "bin/buildctl",
    "bin/buildkitd",
    "bin/containerd",
    "bin/containerd-shim-runc-v2",
    "bin/ctr",
    "bin/nerdctl",
    "libexec/cni/bridge",
    "libexec/cni/firewall",
    "libexec/cni/host-local",
    "libexec/cni/loopback",
    "libexec/cni/portmap",
    "libexec/cni/tuning",
  ];

  for (const entry of presentEntries) {
    const fullPath = path.join(sourceRoot, entry);
    fs.mkdirSync(path.dirname(fullPath), { recursive: true });
    fs.writeFileSync(fullPath, `${entry}\n`, { mode: 0o755 });
  }

  execFileSync("tar", ["-czf", inputArchive, "-C", sourceRoot, "."], {
    stdio: "inherit",
  });

  assert.throws(
    () =>
      execFileSync("bash", [scriptPath, "--input", inputArchive, "--output", outputArchive], {
        stdio: "pipe",
        encoding: "utf8",
      }),
    /required container-stack entry is missing: bin\/runc/,
  );

  fs.rmSync(tmpRoot, { recursive: true, force: true });
});

test("curate_avf_container_stack.sh produces deterministic archives", () => {
  const tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-avf-curate-stack-deterministic-"));
  const sourceRoot = path.join(tmpRoot, "source");
  const inputArchive = path.join(tmpRoot, "nerdctl-full.tar.gz");
  const outputArchiveA = path.join(tmpRoot, "container-stack-a.tar.gz");
  const outputArchiveB = path.join(tmpRoot, "container-stack-b.tar.gz");
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

  for (const entry of requiredEntries) {
    const fullPath = path.join(sourceRoot, entry);
    fs.mkdirSync(path.dirname(fullPath), { recursive: true });
    fs.writeFileSync(fullPath, `${entry}\n`, { mode: 0o755 });
  }

  execFileSync("tar", ["-czf", inputArchive, "-C", sourceRoot, "."], {
    stdio: "inherit",
  });
  execFileSync("bash", [scriptPath, "--input", inputArchive, "--output", outputArchiveA], {
    stdio: "inherit",
  });
  execFileSync("bash", [scriptPath, "--input", inputArchive, "--output", outputArchiveB], {
    stdio: "inherit",
  });

  const hashA = execFileSync("shasum", ["-a", "256", outputArchiveA], { encoding: "utf8" }).split(" ")[0];
  const hashB = execFileSync("shasum", ["-a", "256", outputArchiveB], { encoding: "utf8" }).split(" ")[0];
  assert.equal(hashA, hashB);
  assert.equal(fs.readFileSync(outputArchiveA).equals(fs.readFileSync(outputArchiveB)), true);

  fs.rmSync(tmpRoot, { recursive: true, force: true });
});

test("curate_avf_container_stack.sh produces identical archives from identical inputs in different source roots", () => {
  const tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-avf-curate-stack-cross-root-"));
  const inputArchiveA = path.join(tmpRoot, "nerdctl-full-a.tar.gz");
  const inputArchiveB = path.join(tmpRoot, "nerdctl-full-b.tar.gz");
  const outputArchiveA = path.join(tmpRoot, "container-stack-a.tar.gz");
  const outputArchiveB = path.join(tmpRoot, "container-stack-b.tar.gz");
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

  for (const suffix of ["a", "b"]) {
    const sourceRoot = path.join(tmpRoot, `source-${suffix}`);
    for (const entry of requiredEntries) {
      const fullPath = path.join(sourceRoot, entry);
      fs.mkdirSync(path.dirname(fullPath), { recursive: true });
      fs.writeFileSync(fullPath, `${entry}\n`, { mode: 0o755 });
    }
    execFileSync("tar", ["-czf", suffix === "a" ? inputArchiveA : inputArchiveB, "-C", sourceRoot, "."], {
      stdio: "inherit",
    });
  }

  execFileSync("bash", [scriptPath, "--input", inputArchiveA, "--output", outputArchiveA], {
    stdio: "inherit",
  });
  execFileSync("bash", [scriptPath, "--input", inputArchiveB, "--output", outputArchiveB], {
    stdio: "inherit",
  });

  assert.equal(fs.readFileSync(outputArchiveA).equals(fs.readFileSync(outputArchiveB)), true);

  fs.rmSync(tmpRoot, { recursive: true, force: true });
});
