import { describe, expect, it } from "vitest";
import {
  buildHarnessCatalogEntryMap,
  findHarnessCatalogEntry,
  HARNESS_CATALOG,
  resolveHarnessCatalogId,
} from "./harnessCatalog";

describe("harnessCatalog", () => {
  it("uses text fallbacks instead of redistributed brand artwork", () => {
    const pi = HARNESS_CATALOG.find((entry) => entry.id === "pi");
    expect(pi).toBeDefined();
    expect(pi?.logoSrc).toBe("");
    expect(HARNESS_CATALOG.every((entry) => entry.logoSrc === "")).toBe(true);
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

  it("uses product identity for Codex", () => {
    expect(resolveHarnessCatalogId("codex")).toBe("codex");
    const codex = findHarnessCatalogEntry("codex");
    expect(codex?.id).toBe("codex");
    expect(codex?.label).toBe("Codex");
    expect(codex?.logoSrc).toBe("");
  });

  it("does not alias adapter implementation ids into provider ids", () => {
    const byProviderId = buildHarnessCatalogEntryMap();
    expect(byProviderId.get("codex-crp")).toBeUndefined();
    expect(findHarnessCatalogEntry("codex-crp")).toBeUndefined();
  });
});
