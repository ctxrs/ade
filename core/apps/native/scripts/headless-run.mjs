#!/usr/bin/env node
// Headless native automation runner (Xorg dummy + openbox + ctx + automation script)
// - Extracts user-space Xorg + dummy driver into /tmp/xorg-user/root if missing
// - Launches Xorg (dummy) and openbox on a free DISPLAY
// - Sets software rendering env vars
// - Builds and runs ctx with automation enabled
// - Waits for /ready, then runs native-automation.mjs
// - Writes logs to /tmp/ctx-headless-<display> and prints a concise summary

import { spawn, spawnSync } from 'node:child_process';
import { promises as fs } from 'node:fs';
import { createWriteStream, existsSync } from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import http from 'node:http';
import { setTimeout as sleep } from 'node:timers/promises';
import net from 'node:net';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const REPO_ROOT = path.resolve(__dirname, '../../../..');
const NATIVE_DIR = path.join(REPO_ROOT, 'core', 'apps', 'native');
const SCRIPTS_DIR = path.join(NATIVE_DIR, 'scripts');
const AUTOMATION_SCRIPT = path.join(SCRIPTS_DIR, 'native-automation.mjs');

const XORG_USER_ROOT = '/tmp/xorg-user/root';
const XORG_BIN = path.join(XORG_USER_ROOT, 'usr', 'lib', 'xorg', 'Xorg');
const XORG_MODULES = path.join(XORG_USER_ROOT, 'usr', 'lib', 'xorg', 'modules');

function now() { return Date.now(); }
function log(msg) { console.log(`[headless] ${msg}`); }

function spawnLogged(cmd, args, opts, logFilePath) {
  const child = spawn(cmd, args, { stdio: ['ignore', 'pipe', 'pipe'], ...opts });
  const logStream = logFilePath ? createWriteStream(logFilePath, { flags: 'a' }) : null;
  if (child.stdout) child.stdout.on('data', (d) => { if (logStream) logStream.write(d); });
  if (child.stderr) child.stderr.on('data', (d) => { if (logStream) logStream.write(d); });
  child.once('close', () => { if (logStream) logStream.end(); });
  return child;
}

async function runLogged(cmd, args, opts, logFilePath) {
  return new Promise((resolve, reject) => {
    const child = spawn(cmd, args, { stdio: ['ignore', 'pipe', 'pipe'], ...opts });
    const logStream = logFilePath ? createWriteStream(logFilePath, { flags: 'a' }) : null;
    const chunks = []; const errs = [];
    if (child.stdout) child.stdout.on('data', (d) => { chunks.push(d); if (logStream) logStream.write(d); });
    if (child.stderr) child.stderr.on('data', (d) => { errs.push(d); if (logStream) logStream.write(d); });
    child.on('error', reject);
    child.on('close', (code) => { if (logStream) logStream.end(); if (code === 0) resolve(Buffer.concat(chunks).toString('utf8')); else reject(new Error(`${cmd} exited with code ${code}: ${Buffer.concat(errs).toString('utf8')}`)); });
  });
}

async function ensureDir(p) { await fs.mkdir(p, { recursive: true }); return p; }
async function fileExists(p) { try { await fs.access(p); return true; } catch { return false; } }

