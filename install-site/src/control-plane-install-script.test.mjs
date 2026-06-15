import test from "node:test";
import assert from "node:assert/strict";
import { renderControlPlaneInstallScript } from "./control-plane-install-script.js";

test("control-plane installer resolves the product-specific release manifest", () => {
  const script = renderControlPlaneInstallScript({
    functionsBase: "https://api.example.test/functions/v1/",
    channel: "canary",
  });

  assert.match(script, /^#!\/bin\/sh/m);
  assert.match(script, /functions_base="\$\{CTX_CONTROL_PLANE_FUNCTIONS_BASE:-https:\/\/api\.example\.test\/functions\/v1\}"/);
  assert.match(script, /channel="\$\{CTX_CONTROL_PLANE_CHANNEL:-canary\}"/);
  assert.match(script, /manifest_url="\$\{functions_base%\/\}\/releases\/\$channel\/control-plane\/latest\.json"/);
});

test("control-plane installer installs ctx without editing shell profile PATH", () => {
  const script = renderControlPlaneInstallScript();

  assert.match(script, /install_root="\$\{CTX_CONTROL_PLANE_INSTALL_DIR:-\$HOME\/\.local\/share\/ctx-control-plane\}"/);
  assert.match(script, /bin_dir="\$\{CTX_CONTROL_PLANE_BIN_DIR:-\$HOME\/\.local\/bin\}"/);
  assert.match(script, /target_link="\$bin_dir\/ctx"/);
  assert.match(script, /Add \$bin_dir to PATH to run ctx from any shell\./);
  assert.doesNotMatch(script, />>\s*"\$HOME\/\.(bashrc|zshrc|profile)"/);
});
