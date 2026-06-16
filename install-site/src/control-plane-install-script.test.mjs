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
  assert.match(script, /PYTHONHOME= PYTHONPATH= python3/);
});

test("control-plane installer installs ctx without editing shell profile PATH", () => {
  const script = renderControlPlaneInstallScript();

  assert.match(script, /install_root="\$\{CTX_CONTROL_PLANE_INSTALL_DIR:-\$HOME\/\.local\/share\/ctx-control-plane\}"/);
  assert.match(script, /bin_dir="\$\{CTX_CONTROL_PLANE_BIN_DIR:-\$HOME\/\.local\/bin\}"/);
  assert.match(script, /target_link="\$bin_dir\/ctx"/);
  assert.match(script, /Add \$bin_dir to PATH to run ctx from any shell\./);
  assert.doesNotMatch(script, />>\s*"\$HOME\/\.(bashrc|zshrc|profile)"/);
});

test("control-plane installer supports windows-x64 zip archives from Git Bash", () => {
  const script = renderControlPlaneInstallScript();

  assert.match(script, /MINGW\*:x86_64\|MSYS\*:x86_64\|CYGWIN\*:x86_64\).*"windows-x64"/);
  assert.match(script, /archive_format="\$\(extract_manifest_field "platforms\.\$platform\.cli\.archive_format"\)"/);
  assert.match(script, /zip\)\s+need_cmd unzip[\s\S]*archive_path="\$tmp_dir\/ctx-control-plane\.zip"/);
  assert.match(script, /zip\) unzip -q "\$archive_path" -d "\$extract_dir" ;;/);
  assert.match(script, /windows-\*\) executable_name="ctx\.exe" ;;/);
  assert.match(script, /windows-\*\) target_link="\$bin_dir\/ctx\.exe" ;;/);
});
