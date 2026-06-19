import type {
  PluginContributionRegistration,
  PluginExtensionRegistry,
  PluginUiSurfaceContribution,
  PluginUiSurfaceKind,
} from "@ctx/types";
import type {
  WorkbenchBuiltinTemplateId,
  WorkbenchPluginTemplateId,
  WorkbenchTemplateId,
} from "../../workbench/types";

export type WorkbenchContributionProjectionLoadState =
  | { kind: "loading" }
  | { kind: "error"; message: string }
  | { kind: "ready" };

export type WorkbenchContributionCompatibility =
  | { kind: "compatible" }
  | { kind: "unsupported_surface"; surface: PluginUiSurfaceKind }
  | { kind: "invalid"; reasons: string[] };

export type WorkbenchContributionSource = {
  kind: "plugin";
  pluginId: string;
  pluginName: string;
  pluginVersion: string;
  pluginPath: string;
  pluginRevision: string | null;
};

export type WorkbenchContributionCandidate = {
  id: WorkbenchPluginTemplateId;
  contributionId: string;
  title: string;
  description: string | null;
  surface: PluginUiSurfaceKind;
  contexts: string[];
  entrypoint: string | null;
  source: WorkbenchContributionSource;
  compatibility: WorkbenchContributionCompatibility;
};

export type WorkbenchContributionFallback =
  | {
      kind: "removed_plugin";
      requestedTemplateId: WorkbenchPluginTemplateId;
      fallbackTemplateId: WorkbenchBuiltinTemplateId;
      pluginId: string;
      contributionId: string;
    }
  | {
      kind: "unavailable";
      requestedTemplateId: WorkbenchPluginTemplateId;
      fallbackTemplateId: WorkbenchBuiltinTemplateId;
      reason: "loading" | "error" | "incompatible";
    };

export type WorkbenchContributionProjection =
  | {
      kind: "loading";
      candidates: WorkbenchContributionCandidate[];
      activeCandidate: null;
      fallback: WorkbenchContributionFallback | null;
      effectiveTemplateId: WorkbenchTemplateId;
    }
  | {
      kind: "error";
      message: string;
      candidates: WorkbenchContributionCandidate[];
      activeCandidate: null;
      fallback: WorkbenchContributionFallback | null;
      effectiveTemplateId: WorkbenchTemplateId;
    }
  | {
      kind: "empty";
      candidates: [];
      activeCandidate: null;
      fallback: null;
      effectiveTemplateId: WorkbenchTemplateId;
    }
  | {
      kind: "ready";
      candidates: WorkbenchContributionCandidate[];
      activeCandidate: WorkbenchContributionCandidate | null;
      fallback: WorkbenchContributionFallback | null;
      effectiveTemplateId: WorkbenchTemplateId;
    }
  | {
      kind: "fallback";
      candidates: WorkbenchContributionCandidate[];
      activeCandidate: null;
      fallback: WorkbenchContributionFallback;
      effectiveTemplateId: WorkbenchBuiltinTemplateId;
    };

export type ProjectWorkbenchContributionProjectionOptions = {
  loadState: WorkbenchContributionProjectionLoadState;
  registry?: PluginExtensionRegistry | null;
  activeTemplateId?: WorkbenchTemplateId | null;
  fallbackTemplateId?: WorkbenchBuiltinTemplateId;
};

const WORKBENCH_COMPATIBLE_SURFACES = new Set<PluginUiSurfaceKind>([
  "panel",
  "sidebar",
  "status_bar",
]);

const DEFAULT_FALLBACK_TEMPLATE_ID: WorkbenchBuiltinTemplateId = "classic";

const normalizeText = (value: string | null | undefined): string => String(value ?? "").trim();

const normalizeStringList = (values: readonly string[] | null | undefined): string[] => {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const value of values ?? []) {
    const normalized = normalizeText(value);
    if (!normalized || seen.has(normalized)) continue;
    seen.add(normalized);
    out.push(normalized);
  }
  return out;
};

