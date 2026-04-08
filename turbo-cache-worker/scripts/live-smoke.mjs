import crypto from "node:crypto";

function requireEnv(name) {
  const value = String(process.env[name] || "").trim();
  if (!value) {
    throw new Error(`missing required env ${name}`);
  }
  return value;
}

async function request(url, init) {
  const response = await fetch(url, init);
  if (!response.ok) {
    const body = await response.text();
    throw new Error(`${init?.method || "GET"} ${url} failed: ${response.status} ${body}`);
  }
  return response;
}

async function main() {
  const api = requireEnv("TURBO_API").replace(/\/+$/, "");
  const token = requireEnv("TURBO_TOKEN");
  const team = requireEnv("TURBO_TEAM");
  const hash = crypto.randomBytes(8).toString("hex");
  const payload = new TextEncoder().encode(`ctx-turbo-cache-live-smoke:${hash}`);
  const headers = {
    authorization: `Bearer ${token}`,
  };
  const scopedUrl = `${api}/artifacts/${hash}?slug=${encodeURIComponent(team)}`;

  const statusResponse = await request(`${api}/artifacts/status?slug=${encodeURIComponent(team)}`, {
    headers,
  });
  const statusBody = await statusResponse.json();

  await request(scopedUrl, {
    method: "PUT",
    headers: {
      ...headers,
      "content-length": String(payload.length),
      "x-artifact-duration": "9",
    },
    body: payload,
  });

  const headResponse = await request(scopedUrl, {
    method: "HEAD",
    headers,
  });
  const getResponse = await request(scopedUrl, {
    headers,
  });
  const getBody = new Uint8Array(await getResponse.arrayBuffer());

  const summary = {
    status: statusBody,
    hash,
    contentLength: headResponse.headers.get("content-length"),
    artifactDuration: headResponse.headers.get("x-artifact-duration"),
    payloadMatches: Buffer.compare(Buffer.from(getBody), Buffer.from(payload)) === 0,
  };

  process.stdout.write(`${JSON.stringify(summary, null, 2)}\n`);
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : String(error));
  process.exit(1);
});
