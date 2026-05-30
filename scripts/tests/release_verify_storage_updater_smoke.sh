#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

tmp="$(mktemp -d /tmp/ctx-release-verify-updater.XXXXXX)"
server_pid=""

cleanup() {
  if [[ -n "$server_pid" ]]; then
    kill "$server_pid" >/dev/null 2>&1 || true
    wait "$server_pid" >/dev/null 2>&1 || true
  fi
  rm -rf "$tmp"
}
trap cleanup EXIT

mac_artifact_name="ctx_0.4.99_macos-arm64_updater.app.tar.gz"
mac_artifact_path="$tmp/$mac_artifact_name"
mac_desktop_name="ctx_0.4.99_macos-arm64.dmg"
mac_desktop_path="$tmp/$mac_desktop_name"
mac_daemon_name="ctx_0.4.99_macos-arm64"
mac_daemon_path="$tmp/$mac_daemon_name"
linux_appimage_name="ctx_0.4.99_linux-x64_updater.AppImage"
linux_appimage_path="$tmp/$linux_appimage_name"
linux_daemon_name="ctx_0.4.99_linux-x64"
linux_daemon_path="$tmp/$linux_daemon_name"
port_file="$tmp/port.txt"
pubkey_file="$tmp/updater.pub"
pubkey_b64_file="$tmp/updater.pub.b64"
private_key_pem_file="$tmp/updater-private.pem"
key_id_hex_file="$tmp/updater-key-id.hex"
mac_signature_b64_file="$tmp/macos.signature.b64"
linux_appimage_signature_b64_file="$tmp/linux-appimage.signature.b64"
latest_json="$tmp/latest.json"
latest_json_sig="$tmp/latest.json.sig"
latest_tauri_json="$tmp/latest-tauri.json"
version_json="$tmp/0.4.99.json"
version_json_sig="$tmp/0.4.99.json.sig"
version_tauri_json="$tmp/0.4.99-tauri.json"
storage_override_channel="stable-dry-run-verify"
storage_latest_json="$tmp/latest-storage.json"
storage_latest_json_sig="$tmp/latest-storage.json.sig"
storage_latest_tauri_json="$tmp/latest-storage-tauri.json"
storage_version_json="$tmp/0.4.99-storage.json"
storage_version_json_sig="$tmp/0.4.99-storage.json.sig"
storage_version_tauri_json="$tmp/0.4.99-storage-tauri.json"
linux_get_marker="$tmp/linux-storage-get-requested"

printf 'mac updater bytes for release verify smoke\n' >"$mac_artifact_path"
printf 'mac desktop bytes\n' >"$mac_desktop_path"
printf 'mac daemon bytes\n' >"$mac_daemon_path"
printf 'linux appimage bytes\n' >"$linux_appimage_path"
printf 'linux daemon bytes\n' >"$linux_daemon_path"

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

mac_desktop_sha="$(sha256_file "$mac_desktop_path")"
mac_daemon_sha="$(sha256_file "$mac_daemon_path")"
linux_appimage_sha="$(sha256_file "$linux_appimage_path")"
linux_daemon_sha="$(sha256_file "$linux_daemon_path")"

MAC_ARTIFACT_PATH="$mac_artifact_path" \
LINUX_APPIMAGE_ARTIFACT_PATH="$linux_appimage_path" \
PUBKEY_FILE="$pubkey_file" \
PUBKEY_B64_FILE="$pubkey_b64_file" \
PRIVATE_KEY_PEM_FILE="$private_key_pem_file" \
KEY_ID_HEX_FILE="$key_id_hex_file" \
MAC_SIGNATURE_B64_FILE="$mac_signature_b64_file" \
LINUX_APPIMAGE_SIGNATURE_B64_FILE="$linux_appimage_signature_b64_file" \
node - <<'NODE'
const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");

function b64UrlToBuffer(value) {
  const normalized = String(value).replace(/-/g, "+").replace(/_/g, "/");
  const padded = normalized + "=".repeat((4 - (normalized.length % 4)) % 4);
  return Buffer.from(padded, "base64");
}

const { publicKey, privateKey } = crypto.generateKeyPairSync("ed25519");
const publicJwk = publicKey.export({ format: "jwk" });
const rawPublicKey = b64UrlToBuffer(publicJwk.x);
const keyId = crypto.randomBytes(8);
fs.writeFileSync(process.env.PRIVATE_KEY_PEM_FILE, privateKey.export({ format: "pem", type: "pkcs8" }), "utf8");
fs.writeFileSync(process.env.KEY_ID_HEX_FILE, keyId.toString("hex"), "utf8");

