export const SCROLLBACK_INCREASE_VIEWPORT_BY_PX = 240;

export function buildModelsFromAcpMeta(models: unknown): Array<{ id: string; name?: string }> {
  if (!models || typeof models !== "object") return [];
  const list = "models" in models ? models.models : undefined;
  if (!Array.isArray(list)) return [];
  const parsed: Array<{ id: string; name?: string }> = [];
  for (const item of list) {
    if (!item || typeof item !== "object") continue;
    const id = "id" in item && typeof item.id === "string" ? item.id : "";
    if (!id) continue;
    const name = "name" in item && typeof item.name === "string" ? item.name : undefined;
    parsed.push(name ? { id, name } : { id });
  }
  return parsed;
}

export function formatMemoryMb(value?: number | null): string {
  if (!Number.isFinite(value)) return "—";
  const mb = value as number;
  const gb = mb / 1024;
  if (gb >= 1) {
    const precision = gb >= 10 ? 0 : 1;
    return `${gb.toFixed(precision)} GB`;
  }
  return `${Math.round(mb)} MB`;
}

export function setBooleanStateRef(
  ref: { current: boolean },
  setState: (next: boolean) => void,
  next: boolean,
): void {
  ref.current = next;
  setState(next);
}
