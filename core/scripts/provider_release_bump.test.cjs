const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const { applyBumpPlan } = require("./provider_release_bump.cjs");

function writeJson(filePath, value) {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, `${JSON.stringify(value, null, 2)}\n`, "utf8");
}

function writeText(filePath, value) {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, value, "utf8");
}

function makeTempRepo() {
  return fs.mkdtempSync(path.join(os.tmpdir(), "ctx-provider-release-bump."));
}

function readJson(filePath) {
  return JSON.parse(fs.readFileSync(filePath, "utf8"));
}

test("codex bumps fail closed when provenance is missing", async () => {
  const repoRoot = makeTempRepo();
  const matrix = {
    version: 3,
    providers: [
      {
        id: "codex",
        managed_install: {
          kind: "archive",
          version: "0.124.0-ctx.1",
          targets: {},
        },
        releases: [
          {
            version: "0.124.0-ctx.1",
            status: "supported",
            context_min: "0.1.0",
            notes: "Thin app-server adapter over stock Codex rust-v0.124.0",
            upstream_version: "0.124.0",
            provenance: {
              upstream_repo: "openai/codex",
              upstream_release_tag: "rust-v0.124.0",
              upstream_commit_sha: "e9fb49366c93a1478ec71cc41ecee415a197d036",
              ctx_repo: "ctxorgrs/codex-crp",
              ctx_release_tag: "v0.124.0-ctx.1",
            },
          },
        ],
      },
    ],
  };
  writeText(
    path.join(repoRoot, "core", "crates", "codex-crp", "Cargo.toml"),
    '[package]\nname = "codex-crp"\nversion = "0.124.0-ctx.1"\nedition = "2021"\n',
  );

  await assert.rejects(
    () =>
      applyBumpPlan({
        matrix,
        repoRoot,
        updates: [
          {
            id: "codex",
            version: "1.0.0",
            upstream_version: "0.124.0",
          },
        ],
        index: null,
        indexPath: "",
        artifactBaseUrl: "",
      }),
    /codex bumps require a provenance object/,
  );
});

test("codex bumps rewrite cargo version and provenance metadata", async () => {
  const repoRoot = makeTempRepo();
  const matrix = {
    version: 3,
    providers: [
      {
        id: "codex",
        managed_install: {
          kind: "archive",
          version: "0.124.0-ctx.1",
          targets: {},
        },
        releases: [
          {
            version: "0.124.0-ctx.1",
            status: "supported",
            context_min: "0.1.0",
            notes: "Thin app-server adapter over stock Codex rust-v0.124.0",
            upstream_version: "0.124.0",
            provenance: {
              upstream_repo: "openai/codex",
              upstream_release_tag: "rust-v0.124.0",
              upstream_commit_sha: "e9fb49366c93a1478ec71cc41ecee415a197d036",
              ctx_repo: "ctxorgrs/codex-crp",
              ctx_release_tag: "v0.124.0-ctx.1",
            },
          },
        ],
      },
    ],
  };
  const cargoToml = path.join(repoRoot, "core", "crates", "codex-crp", "Cargo.toml");
  writeText(
    cargoToml,
    '[package]\nname = "codex-crp"\nversion = "0.124.0-ctx.1"\nedition = "2021"\n',
  );

  const result = await applyBumpPlan({
    matrix,
    repoRoot,
    updates: [
      {
        id: "codex",
        version: "1.0.0",
        provenance: {
          upstream_release_tag: "rust-v0.124.0",
          upstream_commit_sha: "0123456789abcdef0123456789abcdef01234567",
        },
      },
    ],
    index: null,
    indexPath: "",
    artifactBaseUrl: "",
  });

  const codex = matrix.providers[0];
  assert.equal(codex.managed_install.version, "1.0.0");
  assert.equal(codex.releases[0].version, "1.0.0");
  assert.equal(codex.releases[0].upstream_version, "0.124.0");
  assert.equal(codex.releases[0].provenance.upstream_repo, "openai/codex");
  assert.equal(codex.releases[0].provenance.ctx_repo, "ctxorgrs/codex-crp");
  assert.equal(codex.releases[0].provenance.ctx_release_tag, "v1.0.0");
  assert.equal(codex.releases[0].notes, "Thin app-server adapter over stock Codex rust-v0.124.0");
  assert.match(fs.readFileSync(cargoToml, "utf8"), /version = "1\.0\.0"/);
  assert.match(result.warnings.join("\n"), /archive version changed/);
});