const publicKeyText =
  "untrusted comment: minisign public key: 0D503F73CDD77B9C\n" +
  `${Buffer.concat([Buffer.from([0x45, 0x64]), keyId, rawPublicKey]).toString("base64")}\n`;

fs.writeFileSync(process.env.PUBKEY_FILE, publicKeyText, "utf8");
fs.writeFileSync(
  process.env.PUBKEY_B64_FILE,
  Buffer.from(publicKeyText, "utf8").toString("base64"),
  "utf8",
);

const signArtifact = (artifactPath, signatureOutPath) => {
  const artifactName = path.basename(artifactPath);
  const artifactBytes = fs.readFileSync(artifactPath);
  const trustedComment = `trusted comment: timestamp:1772585616\tfile:${artifactName}`;
  const signatureBytes = crypto.sign(
    null,
    crypto.createHash("blake2b512").update(artifactBytes).digest(),
    privateKey,
  );
  const signatureText =
    "untrusted comment: signature from minisign secret key\n" +
    `${Buffer.concat([Buffer.from([0x45, 0x44]), keyId, signatureBytes]).toString("base64")}\n` +
    `${trustedComment}\n` +
    `${crypto
      .sign(
        null,
        Buffer.concat([
          signatureBytes,
          Buffer.from(trustedComment.slice("trusted comment: ".length), "utf8"),
        ]),
        privateKey,
      )
      .toString("base64")}\n`;
  fs.writeFileSync(signatureOutPath, Buffer.from(signatureText, "utf8").toString("base64"), "utf8");
};

signArtifact(process.env.MAC_ARTIFACT_PATH, process.env.MAC_SIGNATURE_B64_FILE);
signArtifact(process.env.LINUX_APPIMAGE_ARTIFACT_PATH, process.env.LINUX_APPIMAGE_SIGNATURE_B64_FILE);
NODE

pubkey_b64="$(tr -d '\r\n' < "$pubkey_b64_file")"
mac_signature_b64="$(tr -d '\r\n' < "$mac_signature_b64_file")"
linux_appimage_signature_b64="$(tr -d '\r\n' < "$linux_appimage_signature_b64_file")"

sign_manifest_file() {
  local manifest_path="$1"
  local signature_path="$2"
  PRIVATE_KEY_PEM_FILE="$private_key_pem_file" \
  KEY_ID_HEX_FILE="$key_id_hex_file" \
  MANIFEST_PATH="$manifest_path" \
  SIGNATURE_OUT_PATH="$signature_path" \
  node - <<'NODE'
const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");

const manifestPath = process.env.MANIFEST_PATH;
const manifestBytes = fs.readFileSync(manifestPath);
const privateKey = crypto.createPrivateKey(fs.readFileSync(process.env.PRIVATE_KEY_PEM_FILE, "utf8"));
const keyId = Buffer.from(fs.readFileSync(process.env.KEY_ID_HEX_FILE, "utf8").trim(), "hex");
const trustedComment = `trusted comment: timestamp:1772585616\tfile:${path.basename(manifestPath)}`;
const signatureBytes = crypto.sign(
  null,
  crypto.createHash("blake2b512").update(manifestBytes).digest(),
  privateKey,
);
const signatureText =
  "untrusted comment: signature from minisign secret key\n" +
  `${Buffer.concat([Buffer.from([0x45, 0x44]), keyId, signatureBytes]).toString("base64")}\n` +
  `${trustedComment}\n` +
  `${crypto
    .sign(
      null,
      Buffer.concat([
        signatureBytes,
        Buffer.from(trustedComment.slice("trusted comment: ".length), "utf8"),
      ]),
      privateKey,
    )
    .toString("base64")}\n`;
fs.writeFileSync(process.env.SIGNATURE_OUT_PATH, `${Buffer.from(signatureText, "utf8").toString("base64")}\n`, "utf8");
NODE
}

