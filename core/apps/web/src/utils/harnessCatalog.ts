export type HarnessCatalogEntry = {
  id: string;
  label: string;
  logoSrc: string;
  invertInDark?: boolean;
  invertInLight?: boolean;
};

// Curated list of popular harnesses shown in the selector. The text fallback avoids
// redistributing third-party brand artwork with the source release.
export const HARNESS_CATALOG: HarnessCatalogEntry[] = [
  { id: "claude-crp", label: "Claude Code", logoSrc: "" },
  { id: "codex", label: "Codex", logoSrc: "" },
  { id: "qwen", label: "Qwen Code", logoSrc: "" },
  { id: "cursor", label: "Cursor", logoSrc: "" },
  { id: "pi", label: "Pi", logoSrc: "" },
  { id: "droid", label: "Droid", logoSrc: "" },
  { id: "gemini", label: "Gemini", logoSrc: "" },
  { id: "goose", label: "Goose", logoSrc: "" },
  { id: "copilot", label: "Copilot", logoSrc: "" },
  { id: "opencode", label: "OpenCode", logoSrc: "" },
  { id: "openhands", label: "OpenHands", logoSrc: "" },
  { id: "cline", label: "Cline", logoSrc: "" },
  { id: "mistral", label: "Mistral Vibe", logoSrc: "" },
  { id: "auggie", label: "Auggie", logoSrc: "" },
  { id: "kimi", label: "Kimi", logoSrc: "" },
];

export function resolveHarnessCatalogId(providerId: string | null | undefined): string {
  return (providerId ?? "").trim();
}

export function findHarnessCatalogEntry(
  providerId: string | null | undefined,
): HarnessCatalogEntry | undefined {
  const catalogId = resolveHarnessCatalogId(providerId);
  return HARNESS_CATALOG.find((entry) => entry.id === catalogId);
}

export function buildHarnessCatalogEntryMap(): Map<string, HarnessCatalogEntry> {
  return new Map(HARNESS_CATALOG.map((entry) => [entry.id, entry]));
}

export const UNSUPPORTED_HARNESS_IDS = new Set([
  "codebuff",
  "charm",
  "aider",
  "kilo",
  "junie",
]);

export const HARNESS_LOGO_SRCS = Array.from(
  new Set(HARNESS_CATALOG.map((entry) => entry.logoSrc).filter(Boolean)),
);

let harnessLogosPreloaded = false;

export function preloadHarnessLogos() {
  if (harnessLogosPreloaded || typeof Image === "undefined") return;
  harnessLogosPreloaded = true;
  HARNESS_LOGO_SRCS.forEach((src) => {
    const img = new Image();
    img.src = src;
  });
}
