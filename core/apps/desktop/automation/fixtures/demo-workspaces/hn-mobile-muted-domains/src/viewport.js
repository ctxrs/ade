export function resolveScreenHeight(metrics = {}) {
  const screenHeight = Number(metrics.screenHeight) || 0;
  const viewportHeight = Number(metrics.viewportHeight) || 0;
  const innerHeight = Number(metrics.innerHeight) || 0;
  return Math.max(screenHeight, viewportHeight, innerHeight, 0);
}