cat >"$latest_json" <<EOF
{
  "channel": "stable",
  "latest_version": "0.4.99",
  "published_at": "2026-03-04T00:00:00Z",
  "source_commit": "test-0.4.99",
  "platforms": {
    "macos-arm64": {
      "dmg": {
        "url_path": "/download/stable/0.4.99/${mac_desktop_name}",
        "sha256": "${mac_desktop_sha}"
      },
      "daemon": {
        "url_path": "/download/stable/0.4.99/${mac_daemon_name}",
        "sha256": "${mac_daemon_sha}"
      }
    },
    "linux-x64": {
      "desktop": {
        "url_path": "/download/stable/0.4.99/${linux_appimage_name}",
        "sha256": "${linux_appimage_sha}"
      },
      "appimage": {
        "url_path": "/download/stable/0.4.99/${linux_appimage_name}",
        "sha256": "${linux_appimage_sha}"
      },
      "daemon": {
        "url_path": "/download/stable/0.4.99/${linux_daemon_name}",
        "sha256": "${linux_daemon_sha}"
      }
    }
  }
}
EOF

cat >"$latest_tauri_json" <<EOF
{
  "version": "0.4.99",
  "notes": "ctx 0.4.99",
  "pub_date": "2026-03-04T00:00:00Z",
  "source_commit": "test-0.4.99",
  "platforms": {
    "macos-arm64": {
      "url": "http://127.0.0.1:PORT/functions/v1/download/stable/0.4.99/${mac_artifact_name}",
      "signature": "${mac_signature_b64}"
    },
    "linux-x64": {
      "url": "http://127.0.0.1:PORT/functions/v1/download/stable/0.4.99/${linux_appimage_name}",
      "signature": "${linux_appimage_signature_b64}"
    },
    "linux-x64-appimage": {
      "url": "http://127.0.0.1:PORT/functions/v1/download/stable/0.4.99/${linux_appimage_name}",
      "signature": "${linux_appimage_signature_b64}"
    }
  }
}
EOF

cp "$latest_json" "$version_json"
cp "$latest_tauri_json" "$version_tauri_json"
sign_manifest_file "$latest_json" "$latest_json_sig"
sign_manifest_file "$version_json" "$version_json_sig"

PORT_FILE="$port_file" TMP_DIR="$tmp" LINUX_GET_MARKER="$linux_get_marker" node - <<'NODE' >"$tmp/server.log" 2>&1 &
const fs = require("node:fs");
const http = require("node:http");
const path = require("node:path");