test("github release refresh rewrites archive urls and sha256 for external providers", async () => {
  const repoRoot = makeTempRepo();
  const matrix = {
    version: 3,
    providers: [
      {
        id: "codex-cli",
        managed_install: {
          kind: "archive",
          version: "rust-v0.121.0",
          targets: {
            "darwin-aarch64": {
              url: "https://github.com/openai/codex/releases/download/rust-v0.121.0/codex-aarch64-apple-darwin.tar.gz",
              archive: "tar_gz",
              bin_path: "codex-aarch64-apple-darwin",
              sha256: "old",
            },
            "linux-x86_64": {
              url: "https://github.com/openai/codex/releases/download/rust-v0.121.0/codex-x86_64-unknown-linux-gnu.tar.gz",
              archive: "tar_gz",
              bin_path: "codex-x86_64-unknown-linux-gnu",
              sha256: "old2",
            },
          },
        },
        releases: [
          {
            version: "rust-v0.121.0",
            status: "supported",
            context_min: "0.1.0",
            notes: "Pinned stock Codex CLI release",
            upstream_version: "0.121.0",
          },
        ],
      },
    ],
  };

  const requests = [];
  const result = await applyBumpPlan({
    matrix,
    repoRoot,
    updates: [
      {
        id: "codex-cli",
        version: "rust-v0.124.0",
        upstream_version: "0.124.0",
        github_release: {
          tag: "rust-v0.124.0",
        },
      },
    ],
    index: null,
    indexPath: "",
    artifactBaseUrl: "",
    network: {
      async fetchGithubRelease(repo, tag) {
        assert.equal(repo, "openai/codex");
        assert.equal(tag, "rust-v0.124.0");
        return {
          repo,
          tag,
          assetsByName: new Map([
            [
              "codex-aarch64-apple-darwin.tar.gz",
              "https://github.com/openai/codex/releases/download/rust-v0.124.0/codex-aarch64-apple-darwin.tar.gz",
            ],
            [
              "codex-x86_64-unknown-linux-gnu.tar.gz",
              "https://github.com/openai/codex/releases/download/rust-v0.124.0/codex-x86_64-unknown-linux-gnu.tar.gz",
            ],
          ]),
        };
      },
      async hashUrlSha256(url) {
        requests.push(url);
        if (url.includes("aarch64-apple-darwin")) {
          return "a".repeat(64);
        }
        return "b".repeat(64);
      },
    },
  });

  const codexCli = matrix.providers[0];
  assert.equal(result.warnings.length, 0);
  assert.deepEqual(requests, [
    "https://github.com/openai/codex/releases/download/rust-v0.124.0/codex-aarch64-apple-darwin.tar.gz",
    "https://github.com/openai/codex/releases/download/rust-v0.124.0/codex-x86_64-unknown-linux-gnu.tar.gz",
  ]);
  assert.equal(codexCli.managed_install.version, "rust-v0.124.0");
  assert.equal(codexCli.releases[0].upstream_version, "0.124.0");
  assert.equal(
    codexCli.managed_install.targets["darwin-aarch64"].url,
    "https://github.com/openai/codex/releases/download/rust-v0.124.0/codex-aarch64-apple-darwin.tar.gz",
  );
  assert.equal(codexCli.managed_install.targets["darwin-aarch64"].sha256, "a".repeat(64));
  assert.equal(
    codexCli.managed_install.targets["linux-x86_64"].url,
    "https://github.com/openai/codex/releases/download/rust-v0.124.0/codex-x86_64-unknown-linux-gnu.tar.gz",
  );
  assert.equal(codexCli.managed_install.targets["linux-x86_64"].sha256, "b".repeat(64));
});