const candidateSortKey = (entry: WorkbenchContributionCandidate): string =>
  [
    entry.compatibility.kind === "compatible" ? "0" : "1",
    entry.surface,
    entry.title.toLowerCase(),
    entry.source.pluginName.toLowerCase(),
    entry.id.toLowerCase(),
  ].join("\0");

const PLUGIN_TEMPLATE_ID_PREFIX = "plugin:";
const PLUGIN_TEMPLATE_ID_SEPARATOR = "/";

const encodePluginTemplateIdPart = (value: string): string => encodeURIComponent(value);

const decodePluginTemplateIdPart = (value: string): string | null => {
  try {
    return decodeURIComponent(value);
  } catch {
    return null;
  }
};

const parsePluginTemplateIdParts = (
  value: WorkbenchTemplateId | string,
): { pluginId: string; contributionId: string } | null => {
  if (!value.startsWith(PLUGIN_TEMPLATE_ID_PREFIX)) return null;

  const body = value.slice(PLUGIN_TEMPLATE_ID_PREFIX.length);
  const separator = body.indexOf(PLUGIN_TEMPLATE_ID_SEPARATOR);
  if (separator <= 0 || separator !== body.lastIndexOf(PLUGIN_TEMPLATE_ID_SEPARATOR) || separator === body.length - 1) {
    return null;
  }

  const pluginId = decodePluginTemplateIdPart(body.slice(0, separator));
  const contributionId = decodePluginTemplateIdPart(body.slice(separator + 1));
  if (!pluginId || !contributionId) return null;

  return { pluginId, contributionId };
};

export const isWorkbenchPluginTemplateId = (value: WorkbenchTemplateId | string): value is WorkbenchPluginTemplateId =>
  parsePluginTemplateIdParts(value) !== null;

export const parseWorkbenchPluginTemplateId = (
  value: WorkbenchPluginTemplateId,
): { pluginId: string; contributionId: string } => {
  const parsed = parsePluginTemplateIdParts(value);
  if (!parsed) throw new Error(`Invalid Workbench plugin template ID: ${value}`);
  return parsed;
};

export const toWorkbenchPluginTemplateId = (
  pluginId: string,
  contributionId: string,
): WorkbenchPluginTemplateId =>
  `${PLUGIN_TEMPLATE_ID_PREFIX}${encodePluginTemplateIdPart(pluginId)}${PLUGIN_TEMPLATE_ID_SEPARATOR}${encodePluginTemplateIdPart(
    contributionId,
  )}`;

export const projectPluginWorkbenchContributions = (
  registry: PluginExtensionRegistry,
): WorkbenchContributionCandidate[] => {
  const candidates: WorkbenchContributionCandidate[] = [];
  for (const registration of registry.ui_surfaces ?? []) {
    const candidate = projectPluginWorkbenchContribution(registration);
    if (candidate) candidates.push(candidate);
  }
  return candidates.sort((left, right) => candidateSortKey(left).localeCompare(candidateSortKey(right)));
};

const projectPluginWorkbenchContribution = (
  registration: PluginContributionRegistration<PluginUiSurfaceContribution>,
): WorkbenchContributionCandidate | null => {
  const contributionId = normalizeText(registration.contribution.id);
  const pluginId = normalizeText(registration.plugin_id);
  const title = normalizeText(registration.contribution.name);
  const surface = registration.contribution.surface;
  if (!contributionId || !pluginId || !title || !surface) return null;

  const reasons: string[] = [];
  if (!normalizeText(registration.plugin_name)) reasons.push("missing_plugin_name");
  if (!normalizeText(registration.plugin_version)) reasons.push("missing_plugin_version");

  const compatibility: WorkbenchContributionCompatibility = reasons.length
    ? { kind: "invalid", reasons }
    : WORKBENCH_COMPATIBLE_SURFACES.has(surface)
      ? { kind: "compatible" }
      : { kind: "unsupported_surface", surface };

  return {
    id: toWorkbenchPluginTemplateId(pluginId, contributionId),
    contributionId,
    title,
    description: normalizeText(registration.contribution.description) || null,
    surface,
    contexts: normalizeStringList(registration.contribution.contexts),
    entrypoint: normalizeText(registration.contribution.entrypoint) || null,
    source: {
      kind: "plugin",
      pluginId,
      pluginName: normalizeText(registration.plugin_name) || pluginId,
      pluginVersion: normalizeText(registration.plugin_version),
      pluginPath: normalizeText(registration.plugin_path),
      pluginRevision: normalizeText(registration.plugin_revision) || null,
    },
    compatibility,
  };
};