const tmpDir = process.env.TMP_DIR;
const portFile = process.env.PORT_FILE;
const linuxGetMarker = process.env.LINUX_GET_MARKER;
const omitDmgContentLengthMarker = path.join(tmpDir, "omit-dmg-content-length");
let corruptUpdaterGetsRemaining = 1;
const updaterPaths = new Set([
  "/storage/ctx_0.4.99_macos-arm64_updater.app.tar.gz",
]);
const linuxStoragePaths = new Set([
  "/storage/ctx_0.4.99_linux-x64_updater.AppImage",
  "/storage/ctx_0.4.99_linux-x64",
]);
const files = new Map([
  ["/functions/v1/releases/stable/latest.json", path.join(tmpDir, "latest.json")],
  ["/functions/v1/releases/stable/latest.json.sig", path.join(tmpDir, "latest.json.sig")],
  ["/functions/v1/releases/stable/latest-tauri.json", path.join(tmpDir, "latest-tauri.json")],
  ["/functions/v1/releases/stable/0.4.99.json", path.join(tmpDir, "0.4.99.json")],
  ["/functions/v1/releases/stable/0.4.99.json.sig", path.join(tmpDir, "0.4.99.json.sig")],
  ["/functions/v1/releases/stable/0.4.99-tauri.json", path.join(tmpDir, "0.4.99-tauri.json")],
  ["/functions/v1/releases/stable-dry-run-verify/latest.json", path.join(tmpDir, "latest-storage.json")],
  ["/functions/v1/releases/stable-dry-run-verify/latest.json.sig", path.join(tmpDir, "latest-storage.json.sig")],
  ["/functions/v1/releases/stable-dry-run-verify/latest-tauri.json", path.join(tmpDir, "latest-storage-tauri.json")],
  ["/functions/v1/releases/stable-dry-run-verify/0.4.99.json", path.join(tmpDir, "0.4.99-storage.json")],
  ["/functions/v1/releases/stable-dry-run-verify/0.4.99.json.sig", path.join(tmpDir, "0.4.99-storage.json.sig")],
  ["/functions/v1/releases/stable-dry-run-verify/0.4.99-tauri.json", path.join(tmpDir, "0.4.99-storage-tauri.json")],
  [
    "/storage/ctx_0.4.99_macos-arm64_updater.app.tar.gz",
    path.join(tmpDir, "ctx_0.4.99_macos-arm64_updater.app.tar.gz"),
  ],
  ["/storage/ctx_0.4.99_macos-arm64.dmg", path.join(tmpDir, "ctx_0.4.99_macos-arm64.dmg")],
  ["/storage/ctx_0.4.99_macos-arm64", path.join(tmpDir, "ctx_0.4.99_macos-arm64")],
  ["/storage/ctx_0.4.99_linux-x64_updater.AppImage", path.join(tmpDir, "ctx_0.4.99_linux-x64_updater.AppImage")],
  ["/storage/ctx_0.4.99_linux-x64", path.join(tmpDir, "ctx_0.4.99_linux-x64")],
]);
const redirects = new Map([
  [
    "/functions/v1/download/stable/0.4.99/ctx_0.4.99_macos-arm64_updater.app.tar.gz",
    "/storage/ctx_0.4.99_macos-arm64_updater.app.tar.gz",
  ],
  [
    "/functions/v1/download/stable-dry-run-verify/0.4.99/ctx_0.4.99_macos-arm64_updater.app.tar.gz",
    "/storage/ctx_0.4.99_macos-arm64_updater.app.tar.gz",
  ],
  [
    "/functions/v1/download/stable/0.4.99/ctx_0.4.99_macos-arm64.dmg",
    "/storage/ctx_0.4.99_macos-arm64.dmg",
  ],
  [
    "/functions/v1/download/stable-dry-run-verify/0.4.99/ctx_0.4.99_macos-arm64.dmg",
    "/storage/ctx_0.4.99_macos-arm64.dmg",
  ],
  [
    "/functions/v1/download/stable/0.4.99/ctx_0.4.99_macos-arm64",
    "/storage/ctx_0.4.99_macos-arm64",
  ],
  [
    "/functions/v1/download/stable-dry-run-verify/0.4.99/ctx_0.4.99_macos-arm64",
    "/storage/ctx_0.4.99_macos-arm64",
  ],
  [
    "/functions/v1/download/stable/0.4.99/ctx_0.4.99_linux-x64_updater.AppImage",
    "/storage/ctx_0.4.99_linux-x64_updater.AppImage",
  ],
  [
    "/functions/v1/download/stable-dry-run-verify/0.4.99/ctx_0.4.99_linux-x64_updater.AppImage",
    "/storage/ctx_0.4.99_linux-x64_updater.AppImage",
  ],
  [
    "/functions/v1/download/stable/0.4.99/ctx_0.4.99_linux-x64",
    "/storage/ctx_0.4.99_linux-x64",
  ],
  [
    "/functions/v1/download/stable-dry-run-verify/0.4.99/ctx_0.4.99_linux-x64",
    "/storage/ctx_0.4.99_linux-x64",
  ],
]);

const server = http.createServer((req, res) => {
  const url = new URL(req.url, "http://127.0.0.1");
  if (redirects.has(url.pathname)) {
    res.statusCode = 302;
    res.setHeader("Location", redirects.get(url.pathname));
    res.end();
    return;
  }

  const filePath = files.get(url.pathname);
  if (!filePath) {
    res.statusCode = 404;
    res.end("not found");
    return;
  }

  let body = fs.readFileSync(filePath);
  if (req.method === "GET" && linuxStoragePaths.has(url.pathname)) {
    fs.writeFileSync(linuxGetMarker, `${url.pathname}\n`, "utf8");
  }
  if (
    req.method === "GET" &&
    updaterPaths.has(url.pathname) &&
    corruptUpdaterGetsRemaining > 0
  ) {
    corruptUpdaterGetsRemaining -= 1;
    body = Buffer.from("corrupt updater bytes\n", "utf8");
  }
  res.statusCode = 200;
  const omitContentLength =
    url.pathname === "/storage/ctx_0.4.99_macos-arm64.dmg" &&
    fs.existsSync(omitDmgContentLengthMarker);
  if (!omitContentLength) {
    res.setHeader("Content-Length", String(body.length));
  }
  if (req.method === "HEAD") {
    res.end();
    return;
  }
  res.end(body);
});

