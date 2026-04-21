const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

function captureHostSample(label = "") {
  const load = os.loadavg();
  const totalMemBytes = os.totalmem();
  const freeMemBytes = os.freemem();
  const usedMemBytes = Math.max(0, totalMemBytes - freeMemBytes);
  return {
    timestamp: new Date().toISOString(),
    label: String(label || ""),
    cpuCount: os.cpus().length,
    loadAvg1m: Number(load[0] || 0),
    loadAvg5m: Number(load[1] || 0),
    loadAvg15m: Number(load[2] || 0),
    totalMemBytes,
    freeMemBytes,
    usedMemBytes,
    usedMemRatio: totalMemBytes > 0 ? usedMemBytes / totalMemBytes : 0,
  };
}

function appendHostSample(outputPath, label = "") {
  fs.mkdirSync(path.dirname(outputPath), { recursive: true });
  fs.appendFileSync(outputPath, `${JSON.stringify(captureHostSample(label))}\n`, "utf8");
}

function readHostSamples(outputPath) {
  if (!outputPath || !fs.existsSync(outputPath)) {
    return [];
  }
  return fs.readFileSync(outputPath, "utf8")
    .split(/\r?\n/u)
    .map((line) => line.trim())
    .filter(Boolean)
    .map((line) => JSON.parse(line));
}

function summarizeHostSamples(outputPath) {
  const samples = readHostSamples(outputPath);
  if (samples.length === 0) {
    return {
      sampleCount: 0,
      cpuCount: 0,
      avgLoadAvg1m: 0,
      maxLoadAvg1m: 0,
      avgUsedMemRatio: 0,
      maxUsedMemRatio: 0,
    };
  }

  const totals = samples.reduce((accumulator, sample) => ({
    loadAvg1m: accumulator.loadAvg1m + Number(sample.loadAvg1m || 0),
    usedMemRatio: accumulator.usedMemRatio + Number(sample.usedMemRatio || 0),
    maxLoadAvg1m: Math.max(accumulator.maxLoadAvg1m, Number(sample.loadAvg1m || 0)),
    maxUsedMemRatio: Math.max(accumulator.maxUsedMemRatio, Number(sample.usedMemRatio || 0)),
    cpuCount: Math.max(accumulator.cpuCount, Number(sample.cpuCount || 0)),
  }), {
    loadAvg1m: 0,
    usedMemRatio: 0,
    maxLoadAvg1m: 0,
    maxUsedMemRatio: 0,
    cpuCount: 0,
  });

  return {
    sampleCount: samples.length,
    cpuCount: totals.cpuCount,
    avgLoadAvg1m: totals.loadAvg1m / samples.length,
    maxLoadAvg1m: totals.maxLoadAvg1m,
    avgUsedMemRatio: totals.usedMemRatio / samples.length,
    maxUsedMemRatio: totals.maxUsedMemRatio,
  };
}

module.exports = {
  appendHostSample,
  captureHostSample,
  readHostSamples,
  summarizeHostSamples,
};
