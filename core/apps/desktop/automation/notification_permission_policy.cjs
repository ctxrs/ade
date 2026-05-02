function resolveNotificationPermissionAction({ permission, ci = false } = {}) {
  const normalizedPermission = String(permission || "").trim().toLowerCase();
  if (normalizedPermission === "granted") {
    return { action: "proceed", normalizedPermission };
  }
  if (normalizedPermission === "default") {
    return {
      action: ci ? "proceed" : "request",
      normalizedPermission,
    };
  }
  return { action: "fail", normalizedPermission: normalizedPermission || "unknown" };
}

module.exports = {
  resolveNotificationPermissionAction,
};