server.listen(0, "127.0.0.1", () => {
  fs.writeFileSync(portFile, String(server.address().port), "utf8");
});
NODE
server_pid=$!

for _ in $(seq 1 50); do
  if [[ -s "$port_file" ]]; then
    break
  fi
  sleep 0.1
done

if [[ ! -s "$port_file" ]]; then
  echo "error: test server did not report a port" >&2
  exit 1
fi

port="$(<"$port_file")"
verified_manifest_path="$tmp/verified-version.json"
verified_manifest_sig_path="$tmp/verified-version.json.sig"
verified_tauri_manifest_path="$tmp/verified-version-tauri.json"
python3 - <<'PY' "$latest_tauri_json" "$port"
from pathlib import Path
import sys

manifest_path = Path(sys.argv[1])
port = sys.argv[2]
manifest_path.write_text(manifest_path.read_text().replace("PORT", port))
PY
python3 - <<'PY' "$version_tauri_json" "$port"
from pathlib import Path
import sys

manifest_path = Path(sys.argv[1])
port = sys.argv[2]
manifest_path.write_text(manifest_path.read_text().replace("PORT", port))
PY

python3 - <<'PY' "$latest_json" "$latest_tauri_json" "$version_json" "$version_tauri_json" "$storage_latest_json" "$storage_latest_tauri_json" "$storage_version_json" "$storage_version_tauri_json" "$storage_override_channel"
from pathlib import Path
import sys

latest_json = Path(sys.argv[1]).read_text()
latest_tauri_json = Path(sys.argv[2]).read_text()
version_json = Path(sys.argv[3]).read_text()
version_tauri_json = Path(sys.argv[4]).read_text()
storage_latest_json = Path(sys.argv[5])
storage_latest_tauri_json = Path(sys.argv[6])
storage_version_json = Path(sys.argv[7])
storage_version_tauri_json = Path(sys.argv[8])
storage_channel = sys.argv[9]

storage_latest_json.write_text(latest_json.replace("/download/stable/", f"/download/{storage_channel}/"))
storage_latest_tauri_json.write_text(latest_tauri_json.replace("/download/stable/", f"/download/{storage_channel}/"))
storage_version_json.write_text(version_json.replace("/download/stable/", f"/download/{storage_channel}/"))
storage_version_tauri_json.write_text(version_tauri_json.replace("/download/stable/", f"/download/{storage_channel}/"))
PY
sign_manifest_file "$storage_latest_json" "$storage_latest_json_sig"
sign_manifest_file "$storage_version_json" "$storage_version_json_sig"

CTX_DESKTOP_UPDATER_PUBKEY="$pubkey_b64" \
RELEASE_FUNCTIONS_URL="http://127.0.0.1:${port}/functions/v1" \
RELEASE_SOURCE_COMMIT="test-0.4.99" \
RELEASE_CHANNEL="stable" \
RELEASE_STORAGE_CHANNEL="stable" \
RELEASE_EXPECTED_PLATFORMS="linux-x64,macos-arm64" \
RELEASE_VERIFY_SCOPE="expected" \
RELEASE_VERIFY_VERSION="0.4.99" \
RELEASE_VERIFIED_MANIFEST_PATH="$verified_manifest_path" \
RELEASE_VERIFIED_MANIFEST_SIG_PATH="$verified_manifest_sig_path" \
RELEASE_VERIFIED_TAURI_MANIFEST_PATH="$verified_tauri_manifest_path" \
RELEASE_VERIFY_ALLOW_HTTP_UPDATER_URLS="1" \
RELEASE_VERIFY_VALIDATION_RETRIES="2" \
RELEASE_MIN_DMG_SIZE_BYTES="1" \
RELEASE_VERIFY_LINUX_REMOTE_ARTIFACTS="0" \
./scripts/release_verify_storage.sh >/dev/null
if [[ -e "$linux_get_marker" ]]; then
  echo "error: disabled Linux artifact verification still downloaded Linux storage artifact" >&2
  cat "$linux_get_marker" >&2
  exit 1
fi

python3 - <<'PY' "$verified_manifest_path" "$verified_manifest_sig_path" "$verified_tauri_manifest_path"
from pathlib import Path
import json
import sys

