const { withAppDelegate, createRunOncePlugin } = require("@expo/config-plugins");

function ensureImport(contents, importLine) {
  if (contents.includes(importLine)) {
    return contents;
  }
  const lines = contents.split("\n");
  let insertIndex = 0;
  for (let i = 0; i < lines.length; i += 1) {
    if (lines[i].startsWith("import ")) {
      insertIndex = i + 1;
    }
  }
  lines.splice(insertIndex, 0, importLine);
  return lines.join("\n");
}

function withDisableDevLoadingView(config) {
  return withAppDelegate(config, (config) => {
    if (config.modResults.language !== "swift") {
      return config;
    }

    let contents = config.modResults.contents;
    contents = ensureImport(contents, "import React");

    const launchNeedle = "let delegate = ReactNativeDelegate()";
    if (contents.includes(launchNeedle) && !contents.includes("RCTDevLoadingView.setEnabled(false)")) {
      const block = ["#if DEBUG", "    RCTDevLoadingView.setEnabled(false)", "#endif"].join("\n");
      contents = contents.replace(launchNeedle, `${block}\n    ${launchNeedle}`);
    }

    config.modResults.contents = contents;
    return config;
  });
}

module.exports = createRunOncePlugin(
  withDisableDevLoadingView,
  "with-disable-dev-loading-view",
  "1.0.0"
);