export const projectWorkbenchContributionProjection = ({
  loadState,
  registry,
  activeTemplateId = "classic",
  fallbackTemplateId = DEFAULT_FALLBACK_TEMPLATE_ID,
}: ProjectWorkbenchContributionProjectionOptions): WorkbenchContributionProjection => {
  const candidates = registry ? projectPluginWorkbenchContributions(registry) : [];
  const requestedTemplateId = activeTemplateId ?? fallbackTemplateId;
  const pluginTemplateRequested = isWorkbenchPluginTemplateId(requestedTemplateId);

  if (loadState.kind === "loading") {
    const fallback = pluginTemplateRequested
      ? buildUnavailableFallback(requestedTemplateId, fallbackTemplateId, "loading")
      : null;
    return {
      kind: "loading",
      candidates,
      activeCandidate: null,
      fallback,
      effectiveTemplateId: fallback ? fallback.fallbackTemplateId : requestedTemplateId,
    };
  }

  if (loadState.kind === "error") {
    const fallback = pluginTemplateRequested
      ? buildUnavailableFallback(requestedTemplateId, fallbackTemplateId, "error")
      : null;
    return {
      kind: "error",
      message: loadState.message,
      candidates,
      activeCandidate: null,
      fallback,
      effectiveTemplateId: fallback ? fallback.fallbackTemplateId : requestedTemplateId,
    };
  }

  if (candidates.length === 0 && !pluginTemplateRequested) {
    return {
      kind: "empty",
      candidates: [],
      activeCandidate: null,
      fallback: null,
      effectiveTemplateId: requestedTemplateId,
    };
  }

  const activeCandidate = pluginTemplateRequested
    ? candidates.find((candidate) => candidate.id === requestedTemplateId) ?? null
    : null;
  if (!pluginTemplateRequested || activeCandidate?.compatibility.kind === "compatible") {
    return {
      kind: "ready",
      candidates,
      activeCandidate,
      fallback: null,
      effectiveTemplateId: requestedTemplateId,
    };
  }

  const fallback = activeCandidate
    ? buildUnavailableFallback(requestedTemplateId, fallbackTemplateId, "incompatible")
    : buildRemovedPluginFallback(requestedTemplateId, fallbackTemplateId);
  return {
    kind: "fallback",
    candidates,
    activeCandidate: null,
    fallback,
    effectiveTemplateId: fallbackTemplateId,
  };
};

const buildRemovedPluginFallback = (
  requestedTemplateId: WorkbenchPluginTemplateId,
  fallbackTemplateId: WorkbenchBuiltinTemplateId,
): WorkbenchContributionFallback => {
  const { pluginId, contributionId } = parseWorkbenchPluginTemplateId(requestedTemplateId);
  return {
    kind: "removed_plugin",
    requestedTemplateId,
    fallbackTemplateId,
    pluginId,
    contributionId,
  };
};

const buildUnavailableFallback = (
  requestedTemplateId: WorkbenchPluginTemplateId,
  fallbackTemplateId: WorkbenchBuiltinTemplateId,
  reason: "loading" | "error" | "incompatible",
): WorkbenchContributionFallback => ({
  kind: "unavailable",
  requestedTemplateId,
  fallbackTemplateId,
  reason,
});
