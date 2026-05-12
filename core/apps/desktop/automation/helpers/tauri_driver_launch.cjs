const buildNonDarwinTauriDriverLaunch = ({ platform, hasDisplay, port, nativePort }) => {
  const resolvedPort = String(port || "").trim();
  if (!resolvedPort) {
    throw new Error("port is required");
  }
  const resolvedNativePort = String(nativePort || "").trim();
  if (!resolvedNativePort) {
    throw new Error("nativePort is required");
  }
  const portArgs = ["--port", resolvedPort, "--native-port", resolvedNativePort];
  if (platform === "linux" && !hasDisplay) {
    return {
      command: "xvfb-run",
      args: ["-a", "pnpm", "exec", "tauri-driver", ...portArgs],
    };
  }
  return {
    command: "pnpm",
    args: ["exec", "tauri-driver", ...portArgs],
  };
};

module.exports = {
  buildNonDarwinTauriDriverLaunch,
};
