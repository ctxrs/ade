#!/usr/bin/env node

const assert = require("node:assert/strict");
const childProcess = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const {
  detectCapturePolicyEvidence,
  detectAnalyticsEnvironmentEvidence,
  resolveWebDistCandidates,
  verifyCapturePolicyEnabled,
  verifyFinalArtifactAnalytics,
  verifyWebDistAnalytics,
} = require("./final_artifact_analytics_gate.cjs");

const scriptPath = path.join(__dirname, "final_artifact_analytics_gate.cjs");

function withTempDir(name, fn) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), name));
  try {
    return fn(root);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
}

function writeFixtureDist(webDist, js) {
  fs.mkdirSync(path.join(webDist, "assets"), { recursive: true });
  fs.writeFileSync(
    path.join(webDist, "index.html"),
    '<!doctype html><script type="module" src="/assets/index.js"></script>',
    "utf8",
  );
  fs.writeFileSync(path.join(webDist, "assets", "index.js"), js, "utf8");
}

function minifiedAnalyticsBundle(env, version = "1.2.3") {
  return [
    'const Ay=e=>{const t=e==null?void 0:e.trim();return t||void 0}',
    'Phe=(e,t,n)=>{var i;const r=String(t).trim().toLowerCase();if(r==="development"||r==="dev")return"staging";const s=(i=Ay(e))==null?void 0:i.toLowerCase();return s==="production"?"production":s==="staging"?"staging":r==="production"&&Ay(n)?"production":"staging"}',
    `c6=()=>Phe("${env}","production","${version}")`,
    `Fhe=()=>"${version}".trim()||"0.0.0"`,
    'Hhe=(e,t={})=>({event_version:e,app_version:Fhe(),analytics_environment:c6(),traffic_class:"user",...t})',
    'Qct=e=>!e.settingsLoaded||!e.telemetryEnabled?!1:e.isDev?!1:!0',
  ].join(",");
}

test("verifyWebDistAnalytics accepts production minified web assets with the expected version", () => {
  withTempDir("ctx-final-artifact-analytics-ok-", (root) => {
    writeFixtureDist(root, minifiedAnalyticsBundle("production"));
    const result = verifyWebDistAnalytics(root, {
      expectedVersion: "1.2.3",
      expectedAnalyticsEnvironment: "production",
    });
    assert.equal(result.textAssetCount, 2);
    assert.deepEqual(result.versionEvidence, ["assets/index.js"]);
    assert.equal(result.analyticsEvidenceCount, 1);
  });
});

test("verifyWebDistAnalytics rejects staging analytics in minified resolver form", () => {
  withTempDir("ctx-final-artifact-analytics-staging-", (root) => {
    writeFixtureDist(root, minifiedAnalyticsBundle("staging"));
    assert.throws(
      () => verifyWebDistAnalytics(root, {
        expectedVersion: "1.2.3",
        expectedAnalyticsEnvironment: "production",
      }),
      /resolve analytics_environment to staging, expected production/,
    );
  });
});

test("verifyWebDistAnalytics accepts inferred production resolver with undefined explicit env", () => {
  withTempDir("ctx-final-artifact-analytics-inferred-production-", (root) => {
    writeFixtureDist(
      root,
      [
        'const Ay=e=>{const t=e==null?void 0:e.trim();return t||void 0}',
        'Phe=(e,t,n)=>{const r=String(t).trim().toLowerCase();if(r==="development"||r==="dev")return"staging";const s=Ay(e)?.toLowerCase();return s==="production"?"production":s==="staging"?"staging":r==="production"&&Ay(n)?"production":"staging"}',
        'c6=()=>Phe(void 0,"production","1.2.3")',
        'Hhe=(e,t={})=>({app_version:"1.2.3",analytics_environment:c6(),traffic_class:"user",...t})',
        'Qct=e=>!e.settingsLoaded||!e.telemetryEnabled?!1:e.isDev?!1:!0',
      ].join(","),
    );
    const result = verifyWebDistAnalytics(root, {
      expectedVersion: "1.2.3",
      expectedAnalyticsEnvironment: "production",
    });
    assert.equal(result.analyticsEvidenceCount, 1);
  });
});

test("verifyWebDistAnalytics rejects direct staging analytics property", () => {
  withTempDir("ctx-final-artifact-analytics-direct-staging-", (root) => {
    writeFixtureDist(
      root,
      'const event={app_version:"1.2.3",analytics_environment:"staging",traffic_class:"user"};',
    );
    assert.throws(
      () => verifyWebDistAnalytics(root, {
        expectedVersion: "1.2.3",
        expectedAnalyticsEnvironment: "production",
      }),
      /direct-property/,
    );
  });
});

