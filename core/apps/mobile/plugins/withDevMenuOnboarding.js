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

function withDevMenuOnboarding(config) {
  return withAppDelegate(config, (config) => {
    if (config.modResults.language !== "swift") {
      return config;
    }

    let contents = config.modResults.contents;
    contents = ensureImport(contents, "import EXDevMenu");

    const interceptorClass = [
      "#if DEBUG",
      "final class DevMenuOnboardingInterceptor: NSObject, DevMenuTestInterceptor {",
      "  @objc var shouldShowAtLaunch: Bool { false }",
      "  @objc var isOnboardingFinishedKey: Bool { true }",
      "}",
      "#endif",
      "",
    ].join("\n");

    if (!contents.includes("DevMenuOnboardingInterceptor")) {
      const importEndIndex = contents.lastIndexOf("import ");
      if (importEndIndex !== -1) {
        const nextLineBreak = contents.indexOf("\n", importEndIndex);
        const insertAt = nextLineBreak === -1 ? contents.length : nextLineBreak + 1;
        contents = contents.slice(0, insertAt) + interceptorClass + contents.slice(insertAt);
      } else {
        contents = interceptorClass + contents;
      }
    }

    const launchNeedle = "let delegate = ReactNativeDelegate()";
    if (contents.includes(launchNeedle) && !contents.includes("DevMenuTestInterceptorManager")) {
      const block = [
        "#if DEBUG",
        "    DevMenuTestInterceptorManager.setTestInterceptor(DevMenuOnboardingInterceptor())",
        "#endif",
      ].join("\n");
      contents = contents.replace(launchNeedle, `${block}\n    ${launchNeedle}`);
    }

    config.modResults.contents = contents;
    return config;
  });
}

module.exports = createRunOncePlugin(withDevMenuOnboarding, "with-dev-menu-onboarding", "1.0.0");
