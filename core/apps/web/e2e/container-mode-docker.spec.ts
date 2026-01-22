import { test, expect, request, type APIRequestContext } from "./utils/fixtures";
import { execFileSync, execSync } from "child_process";
import crypto from "crypto";
import fs from "fs";
import net from "net";
import os from "os";
import path from "path";

const DOCKER_OPT_IN = process.env.CTX_E2E_DOCKER === "1";
const IS_LINUX = process.platform === "linux";

type DockerGate = { ok: boolean; reason: string };

type DockerDaemon = {
  baseUrl: string;
  containerName: string;
  dataDir: string;
  repoRoot: string;
  disallowedRoot: string;
  dockerProxySocket: string;
};

const dockerGate = (): DockerGate => {
  if (!DOCKER_OPT_IN) {
    return { ok: false, reason: "CTX_E2E_DOCKER=1 not set (skipping docker container tests)." };
  }
  if (!IS_LINUX) {
    return { ok: false, reason: "Docker container tests are Linux-only." };
  }
  try {
    execFileSync("docker", ["info", "--format", "{{.ServerVersion}}"], { stdio: "pipe" });
    return { ok: true, reason: "" };
  } catch (err: any) {
    const stderr = String(err?.stderr ?? err?.message ?? err ?? "").trim();
    const detail = stderr ? `Docker unavailable: ${stderr}` : "Docker unavailable.";
    return { ok: false, reason: detail };
  }
};

const dockerCheck = dockerGate();

test.describe("container mode (docker)", () => {
  test.skip(!dockerCheck.ok, dockerCheck.reason);
  test.describe.configure({ mode: "serial" });

  let daemon: DockerDaemon | null = null;
  let api: APIRequestContext | null = null;
  let workspaceId = "";

  test.beforeAll(async () => {
    daemon = await startContainerizedDaemon();
    const token = await readAuthToken(daemon.dataDir);
    api = await request.newContext({
      baseURL: daemon.baseUrl,
      extraHTTPHeaders: { Authorization: `Bearer ${token}` },
    });
    await waitForDaemon(api);
  });

  test.afterAll(async () => {
    if (api) {
      await api.dispose();
      api = null;
    }
    if (daemon) {
      await stopContainer(daemon.containerName);
      cleanupPath(daemon.dataDir);
      cleanupPath(daemon.repoRoot);
      cleanupPath(daemon.disallowedRoot);
      daemon = null;
    }
  });

  test("containerized daemon starts with host-mounted workspace root", async () => {
    if (!daemon || !api) throw new Error("docker daemon not ready");

    const resp = await api.post("/api/workspaces", {
      data: { root_path: daemon.repoRoot, name: "docker-e2e" },
    });
    expect(resp.ok()).toBeTruthy();
    const body = (await resp.json()) as { id?: string };
    workspaceId = String(body?.id ?? "");
    expect(workspaceId).not.toEqual("");
  });

  test("container mounts reject non-allowlisted paths", async () => {
    if (!daemon || !api) throw new Error("docker daemon not ready");

    const resp = await api.post("/api/workspaces", {
      data: { root_path: daemon.disallowedRoot, name: "docker-disallowed" },
    });
    expect(resp.status()).toBe(400);
    const body = (await resp.json()) as { error?: string };
    expect(String(body?.error ?? "")).toContain("invalid root_path");
  });

  test("docker passthrough exposes DOCKER_HOST", async () => {
    if (!daemon || !api) throw new Error("docker daemon not ready");
    if (!workspaceId) throw new Error("workspaceId missing from container setup");

    const taskId = await createTask(api, workspaceId, "docker env task");
    const sessionId = await createSession(api, taskId);

    const prompt = "docker env [[dump_env]]";
    const resp = await api.post(`/api/sessions/${sessionId}/messages`, {
      data: { content: prompt, delivery: "immediate" },
    });
    expect(resp.ok()).toBeTruthy();

    const envDump = await waitForEnvDump(api, sessionId);
    expect(envDump?.DOCKER_HOST).toBe(`unix://${daemon.dockerProxySocket}`);
  });
});

async function waitForDaemon(api: APIRequestContext): Promise<void> {
  const deadline = Date.now() + 20_000;
  while (Date.now() < deadline) {
    try {
      const resp = await api.get("/api/workspaces");
      if (resp.ok()) return;
    } catch {
      // ignore
    }
    await sleep(200);
  }
  throw new Error("docker daemon did not become ready");
}

async function readAuthToken(dataDir: string): Promise<string> {
  const authPath = path.join(dataDir, "daemon_auth.json");
  const deadline = Date.now() + 10_000;
  while (Date.now() < deadline) {
    try {
      const raw = fs.readFileSync(authPath, "utf8");
      const parsed = JSON.parse(raw);
      const token = String(parsed?.token ?? "").trim();
      if (token) return token;
    } catch {
      // ignore
    }
    await sleep(200);
  }
  throw new Error(`docker auth token not found at ${authPath}`);
}

async function waitForEnvDump(
  api: APIRequestContext,
  sessionId: string,
): Promise<Record<string, string | null>> {
  const deadline = Date.now() + 20_000;
  while (Date.now() < deadline) {
    const resp = await api.get(`/api/sessions/${sessionId}/snapshot?limit=50`);
    if (resp.ok()) {
      const snapshot = (await resp.json()) as any;
      const msgs = snapshot?.head?.messages ?? [];
      for (const message of msgs) {
        if (message?.role !== "assistant") continue;
        const content = String(message?.content ?? "");
        const env = extractEnvDump(content);
        if (env) return env;
      }
    }
    await sleep(200);
  }
  throw new Error("env dump not found in assistant messages");
}