test("verifyWebDistAnalytics rejects missing app version", () => {
  withTempDir("ctx-final-artifact-analytics-missing-version-", (root) => {
    writeFixtureDist(root, minifiedAnalyticsBundle("production", "9.9.9"));
    assert.throws(
      () => verifyWebDistAnalytics(root, {
        expectedVersion: "1.2.3",
        expectedAnalyticsEnvironment: "production",
      }),
      /do not contain expected app version: 1\.2\.3/,
    );
  });
});

test("verifyWebDistAnalytics rejects bundles compiled to dev-only capture", () => {
  withTempDir("ctx-final-artifact-analytics-dev-only-capture-", (root) => {
    writeFixtureDist(
      root,
      [
        minifiedAnalyticsBundle("production"),
        'Yct=e=>["1","true","yes","on"].includes("".trim().toLowerCase())',
        'Qct=e=>!e.settingsLoaded||!e.telemetryEnabled?!1:Yct()',
      ].join(","),
    );
    assert.throws(
      () => verifyWebDistAnalytics(root, {
        expectedVersion: "1.2.3",
        expectedAnalyticsEnvironment: "production",
      }),
      /dev\/CI override path/,
    );
  });
});

test("verifyCapturePolicyEnabled accepts normal user capture evidence", () => {
  const assets = [{
    relativePath: "assets/index.js",
    text: 'Qct=e=>!e.settingsLoaded||!e.telemetryEnabled?!1:e.isDev?!1:!0',
  }];
  const evidence = detectCapturePolicyEvidence(assets);
  assert.equal(evidence.length, 1);
  assert.equal(evidence[0].normalUserCapture, true);
  assert.doesNotThrow(() => verifyCapturePolicyEnabled(assets));
});

test("verifyCapturePolicyEnabled rejects dev flag only capture evidence", () => {
  const assets = [{
    relativePath: "assets/index.js",
    text: 'Yct=e=>["1","true","yes","on"].includes("".trim().toLowerCase()),Qct=e=>!e.settingsLoaded||!e.telemetryEnabled?!1:Yct()',
  }];
  assert.throws(() => verifyCapturePolicyEnabled(assets), /dev\/CI override path/);
});

test("verifyWebDistAnalytics fails closed when analytics resolver cannot be proven", () => {
  withTempDir("ctx-final-artifact-analytics-unresolved-", (root) => {
    writeFixtureDist(
      root,
      'const runtimeResolver=window.__analyticsEnv;const event={app_version:"1.2.3",analytics_environment:runtimeResolver()};',
    );
    assert.throws(
      () => verifyWebDistAnalytics(root, {
        expectedVersion: "1.2.3",
        expectedAnalyticsEnvironment: "production",
      }),
      /could not prove compiled web assets resolve analytics_environment to production/,
    );
  });
});

test("resolveWebDistCandidates finds Linux AppImage extracted web dist layout", () => {
  withTempDir("ctx-final-artifact-analytics-appimage-", (root) => {
    const webDist = path.join(root, "squashfs-root", "usr", "lib", "ctx", "web", "dist");
    writeFixtureDist(webDist, minifiedAnalyticsBundle("production"));
    const candidates = resolveWebDistCandidates({ artifactRoot: path.join(root, "squashfs-root") });
    assert.equal(candidates[0], webDist);
    const result = verifyFinalArtifactAnalytics({
      artifactRoot: root,
      expectedVersion: "1.2.3",
    });
    assert.equal(result.webDist, webDist);
  });
});

test("detectAnalyticsEnvironmentEvidence handles worker-style minified resolver names", () => {
  const text = [
    'const zi=t=>{const e=t==null?void 0:t.trim();return e||void 0}',
    'dd=(t,e,r)=>{const n=String(e).trim().toLowerCase();if(n==="development")return"staging";const s=zi(t)?.toLowerCase();return s==="production"?"production":s==="staging"?"staging":n==="production"&&zi(r)?"production":"staging"}',
    'hd=()=>dd("production","production","1.2.3")',
    'yd=(t,e={})=>({app_version:"1.2.3",analytics_environment:hd(),traffic_class:"user",...e})',
  ].join(",");
  const { evidence } = detectAnalyticsEnvironmentEvidence([{ relativePath: "assets/worker.js", text }]);
  assert.equal(evidence.length, 1);
  assert.equal(evidence[0].environment, "production");
  assert.equal(evidence[0].resolver, "hd");
});

test("CLI verifies an explicit web dist", () => {
  withTempDir("ctx-final-artifact-analytics-cli-", (root) => {
    writeFixtureDist(root, minifiedAnalyticsBundle("production"));
    const result = childProcess.spawnSync(
      "node",
      [scriptPath, "--web-dist", root, "--expected-version", "1.2.3"],
      { encoding: "utf8" },
    );
    assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
    assert.match(result.stdout, /ok: final artifact analytics verified/);
  });
});
