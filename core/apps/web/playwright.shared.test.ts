import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { createCtxPlaywrightConfig, resolvePlaywrightCargoTargetDir } from "./playwright.shared";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const ORIGINAL_ENV = { ...process.env };

type ReporterTuple = readonly [string, Record<string, unknown>?];

const restoreEnv = () => {
  process.env = { ...ORIGINAL_ENV };
  delete process.env.CTX_E2E_ARGOS;
  delete process.env.ARGOS_TOKEN;
  delete process.env.CTX_E2E_REPORTER;
  delete process.env.CTX_VOLATILE_ROOT;
  delete process.env.CTX_VOLATILE_TARGETS_DIR;
  delete process.env.CTX_VOLATILE_TMPDIR;
  delete process.env.CTX_E2E_TMPDIR;
  delete process.env.CTX_E2E_DATA_DIR;
};

const getReporterTuples = (reporter: unknown): ReporterTuple[] => {
  if (!Array.isArray(reporter)) return [];
  return reporter.filter((entry): entry is ReporterTuple => {
    return Array.isArray(entry) && typeof entry[0] === "string";
  });
};

afterEach(() => {
  restoreEnv();
});

describe("createCtxPlaywrightConfig", () => {
  it("writes reports under e2e artifact roots", async () => {
    restoreEnv();
    const config = await createCtxPlaywrightConfig("all");
    expect(config.outputDir).toBe(path.resolve(__dirname, "e2e/test-results/all"));
    expect(config.webServer?.url).toMatch(/^http:\/\/127\.0\.0\.1:\d+\/api\/health$/);

    const reporters = getReporterTuples(config.reporter);
    const htmlReporter = reporters.find((entry) => entry[0] === "html");
    expect(htmlReporter?.[1]).toMatchObject({
      outputFolder: path.resolve(__dirname, "e2e/playwright-report/all"),
      open: "never",
    });
  });

  it("does not add Argos by default", async () => {
    restoreEnv();
    const config = await createCtxPlaywrightConfig("premerge_required");
    const reporters = getReporterTuples(config.reporter);
    expect(reporters.map((entry) => entry[0])).not.toContain("@argos-ci/playwright/reporter");
  });

  it("adds the Argos reporter when an Argos token is present", async () => {
    restoreEnv();
    process.env.ARGOS_TOKEN = "test-token";
    const config = await createCtxPlaywrightConfig("premerge_required");
    const reporters = getReporterTuples(config.reporter);
    const argosReporter = reporters.find((entry) => entry[0] === "@argos-ci/playwright/reporter");
    expect(argosReporter).toBeTruthy();
    expect(argosReporter?.[1]).toMatchObject({
      uploadToArgos: true,
      buildName: "ctx-web-premerge_required",
    });
  });

  it("defaults e2e cargo builds to a stable cache dir instead of per-run temp dirs", async () => {
    restoreEnv();
    delete process.env.CTX_E2E_CARGO_TARGET_DIR;
    delete process.env.CARGO_TARGET_DIR;

    const resolved = resolvePlaywrightCargoTargetDir(process.env);
    expect(resolved).toContain(path.join(".ctx", "volatile", "targets", "ctx-e2e"));
    expect(resolved).toContain("e2e-");
    expect(resolved).not.toContain("ctx-e2e-cargo-");
  });

  it("threads the resolved cargo target dir into the webServer env", async () => {
    restoreEnv();
    const config = await createCtxPlaywrightConfig("all");
    const webServer = Array.isArray(config.webServer) ? config.webServer[0] : config.webServer;
    expect(webServer?.env?.CTX_E2E_CARGO_TARGET_DIR).toBe(resolvePlaywrightCargoTargetDir(process.env));
    expect(webServer?.env?.CARGO_TARGET_DIR).toBe(resolvePlaywrightCargoTargetDir(process.env));
  });

  it("defaults e2e tmp and data dirs under separate volatile tmp roots", async () => {
    restoreEnv();
    const config = await createCtxPlaywrightConfig("all");
    const webServer = Array.isArray(config.webServer) ? config.webServer[0] : config.webServer;
    const expectedTmpPrefix = path.join(".ctx", "volatile", "tmp", "ctx-e2e-all-tmp-");
    const expectedDataPrefix = path.join(".ctx", "volatile", "tmp", "ctx-e2e-all-data-");

    expect(String(webServer?.env?.CTX_E2E_TMPDIR)).toContain(expectedTmpPrefix);
    expect(String(webServer?.env?.CTX_E2E_DATA_DIR)).toContain(expectedDataPrefix);
    expect(webServer?.env?.CTX_E2E_TMPDIR).not.toBe(webServer?.env?.CTX_E2E_DATA_DIR);
    expect(webServer?.env?.CTX_VOLATILE_TMPDIR).toContain(path.join(".ctx", "volatile", "tmp"));
    expect(webServer?.env?.TMPDIR).toBe(webServer?.env?.CTX_E2E_TMPDIR);
    expect(webServer?.env?.TMP).toBe(webServer?.env?.CTX_E2E_TMPDIR);
    expect(webServer?.env?.TEMP).toBe(webServer?.env?.CTX_E2E_TMPDIR);
  });

  it("does not leak ambient desktop runtime paths into the e2e webServer env", async () => {
    restoreEnv();
    process.env.CTX_WEB_DIST = "/Applications/ctx.app/Contents/Resources/web/dist";
    process.env.CTX_BUNDLE_DIR = "/Applications/ctx.app/Contents/Resources/bundles";

    const config = await createCtxPlaywrightConfig("premerge_required");
    const webServer = Array.isArray(config.webServer) ? config.webServer[0] : config.webServer;

    expect(webServer?.env?.CTX_WEB_DIST).toBeUndefined();
    expect(webServer?.env?.CTX_BUNDLE_DIR).not.toBe("/Applications/ctx.app/Contents/Resources/bundles");
  });

  it("uses CTX_E2E_BUNDLE_DIR instead of ambient CTX_BUNDLE_DIR", async () => {
    restoreEnv();
    process.env.CTX_BUNDLE_DIR = "/Applications/ctx.app/Contents/Resources/bundles";
    process.env.CTX_E2E_BUNDLE_DIR = "/tmp/ctx-e2e-bundles";

    const config = await createCtxPlaywrightConfig("premerge_required");
    const webServer = Array.isArray(config.webServer) ? config.webServer[0] : config.webServer;

    expect(webServer?.env?.CTX_BUNDLE_DIR).toBe("/tmp/ctx-e2e-bundles");
  });
});
