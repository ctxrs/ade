import test from "node:test";
import assert from "node:assert/strict";
import { renderInstallScript } from "./install-script.js";

test("renderInstallScript emits a macOS bootstrap script with stable defaults", () => {
  const script = renderInstallScript();
  assert.match(script, /^#!\/bin\/sh/m);
  assert.match(script, /set -eu/);
  assert.match(script, /install_macos/);
  assert.match(script, /install_linux/);
  assert.match(script, /install_windows/);
  assert.match(script, /windows support is coming soon!/);
  assert.match(script, /https:\/\/api\.ctx\.rs\/functions\/v1/);
  assert.match(script, /channel="\$\{CTX_CHANNEL:-stable\}"/);
});

test("renderInstallScript includes release resolution, checksum verify, and app launch", () => {
  const script = renderInstallScript();
  assert.match(script, /releases\/\$channel\/latest\.json/);
  assert.match(script, /plutil -extract/);
  assert.match(script, /sha256sum/);
  assert.match(script, /shasum -a 256/);
  assert.match(script, /python3 - "\$manifest_json" "\$key"/);
  assert.match(script, /hdiutil attach/);
  assert.match(script, /ditto "\$app_src" "\$target_app"/);
  assert.match(script, /ctx\.AppImage/);
  assert.match(script, /ctx-desktop/);
  assert.match(script, /open "\$target_app"/);
  assert.match(script, /CTX_INSTALL_DIR/);
});

test("renderInstallScript allows overriding function base and channel", () => {
  const script = renderInstallScript({
    functionsBase: "https://example.test/functions/v1/",
    channel: "rc",
  });
  assert.match(script, /functions_base="\$\{CTX_FUNCTIONS_BASE:-https:\/\/example\.test\/functions\/v1\}"/);
  assert.match(script, /channel="\$\{CTX_CHANNEL:-rc\}"/);
});