function extractEnvDump(content: string): Record<string, string | null> | null {
  const match = content.match(/\[\[env]]([\s\S]*?)\[\[\/env]]/);
  if (!match) return null;
  try {
    return JSON.parse(match[1]);
  } catch {
    return null;
  }
}

async function createTask(
  api: APIRequestContext,
  workspaceId: string,
  title: string,
): Promise<string> {
  const resp = await api.post(`/api/workspaces/${workspaceId}/tasks`, {
    data: { title },
  });
  expect(resp.ok()).toBeTruthy();
  const body = (await resp.json()) as { id?: string };
  const taskId = String(body?.id ?? "");
  if (!taskId) throw new Error("task creation failed");
  return taskId;
}

async function createSession(
  api: APIRequestContext,
  taskId: string,
): Promise<string> {
  const resp = await api.post(`/api/tasks/${taskId}/sessions`, {
    data: { provider_id: "fake", model_id: "fake-model" },
  });
  expect(resp.ok()).toBeTruthy();
  const body = (await resp.json()) as { id?: string };
  const sessionId = String(body?.id ?? "");
  if (!sessionId) throw new Error("session creation failed");
  return sessionId;
}

async function startContainerizedDaemon(): Promise<DockerDaemon> {
  const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-e2e-docker-data-"));
  const repoRoot = initRepo("ctx-e2e-docker-repo-");
  const disallowedRoot = initRepo("ctx-e2e-docker-disallowed-");

  const cargoTargetDir = resolveCargoTargetDir();
  const ctxBin = await waitForCtxBinary(cargoTargetDir);

  const port = await pickPort();
  const containerName = `ctx-e2e-daemon-${process.pid}-${Date.now()}`;
  const dockerProxySocket = path.join(dataDir, "docker-proxy.sock");

  await stopContainer(containerName);

  const mounts = [
    `type=bind,src=${dataDir},dst=${dataDir}`,
    `type=bind,src=${repoRoot},dst=${repoRoot}`,
    `type=bind,src=${cargoTargetDir},dst=${cargoTargetDir},readonly`,
  ];

  const uid = typeof process.getuid === "function" ? process.getuid() : 0;
  const gid = typeof process.getgid === "function" ? process.getgid() : 0;

  const args = [
    "run",
    "-d",
    "--rm",
    "--name",
    containerName,
    "--publish",
    `127.0.0.1:${port}:${port}`,
    "--user",
    `${uid}:${gid}`,
    "--workdir",
    "/",
    ...mounts.flatMap((mount) => ["--mount", mount]),
    "--env",
    "CTX_SHOW_FAKE_PROVIDER=1",
    "--env",
    "CTX_DOCKER_PROXY_ENABLED=1",
    "--env",
    `CTX_DOCKER_PROXY_SOCKET=${dockerProxySocket}`,
    "--env",
    `CTX_DOCKER_PROXY_UPSTREAM=unix://${dockerProxySocket}`,
    "--env",
    "CTX_DAEMON_CONTAINER=1",
    "ubuntu:24.04",
    ctxBin,
    "serve",
    "--bind",
    `0.0.0.0:${port}`,
    "--data-dir",
    dataDir,
  ];

  execFileSync("docker", args, { stdio: "pipe" });

  return {
    baseUrl: `http://127.0.0.1:${port}`,
    containerName,
    dataDir,
    repoRoot,
    disallowedRoot,
    dockerProxySocket,
  };
}

async function waitForCtxBinary(cargoTargetDir: string): Promise<string> {
  const ctxBin = path.join(cargoTargetDir, "debug", "ctx");
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    if (fs.existsSync(ctxBin)) {
      return ctxBin;
    }
    await sleep(200);
  }
  throw new Error(`ctx binary missing at ${ctxBin}`);
}

function resolveCargoTargetDir(): string {
  if (process.env.CTX_E2E_CARGO_TARGET_DIR) {
    return process.env.CTX_E2E_CARGO_TARGET_DIR;
  }
  const hash = crypto
    .createHash("sha1")
    .update(process.cwd())
    .digest("hex")
    .slice(0, 10);
  return path.join(os.tmpdir(), `ctx-e2e-cargo-${hash}`);
}

function initRepo(prefix: string): string {
  const repo = fs.mkdtempSync(path.join(os.tmpdir(), prefix));
  execSync("git init", { cwd: repo });
  execSync("git config user.email test@example.com", { cwd: repo });
  execSync("git config user.name Test", { cwd: repo });
  fs.writeFileSync(path.join(repo, "README.md"), "fixture\n");
  execSync("git add .", { cwd: repo });
  execSync("git commit -m init", { cwd: repo });
  return repo;
}

async function pickPort(): Promise<number> {
  return await new Promise((resolve, reject) => {
    const server = net.createServer();
    server.unref();
    server.on("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      if (!address || typeof address === "string") {
        server.close();
        reject(new Error("failed to allocate port"));
        return;
      }
      const port = address.port;
      server.close((err) => {
        if (err) {
          reject(err);
          return;
        }
        resolve(port);
      });
    });
  });
}

async function stopContainer(name: string): Promise<void> {
  if (!name) return;
  try {
    execFileSync("docker", ["rm", "-f", name], { stdio: "pipe" });
  } catch {
    // ignore
  }
}

function cleanupPath(target: string): void {
  if (!target) return;
  try {
    fs.rmSync(target, { recursive: true, force: true });
  } catch {
    // ignore
  }
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
