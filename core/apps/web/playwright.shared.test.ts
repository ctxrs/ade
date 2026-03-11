import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { createCtxPlaywrightConfig } from "./playwright.shared";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const ORIGINAL_ENV = { ...process.env };

type ReporterTuple = readonly [string, Record<string, unknown>?];

const restoreEnv = () => {
  process.env = { ...ORIGINAL_ENV };
  delete process.env.CTX_E2E_ARGOS;
  delete process.env.ARGOS_TOKEN;
  delete process.env.CTX_E2E_REPORTER;
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
});
