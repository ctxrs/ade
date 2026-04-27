import { describe, expect, it } from "vitest";
import {
  buildHarnessCatalogEntryMap,
  findHarnessCatalogEntry,
  HARNESS_CATALOG,
  resolveHarnessCatalogId,
} from "./harnessCatalog";

describe("harnessCatalog", () => {
  it("maps Pi to the official pi.dev logo asset", () => {
    const pi = HARNESS_CATALOG.find((entry) => entry.id === "pi");
    expect(pi).toBeDefined();
    expect(pi?.logoSrc).toContain("pi.svg");
    expect(pi?.invertInLight).toBe(true);
  });

  it("omits unsupported harnesses from the curated catalog", () => {
    const ids = new Set(HARNESS_CATALOG.map((entry) => entry.id));
    expect(ids.has("codebuff")).toBe(false);
    expect(ids.has("charm")).toBe(false);
    expect(ids.has("aider")).toBe(false);
    expect(ids.has("kilo")).toBe(false);
    expect(ids.has("junie")).toBe(false);
    expect(ids.has("goose")).toBe(true);
    expect(ids.has("openhands")).toBe(true);
  });

  it("uses product identity for Codex adapter ids", () => {
    expect(resolveHarnessCatalogId("codex-crp")).toBe("codex");
    const codex = findHarnessCatalogEntry("codex-crp");
    expect(codex?.id).toBe("codex");
    expect(codex?.label).toBe("Codex");
    expect(codex?.logoSrc).toContain("openai");
  });

  it("indexes adapter aliases for UI lookup maps", () => {
    const byProviderId = buildHarnessCatalogEntryMap();
    expect(byProviderId.get("codex-crp")).toBe(byProviderId.get("codex"));
    expect(byProviderId.get("codex-crp")?.logoSrc).toContain("openai");
  });
});
