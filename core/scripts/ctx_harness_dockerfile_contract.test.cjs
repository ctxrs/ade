const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");

const dockerfilePath = path.resolve(__dirname, "..", "..", "containers", "ctx-harness", "Dockerfile");
const dockerfile = fs.readFileSync(dockerfilePath, "utf8");

test("ctx-harness Dockerfile rewrites Ubuntu mirrors to HTTPS before apt install", () => {
  assert.match(
    dockerfile,
    /COPY --from=builder \/etc\/ssl\/certs\/ca-certificates\.crt \/etc\/ssl\/certs\/ca-certificates\.crt/,
    "ctx-harness image must seed the runtime stage with a CA bundle before switching apt sources to HTTPS",
  );
  assert.match(
    dockerfile,
    /sed -i[\s\S]*https:\/\/archive\.ubuntu\.com\/ubuntu/,
    "ctx-harness image must rewrite the Ubuntu archive mirror to HTTPS before apt-get update",
  );
  assert.match(
    dockerfile,
    /sed -i[\s\S]*https:\/\/security\.ubuntu\.com\/ubuntu/,
    "ctx-harness image must rewrite the Ubuntu security mirror to HTTPS before apt-get update",
  );
  assert.match(
    dockerfile,
    /done && \\\n\s+apt-get update && \\\n\s+DEBIAN_FRONTEND=noninteractive apt-get install/,
    "ctx-harness image must patch sources before running apt-get update",
  );
});