manifest = json.loads(Path(sys.argv[1]).read_text())
manifest_sig = Path(sys.argv[2]).read_text().strip()
tauri = json.loads(Path(sys.argv[3]).read_text())
if manifest["source_commit"] != "test-0.4.99":
    raise SystemExit("verified version manifest did not preserve source_commit")
if not manifest_sig:
    raise SystemExit("verified version manifest signature was not written")
if tauri["source_commit"] != "test-0.4.99":
    raise SystemExit("verified tauri manifest did not preserve source_commit")
if manifest["latest_version"] != "0.4.99":
    raise SystemExit("verified version manifest did not preserve version")
if tauri["version"] != "0.4.99":
    raise SystemExit("verified tauri manifest did not preserve version")
PY

CTX_DESKTOP_UPDATER_PUBKEY="$pubkey_b64" \
RELEASE_FUNCTIONS_URL="http://127.0.0.1:${port}/functions/v1" \
RELEASE_SOURCE_COMMIT="test-0.4.99" \
RELEASE_CHANNEL="stable" \
RELEASE_STORAGE_CHANNEL="stable" \
RELEASE_EXPECTED_PLATFORMS="linux-x64,macos-arm64" \
RELEASE_VERIFY_SCOPE="expected" \
RELEASE_VERIFY_ALLOW_HTTP_UPDATER_URLS="1" \
RELEASE_MIN_DMG_SIZE_BYTES="1" \
RELEASE_VERIFY_LINUX_REMOTE_ARTIFACTS="0" \
./scripts/release_verify_storage.sh >/dev/null

invalid_sha_log="$tmp/invalid-sha.log"
cp "$latest_json" "$tmp/latest-valid.json"
python3 - <<'PY' "$latest_json"
from pathlib import Path
import json
import sys

path = Path(sys.argv[1])
manifest = json.loads(path.read_text())
manifest["platforms"]["linux-x64"]["daemon"]["sha256"] = "not-a-sha"
path.write_text(json.dumps(manifest, indent=2) + "\n")
PY
sign_manifest_file "$latest_json" "$latest_json_sig"
if CTX_DESKTOP_UPDATER_PUBKEY="$pubkey_b64" \
RELEASE_FUNCTIONS_URL="http://127.0.0.1:${port}/functions/v1" \
RELEASE_SOURCE_COMMIT="test-0.4.99" \
RELEASE_CHANNEL="stable" \
RELEASE_STORAGE_CHANNEL="stable" \
RELEASE_EXPECTED_PLATFORMS="linux-x64,macos-arm64" \
RELEASE_VERIFY_SCOPE="expected" \
RELEASE_VERIFY_ALLOW_HTTP_UPDATER_URLS="1" \
RELEASE_MIN_DMG_SIZE_BYTES="1" \
RELEASE_VERIFY_LINUX_REMOTE_ARTIFACTS="0" \
./scripts/release_verify_storage.sh >"$invalid_sha_log" 2>&1; then
  echo "error: release verify accepted an invalid Linux daemon sha256" >&2
  exit 1
fi
if ! grep -q "platform linux-x64 missing daemon artifact" "$invalid_sha_log"; then
  echo "error: invalid sha failure did not explain the invalid Linux daemon artifact" >&2
  cat "$invalid_sha_log" >&2
  exit 1
fi
cp "$tmp/latest-valid.json" "$latest_json"
sign_manifest_file "$latest_json" "$latest_json_sig"

missing_appimage_log="$tmp/missing-linux-appimage.log"
python3 - <<'PY' "$latest_json"
from pathlib import Path
import json
import sys

path = Path(sys.argv[1])
manifest = json.loads(path.read_text())
del manifest["platforms"]["linux-x64"]["appimage"]
path.write_text(json.dumps(manifest, indent=2) + "\n")
PY
sign_manifest_file "$latest_json" "$latest_json_sig"
if CTX_DESKTOP_UPDATER_PUBKEY="$pubkey_b64" \
RELEASE_FUNCTIONS_URL="http://127.0.0.1:${port}/functions/v1" \
RELEASE_SOURCE_COMMIT="test-0.4.99" \
RELEASE_CHANNEL="stable" \
RELEASE_STORAGE_CHANNEL="stable" \
RELEASE_EXPECTED_PLATFORMS="linux-x64,macos-arm64" \
RELEASE_VERIFY_SCOPE="expected" \
RELEASE_VERIFY_ALLOW_HTTP_UPDATER_URLS="1" \
RELEASE_MIN_DMG_SIZE_BYTES="1" \
RELEASE_VERIFY_LINUX_REMOTE_ARTIFACTS="0" \
./scripts/release_verify_storage.sh >"$missing_appimage_log" 2>&1; then
  echo "error: release verify accepted a missing Linux AppImage artifact" >&2
  exit 1
