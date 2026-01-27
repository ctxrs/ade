export function isAppInForeground(): boolean {
  if (typeof document === "undefined") return true;
  const visibility = document.visibilityState;
  if (visibility && visibility !== "visible") return false;
  if (typeof document.hasFocus === "function") {
    return document.hasFocus();
  }
  return true;
}
