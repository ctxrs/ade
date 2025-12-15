export const KNOWN_EFFORTS = ["low", "medium", "high", "xhigh"] as const;
export type KnownEffort = (typeof KNOWN_EFFORTS)[number];

export type ParsedModelId = {
  full: string;
  base: string;
  effort: KnownEffort | null;
};

export function parseModelId(fullModelId: string): ParsedModelId {
  const full = String(fullModelId || "").trim();
  if (!full) return { full: "", base: "", effort: null };
  const idx = full.lastIndexOf("/");
  if (idx <= 0) return { full, base: full, effort: null };
  const base = full.slice(0, idx);
  const suffix = full.slice(idx + 1);
  const effort = (KNOWN_EFFORTS as readonly string[]).includes(suffix) ? (suffix as KnownEffort) : null;
  return effort ? { full, base, effort } : { full, base: full, effort: null };
}

export type ModelCatalog = {
  baseIds: string[];
  displayNameByBase: Record<string, string>;
  effortsByBase: Record<string, KnownEffort[]>;
  fullIdByBaseEffort: Record<string, Partial<Record<KnownEffort, string>>>;
};

function normalizeBaseDisplayName(name: string): string {
  // Common ACP naming: `gpt-5.1-codex (low)` -> `gpt-5.1-codex`
  const trimmed = String(name || "").trim();
  const m = trimmed.match(/^(.*)\\s*\\((low|medium|high|xhigh)\\)\\s*$/i);
  return m ? m[1].trim() : trimmed;
}

export function buildModelCatalog(
  models: Array<{ id: string; name?: string }> | string[],
): ModelCatalog {
  const list = Array.isArray(models)
    ? models.map((m: any) => (typeof m === "string" ? { id: m, name: m } : { id: String(m.id), name: m.name }))
    : [];

  const baseIdsSet = new Set<string>();
  const displayNameByBase: Record<string, string> = {};
  const effortsByBase: Record<string, KnownEffort[]> = {};
  const fullIdByBaseEffort: Record<string, Partial<Record<KnownEffort, string>>> = {};

  for (const m of list) {
    const id = String(m.id || "").trim();
    if (!id) continue;
    const parsed = parseModelId(id);
    if (!parsed.base) continue;

    baseIdsSet.add(parsed.base);

    const baseNameCandidate = normalizeBaseDisplayName(m.name ?? id);
    if (!displayNameByBase[parsed.base] || (parsed.effort === "medium" && displayNameByBase[parsed.base] !== baseNameCandidate)) {
      displayNameByBase[parsed.base] = baseNameCandidate;
    }

    if (parsed.effort) {
      (fullIdByBaseEffort[parsed.base] ??= {})[parsed.effort] = parsed.full;
      const set = new Set<KnownEffort>(effortsByBase[parsed.base] ?? []);
      set.add(parsed.effort);
      effortsByBase[parsed.base] = KNOWN_EFFORTS.filter((e) => set.has(e));
    }
  }

  const baseIds = [...baseIdsSet].sort((a, b) => a.localeCompare(b));
  for (const b of baseIds) {
    if (!displayNameByBase[b]) displayNameByBase[b] = b;
  }

  return { baseIds, displayNameByBase, effortsByBase, fullIdByBaseEffort };
}

export function composeModelId(base: string, effort: KnownEffort | null): string {
  const b = String(base || "").trim();
  if (!b) return "";
  return effort ? `${b}/${effort}` : b;
}

