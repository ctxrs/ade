import { describe, expect, it } from "vitest";
import type { PluginExtensionRegistry } from "@ctx/types";
import {
  isWorkbenchPluginTemplateId,
  parseWorkbenchPluginTemplateId,
  projectPluginWorkbenchContributions,
  projectWorkbenchContributionProjection,
  toWorkbenchPluginTemplateId,
} from "./pluginWorkbenchContributionProjection";

describe("pluginWorkbenchContributionProjection", () => {
  it("projects plugin UI surfaces into source-labeled Workbench candidates", () => {
    const registry: PluginExtensionRegistry = {
      revision: 7,
      ui_surfaces: [
        {
          plugin_id: "zeta.tools",
          plugin_name: "Zeta Tools",
          plugin_version: "0.2.0",
          plugin_path: "/plugins/zeta/ctx-plugin.json",
          plugin_revision: "rev-z",
          contribution: {
            id: "review",
            name: "Review Panel",
            surface: "panel",
            description: "Shows review context.",
            entrypoint: "main",
            contexts: ["review", "review", "  change-set  ", ""],
          },
        },
        {
          plugin_id: "alpha.tools",
          plugin_name: "Alpha Tools",
          plugin_version: "0.1.0",
          plugin_path: "/plugins/alpha/ctx-plugin.json",
          plugin_revision: "rev-a",
          contribution: {
            id: "status",
            name: "Status Strip",
            surface: "status_bar",
          },
        },
      ],
    };

    expect(projectPluginWorkbenchContributions(registry)).toEqual([
      {
        id: "plugin:zeta.tools/review",
        contributionId: "review",
        title: "Review Panel",
        description: "Shows review context.",
        surface: "panel",
        contexts: ["review", "change-set"],
        entrypoint: "main",
        source: {
          kind: "plugin",
          pluginId: "zeta.tools",
          pluginName: "Zeta Tools",
          pluginVersion: "0.2.0",
          pluginPath: "/plugins/zeta/ctx-plugin.json",
          pluginRevision: "rev-z",
        },
        compatibility: { kind: "compatible" },
      },
      {
        id: "plugin:alpha.tools/status",
        contributionId: "status",
        title: "Status Strip",
        description: null,
        surface: "status_bar",
        contexts: [],
        entrypoint: null,
        source: {
          kind: "plugin",
          pluginId: "alpha.tools",
          pluginName: "Alpha Tools",
          pluginVersion: "0.1.0",
          pluginPath: "/plugins/alpha/ctx-plugin.json",
          pluginRevision: "rev-a",
        },
        compatibility: { kind: "compatible" },
      },
    ]);
  });

  it("filters malformed records and marks non-Workbench surfaces as unsupported", () => {
    const registry: PluginExtensionRegistry = {
      revision: 1,
      ui_surfaces: [
        {
          plugin_id: "broken.tools",
          plugin_name: "Broken Tools",
          plugin_version: "0.1.0",
          plugin_path: "/plugins/broken/ctx-plugin.json",
          contribution: {
            id: "broken",
            name: "   ",
            surface: "panel",
          },
        },
        {
          plugin_id: "palette.tools",
          plugin_name: "Palette Tools",
          plugin_version: "0.1.0",
          plugin_path: "/plugins/palette/ctx-plugin.json",
          contribution: {
            id: "search",
            name: "Search Command",
            surface: "command_palette",
          },
        },
      ],
    };

    expect(projectPluginWorkbenchContributions(registry)).toEqual([
      expect.objectContaining({
        id: "plugin:palette.tools/search",
        compatibility: { kind: "unsupported_surface", surface: "command_palette" },
      }),
    ]);
  });

  it("resolves loading, empty, error, ready, and fallback projection states", () => {
    expect(projectWorkbenchContributionProjection({ loadState: { kind: "loading" } })).toMatchObject({
      kind: "loading",
      candidates: [],
      activeCandidate: null,
      fallback: null,
      effectiveTemplateId: "classic",
    });

    expect(projectWorkbenchContributionProjection({ loadState: { kind: "ready" } })).toEqual({
      kind: "empty",
      candidates: [],
      activeCandidate: null,
      fallback: null,
      effectiveTemplateId: "classic",
    });

    expect(
      projectWorkbenchContributionProjection({
        loadState: { kind: "error", message: "registry unavailable" },
        activeTemplateId: "plugin:removed.tools/review",
      }),
    ).toMatchObject({
      kind: "error",
      message: "registry unavailable",
      fallback: {
        kind: "unavailable",
        requestedTemplateId: "plugin:removed.tools/review",
        fallbackTemplateId: "classic",
        reason: "error",
      },
      effectiveTemplateId: "classic",
    });
  });

  it("keeps a compatible active plugin template selected as data only", () => {
    const registry: PluginExtensionRegistry = {
      revision: 1,
      ui_surfaces: [
        {
          plugin_id: "review/tools",
          plugin_name: "Review Tools",
          plugin_version: "0.1.0",
          plugin_path: "/plugins/review/ctx-plugin.json",
          contribution: {
            id: "panel/main",
            name: "Review Panel",
            surface: "panel",
            entrypoint: "main",
          },
        },
      ],
    };

    const projection = projectWorkbenchContributionProjection({
      loadState: { kind: "ready" },
      registry,
      activeTemplateId: "plugin:review%2Ftools/panel%2Fmain",
    });

    expect(projection).toMatchObject({
      kind: "ready",
      effectiveTemplateId: "plugin:review%2Ftools/panel%2Fmain",
      fallback: null,
      activeCandidate: {
        id: "plugin:review%2Ftools/panel%2Fmain",
        entrypoint: "main",
      },
    });
  });

  it("falls back when a persisted plugin template no longer exists in the registry", () => {
    const projection = projectWorkbenchContributionProjection({
      loadState: { kind: "ready" },
      registry: { revision: 3, ui_surfaces: [] },
      activeTemplateId: "plugin:removed%2Ftools/review%2Fpanel",
    });

    expect(projection).toEqual({
      kind: "fallback",
      candidates: [],
      activeCandidate: null,
      fallback: {
        kind: "removed_plugin",
        requestedTemplateId: "plugin:removed%2Ftools/review%2Fpanel",
        fallbackTemplateId: "classic",
        pluginId: "removed/tools",
        contributionId: "review/panel",
      },
      effectiveTemplateId: "classic",
    });
  });

  it("falls back when the active plugin surface is not Workbench-compatible", () => {
    const registry: PluginExtensionRegistry = {
      revision: 1,
      ui_surfaces: [
        {
          plugin_id: "settings.tools",
          plugin_name: "Settings Tools",
          plugin_version: "0.1.0",
          plugin_path: "/plugins/settings/ctx-plugin.json",
          contribution: {
            id: "prefs",
            name: "Plugin Preferences",
            surface: "settings",
          },
        },
      ],
    };

    expect(
      projectWorkbenchContributionProjection({
        loadState: { kind: "ready" },
        registry,
        activeTemplateId: "plugin:settings.tools/prefs",
      }),
    ).toMatchObject({
      kind: "fallback",
      fallback: {
        kind: "unavailable",
        reason: "incompatible",
        requestedTemplateId: "plugin:settings.tools/prefs",
      },
      effectiveTemplateId: "classic",
    });
  });

  it("round-trips plugin-qualified Workbench IDs", () => {
    const id = toWorkbenchPluginTemplateId("review/tools", "panel/main");

    expect(id).toBe("plugin:review%2Ftools/panel%2Fmain");
    expect(isWorkbenchPluginTemplateId(id)).toBe(true);
    expect(parseWorkbenchPluginTemplateId(id)).toEqual({
      pluginId: "review/tools",
      contributionId: "panel/main",
    });
  });

  it("rejects malformed plugin-qualified Workbench IDs", () => {
    expect(isWorkbenchPluginTemplateId("plugin:review.tools/panel/extra")).toBe(false);
    expect(isWorkbenchPluginTemplateId("plugin:review.tools")).toBe(false);
    expect(isWorkbenchPluginTemplateId("plugin:%E0%A4%A/panel")).toBe(false);
  });
});