fi
if ! grep -q "platform linux-x64 missing required Linux appimage artifact" "$missing_appimage_log"; then
  echo "error: missing AppImage failure did not explain the invalid Linux manifest" >&2
  cat "$missing_appimage_log" >&2
  exit 1
fi
cp "$tmp/latest-valid.json" "$latest_json"
sign_manifest_file "$latest_json" "$latest_json_sig"

CTX_DESKTOP_UPDATER_PUBKEY="$pubkey_b64" \
RELEASE_FUNCTIONS_URL="http://127.0.0.1:${port}/functions/v1" \
RELEASE_SOURCE_COMMIT="test-0.4.99" \
RELEASE_CHANNEL="stable" \
RELEASE_STORAGE_CHANNEL="$storage_override_channel" \
RELEASE_EXPECTED_PLATFORMS="linux-x64,macos-arm64" \
RELEASE_VERIFY_SCOPE="expected" \
RELEASE_VERIFY_ALLOW_HTTP_UPDATER_URLS="1" \
RELEASE_MIN_DMG_SIZE_BYTES="1" \
RELEASE_VERIFY_LINUX_REMOTE_ARTIFACTS="0" \
./scripts/release_verify_storage.sh >/dev/null

missing_size_log="$tmp/missing-size-dmg.log"
printf 'tiny dmg\n' >"$mac_desktop_path"
touch "$tmp/omit-dmg-content-length"
if CTX_DESKTOP_UPDATER_PUBKEY="$pubkey_b64" \
RELEASE_FUNCTIONS_URL="http://127.0.0.1:${port}/functions/v1" \
RELEASE_SOURCE_COMMIT="test-0.4.99" \
RELEASE_CHANNEL="stable" \
RELEASE_STORAGE_CHANNEL="stable" \
RELEASE_EXPECTED_PLATFORMS="linux-x64,macos-arm64" \
RELEASE_VERIFY_SCOPE="expected" \
RELEASE_VERIFY_ALLOW_HTTP_UPDATER_URLS="1" \
RELEASE_MIN_DMG_SIZE_BYTES="1024" \
RELEASE_VERIFY_LINUX_REMOTE_ARTIFACTS="0" \
./scripts/release_verify_storage.sh >"$missing_size_log" 2>&1; then
  echo "error: release verify accepted a tiny DMG without content-length proof" >&2
  exit 1
fi
rm -f "$tmp/omit-dmg-content-length"
if ! grep -q "size could not be proven" "$missing_size_log"; then
  echo "error: release verify missing-size failure did not explain the invalid DMG" >&2
  cat "$missing_size_log" >&2
  exit 1
fi

appledouble_log="$tmp/appledouble-dmg.log"
printf '\000\005\026\007appledouble dmg sidecar\n' >"$mac_desktop_path"
if CTX_DESKTOP_UPDATER_PUBKEY="$pubkey_b64" \
RELEASE_FUNCTIONS_URL="http://127.0.0.1:${port}/functions/v1" \
RELEASE_SOURCE_COMMIT="test-0.4.99" \
RELEASE_CHANNEL="stable" \
RELEASE_STORAGE_CHANNEL="stable" \
RELEASE_EXPECTED_PLATFORMS="linux-x64,macos-arm64" \
RELEASE_VERIFY_SCOPE="expected" \
RELEASE_VERIFY_ALLOW_HTTP_UPDATER_URLS="1" \
RELEASE_MIN_DMG_SIZE_BYTES="1" \
RELEASE_VERIFY_LINUX_REMOTE_ARTIFACTS="0" \
./scripts/release_verify_storage.sh >"$appledouble_log" 2>&1; then
  echo "error: release verify accepted an AppleDouble DMG sidecar" >&2
  exit 1
fi
if ! grep -q "AppleDouble" "$appledouble_log"; then
  echo "error: release verify AppleDouble failure did not explain the invalid DMG" >&2
  cat "$appledouble_log" >&2
  exit 1
fi

echo "ok: release verify updater smoke passed for macOS + Linux versioned/latest manifests"
