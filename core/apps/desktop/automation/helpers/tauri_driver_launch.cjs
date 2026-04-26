const buildNonDarwinTauriDriverLaunch = ({ platform, hasDisplay, port }) => {
  const resolvedPort = String(port || "").trim();
  if (!resolvedPort) {
    throw new Error("port is required");
  }
  if (platform === "linux" && !hasDisplay) {
    return {
      command: "xvfb-run",
      args: ["-a", "pnpm", "exec", "tauri-driver", "--port", resolvedPort],
    };
  }
  return {
    command: "pnpm",
    args: ["exec", "tauri-driver", "--port", resolvedPort],
  };
};

module.exports = {
  buildNonDarwinTauriDriverLaunch,
};