function parseModeline(output) {
  const line = (output || '').split('\n').map((l) => l.trim()).find((l) => l.startsWith('Modeline '));
  if (!line) return null;
  const match = line.match(/Modeline\\s+\"([^\"]+)\"\\s+(.*)$/);
  if (!match) return null;
  return { name: match[1], line };
}

function runCvt(args) {
  const result = spawnSync('cvt', args, { encoding: 'utf8' });
  if (result.status !== 0) return null;
  return parseModeline(result.stdout || '');
}

async function computeModeline(width, height) {
  return (
    runCvt(['-r', String(width), String(height), '60']) ||
    runCvt([String(width), String(height), '60'])
  );
}

function fallbackModeline(width, height) {
  if (width === 2400 && height === 1704) {
    return {
      name: '2400x1704R',
      line: 'Modeline \"2400x1704R\"  269.00  2400 2448 2480 2560  1704 1707 1717 1753 +hsync -vsync',
    };
  }
  return null;
}

function hasXvfb() {
  const result = spawnSync('Xvfb', ['-help'], { stdio: 'ignore' });
  return result.status === 0;
}

async function findFreeDisplay(start = 210, end = 230) {
  for (let d = start; d <= end; d++) {
    const sock = `/tmp/.X11-unix/X${d}`;
    if (!existsSync(sock)) return d;
  }
  throw new Error(`No free DISPLAY found in range :${start}-:${end}`);
}

async function findFreePort() {
  return new Promise((resolve, reject) => {
    const srv = net.createServer();
    srv.listen(0, '127.0.0.1', () => { const address = srv.address(); const port = typeof address === 'object' && address ? address.port : null; srv.close(() => port ? resolve(port) : reject(new Error('Failed to allocate port'))); });
    srv.on('error', reject);
  });
}

async function httpGet(url, timeoutMs = 10000) {
  return new Promise((resolve, reject) => {
    const req = http.get(url, (res) => { const chunks = []; res.on('data', (d) => chunks.push(d)); res.on('end', () => resolve({ statusCode: res.statusCode, body: Buffer.concat(chunks).toString('utf8') })); });
    req.setTimeout(timeoutMs, () => { req.destroy(new Error('timeout')); });
    req.on('error', reject);
  });
}

async function ensureUserSpaceXorg(logDir) {
  if (await fileExists(XORG_BIN)) return { extracted: false };
  log('Extracting user-space Xorg + dummy driver...');
  await ensureDir(path.dirname(XORG_BIN));
  const cwd = await ensureDir('/tmp/xorg-user');
  const logFile = path.join(logDir, 'xorg-extract.log');
  const apt = async (args) => runLogged('bash', ['-lc', args], { cwd }, logFile);
  await apt('set -euo pipefail; apt-get update -y >/dev/null 2>&1 || true');
  await apt('set -euo pipefail; apt-get download -y xserver-xorg-core >/dev/null 2>&1 || apt-get download xserver-xorg-core');
  await apt('set -euo pipefail; apt-get download -y xserver-xorg-video-dummy >/dev/null 2>&1 || apt-get download xserver-xorg-video-dummy');
  await apt('set -euo pipefail; apt-get download -y libxcvt0 >/dev/null 2>&1 || apt-get download libxcvt0 || true');
  await apt('set -euo pipefail; apt-get download -y libxfont2 >/dev/null 2>&1 || apt-get download libxfont2 || true');
  await apt('set -euo pipefail; for f in xserver-xorg-core_* ./*xorg*core*.deb; do [ -f "$f" ] && dpkg -x "$f" /tmp/xorg-user/root; done');
  await apt('set -euo pipefail; for f in xserver-xorg-video-dummy_* ./*video-dummy*.deb; do [ -f "$f" ] && dpkg -x "$f" /tmp/xorg-user/root; done');
  await apt('set -euo pipefail; for f in libxcvt0_* ./*libxcvt*0*.deb; do [ -f "$f" ] && dpkg -x "$f" /tmp/xorg-user/root; done');
  await apt('set -euo pipefail; for f in libxfont2_* ./*libxfont2*.deb; do [ -f "$f" ] && dpkg -x "$f" /tmp/xorg-user/root; done');
  if (!(await fileExists(XORG_BIN))) throw new Error('Failed to extract Xorg binary to user-space');
  return { extracted: true };
}

async function writeXorgConfig({ width, height, logDir }) {
  const confPath = path.join(logDir, `xorg-dummy-${width}x${height}.conf`);
  const modeline = (await computeModeline(width, height)) || fallbackModeline(width, height);
  const modeName = modeline?.name ?? `${width}x${height}`;
  const videoRamKb = Math.max(256000, Math.ceil((width * height * 4) / 1024));
  const conf = `Section "Device"\n  Identifier "DummyDevice"\n  Driver "dummy"\n  VideoRam ${videoRamKb}\nEndSection\nSection "Monitor"\n  Identifier "DummyMonitor"\n  HorizSync 28-160\n  VertRefresh 48-75\n${modeline ? `  ${modeline.line}\n` : ''}EndSection\nSection "Screen"\n  Identifier "DummyScreen"\n  Device "DummyDevice"\n  Monitor "DummyMonitor"\n  DefaultDepth 24\n  SubSection "Display"\n    Depth 24\n    Modes "${modeName}"\n    Virtual ${width} ${height}\n  EndSubSection\nEndSection\n`;
  await fs.writeFile(confPath, conf, 'utf8');
  return confPath;
}

async function waitForXSocket(displayNumber, timeoutMs = 10000) {
  const sock = `/tmp/.X11-unix/X${displayNumber}`;
  const start = now();
  while (now() - start < timeoutMs) { if (existsSync(sock)) return true; await sleep(100); }
  return false;
}

async function countPngs(dir) { try { const files = await fs.readdir(dir); return files.filter(f => f.toLowerCase().endsWith('.png')).length; } catch { return 0; } }

function parseArgs(argv) {
  const args = {
    width: 1280,
    height: 720,
    readyTimeoutMs: 30000,
    addr: null,
    screenshotDir: null,
    fixture: null,
    automationScript: null,
    useXvfb: null,
    automationDelayMs: null,
  };
  for (let i = 2; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--window-size' && argv[i+1]) { const [w,h] = String(argv[++i]).split('x'); args.width = parseInt(w,10); args.height = parseInt(h,10); }
    else if (a.startsWith('--window-size=')) { const [w,h] = a.slice('--window-size='.length).split('x'); args.width = parseInt(w,10); args.height = parseInt(h,10); }
    else if (a === '--ready-timeout-ms' && argv[i+1]) { args.readyTimeoutMs = parseInt(argv[++i], 10); }
    else if (a.startsWith('--ready-timeout-ms=')) { args.readyTimeoutMs = parseInt(a.slice('--ready-timeout-ms='.length), 10); }
    else if (a === '--addr' && argv[i+1]) { args.addr = argv[++i]; }
    else if (a.startsWith('--addr=')) { args.addr = a.slice('--addr='.length); }
    else if (a === '--screenshot-dir' && argv[i+1]) { args.screenshotDir = argv[++i]; }
    else if (a.startsWith('--screenshot-dir=')) { args.screenshotDir = a.slice('--screenshot-dir='.length); }
    else if (a === '--fixture' && argv[i+1]) { args.fixture = argv[++i]; }
    else if (a.startsWith('--fixture=')) { args.fixture = a.slice('--fixture='.length); }
    else if (a === '--automation-script' && argv[i+1]) { args.automationScript = argv[++i]; }
    else if (a.startsWith('--automation-script=')) { args.automationScript = a.slice('--automation-script='.length); }
    else if (a === '--automation-delay-ms' && argv[i+1]) { args.automationDelayMs = Number(argv[++i]); }
    else if (a.startsWith('--automation-delay-ms=')) { args.automationDelayMs = Number(a.slice('--automation-delay-ms='.length)); }
    else if (a === '--xvfb') { args.useXvfb = true; }
    else if (a === '--no-xvfb') { args.useXvfb = false; }
  }
  return args;
}

async function main() {
  const t0 = now();
  const args = parseArgs(process.argv);

  const displayNumber = await findFreeDisplay();
  const DISPLAY = `:${displayNumber}`;
  const baseDir = `/tmp/ctx-headless-${displayNumber}`;
  await ensureDir(baseDir);
  const logs = { xorg: path.join(baseDir, 'xorg.log'), build: path.join(baseDir, 'build.log'), app: path.join(baseDir, 'app.log'), automation: path.join(baseDir, 'automation.log') };
  const shotsDir = args.screenshotDir || path.join(baseDir, 'shots');
  await ensureDir(shotsDir);

  const winSize = `${args.width}x${args.height}`;
  log(`Using DISPLAY=${DISPLAY}, window ${winSize}`);

  const useXvfb = args.useXvfb ?? hasXvfb();
  let extracted = false;
  let tExtract0 = now();
  let tExtract1 = tExtract0;
  let xorgConf = null;

  if (!useXvfb) {
    tExtract0 = now();
    const result = await ensureUserSpaceXorg(baseDir);
    extracted = result.extracted;
    tExtract1 = now();
    xorgConf = await writeXorgConfig({ width: args.width, height: args.height, logDir: baseDir });
  }

  const xEnv = {
    ...process.env,
    DISPLAY,
    LIBGL_ALWAYS_SOFTWARE: '1',
    VK_ICD_FILENAMES: process.env.VK_ICD_FILENAMES || '/usr/share/vulkan/icd.d/lvp_icd.x86_64.json',
    MESA_VK_WSI_PRESENT_MODE: 'immediate',
    vblank_mode: '0',
    LD_LIBRARY_PATH: [
      path.join(XORG_USER_ROOT, 'usr', 'lib', 'xorg'),
      path.join(XORG_USER_ROOT, 'usr', 'lib', 'x86_64-linux-gnu'),
      path.join(XORG_USER_ROOT, 'usr', 'lib'),
      process.env.LD_LIBRARY_PATH || '',
    ].filter(Boolean).join(':'),
  };

  // Start X server
  const xorg = useXvfb
    ? spawnLogged(
        'Xvfb',
        [
          DISPLAY,
          '-screen',
          '0',
          `${args.width}x${args.height}x24`,
          '-nolisten',
          'tcp',
          '-ac',
          '+extension',
          'RANDR',
          '+extension',
          'RENDER',
          '+extension',
          'XFIXES',
          '+extension',
          'GLX',
        ],
        { env: xEnv },
        logs.xorg,
      )
    : spawnLogged(XORG_BIN, [DISPLAY, '-noreset', '+extension', 'RANDR', '+extension', 'XFIXES', '+extension', 'RENDER', '+extension', 'GLX', '-config', xorgConf, '-logfile', logs.xorg, '-modulepath', XORG_MODULES], { env: xEnv }, logs.xorg);
  const xReady = await waitForXSocket(displayNumber, 15000);
  if (!xReady) throw new Error('Xorg socket did not appear in time');

  // Start openbox
  const ob = spawnLogged('openbox', ['--sm-disable'], { env: xEnv }, logs.xorg);
  await sleep(500);

  // Build app
  const tBuild0 = now();
  await runLogged('bash', ['-lc', 'cargo build --manifest-path core/apps/native/Cargo.toml --features automation'], { cwd: REPO_ROOT, env: xEnv }, logs.build);
  const tBuild1 = now();

  // Run app
  const port = args.addr ? Number(String(args.addr).split(':').pop()) : await findFreePort();
  const addr = args.addr || `127.0.0.1:${port}`;
  const targetDir = process.env.CARGO_TARGET_DIR || path.join(REPO_ROOT, 'target');
  const appBin = path.join(targetDir, 'debug', os.platform() === 'win32' ? 'ctx.exe' : 'ctx');
  const appArgs = [ '--automation-addr', addr, '--screenshot-dir', shotsDir, '--window-size', winSize ];
  if (args.fixture) {
    appArgs.push('--fixture', args.fixture);
  }
  const appEnv = {
    ...xEnv,
    RUST_LOG: process.env.RUST_LOG || 'info',
    CTX_SCREENSHOT_DIR: shotsDir,
    CTX_WINDOW_SIZE: winSize,
  };
  const app = spawnLogged(appBin, appArgs, { cwd: REPO_ROOT, env: appEnv }, logs.app);

  // Wait for /ready
  const httpUrl = `http://${addr}`;
  const readyUrl = `${httpUrl}/ready?timeout_ms=${args.readyTimeoutMs}`;
  const tReady0 = now();
  let readyOk = false;
  const readyDeadline = tReady0 + Math.max(args.readyTimeoutMs + 5000, 20000);
  while (!readyOk && now() < readyDeadline) {
    try { const res = await httpGet(readyUrl, Math.max(5000, args.readyTimeoutMs)); readyOk = res.statusCode === 200; if (readyOk) break; } catch (_) {}
    await sleep(250);
  }
  const tReady1 = now();
  if (!readyOk) throw new Error('App did not become ready in time');

  // Run automation script
  const tAuto0 = now();
  const automationScript = args.automationScript
    ? (path.isAbsolute(args.automationScript) ? args.automationScript : path.resolve(REPO_ROOT, args.automationScript))
    : AUTOMATION_SCRIPT;
  await new Promise((resolve, reject) => {
    const automationArgs = ['--addr', httpUrl, '--ready-timeout-ms', String(args.readyTimeoutMs)];
    if (Number.isFinite(args.automationDelayMs)) {
      automationArgs.push('--delay-ms', String(args.automationDelayMs));
    }
    const child = spawn('node', ['--experimental-websocket', automationScript, ...automationArgs], { cwd: REPO_ROOT, env: appEnv });
    const logStream = createWriteStream(logs.automation, { flags: 'a' });
    child.stdout.on('data', (d) => logStream.write(d));
    child.stderr.on('data', (d) => logStream.write(d));
    child.on('error', (e) => { logStream.end(); reject(e); });
    child.on('close', () => { logStream.end(); resolve(); });
  });
  const tAuto1 = now();

  // Teardown
  try { app.kill('SIGINT'); } catch {}
  try { ob.kill('SIGINT'); } catch {}
  try { xorg.kill('SIGINT'); } catch {}

  // Summary
  const shots = await countPngs(shotsDir);
  const summary = {
    display: DISPLAY,
    addr: httpUrl,
    shotsDir,
    screenshots: shots,
    timings_ms: {
      extract: extracted ? (tExtract1 - tExtract0) : 0,
      build: tBuild1 - tBuild0,
      ready: tReady1 - tReady0,
      automation: tAuto1 - tAuto0,
      total: now() - t0,
    },
    logs: logs,
  };
  console.log(JSON.stringify(summary, null, 2));
}

main().catch((err) => { console.error('[headless] ERROR:', err?.message || err); process.exitCode = 1; });
