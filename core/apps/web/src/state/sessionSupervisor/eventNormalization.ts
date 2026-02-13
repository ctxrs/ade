export const readPayloadString = (
  payload: any,
  keys: string[],
): string | null => {
  if (!payload || typeof payload !== "object") return null;
  for (const key of keys) {
    const value = (payload as any)?.[key];
    if (typeof value === "string" && value.trim()) return value;
  }
  return null;
};

export const pickFirstString = (...values: any[]): string | null => {
  for (const v of values) {
    if (typeof v === "string" && v.trim()) return v.trim();
  }
  return null;
};
