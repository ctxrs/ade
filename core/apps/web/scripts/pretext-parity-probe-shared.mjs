export function parseProbeWidthList(raw) {
  const values = String(raw ?? "")
    .split(",")
    .map((value) => value.trim())
    .filter((value) => value.length > 0)
    .map((value) => Number.parseFloat(value));

  if (values.length === 0) {
    throw new Error("Expected at least one width in --widths.");
  }
  if (values.some((value) => !Number.isFinite(value) || value <= 0)) {
    throw new Error(`Invalid --widths value: ${raw}`);
  }

  return dedupeProbeWidths(values);
}

export function parseProbeWidthRange(raw) {
  const [startRaw, endRaw, stepRaw] = String(raw ?? "").split(":");
  const start = Number.parseFloat(startRaw ?? "");
  const end = Number.parseFloat(endRaw ?? "");
  const step = Number.parseFloat(stepRaw ?? "");

  if (!Number.isFinite(start) || !Number.isFinite(end) || !Number.isFinite(step)) {
    throw new Error(`Invalid --width-range: ${raw}`);
  }
  if (start <= 0 || end <= 0 || step <= 0) {
    throw new Error(`--width-range values must be positive: ${raw}`);
  }
  if (end < start) {
    throw new Error(`--width-range must be ascending: ${raw}`);
  }

  const values = [];
  for (let current = start; current <= end + step / 2; current += step) {
    values.push(Number.parseFloat(current.toFixed(4)));
  }
  return dedupeProbeWidths(values);
}

export function resolveProbeWidths(options) {
  if (options.widths && options.widthRange) {
    throw new Error("Use either --widths or --width-range, not both.");
  }
  if (options.widths) {
    return parseProbeWidthList(options.widths);
  }
  if (options.widthRange) {
    return parseProbeWidthRange(options.widthRange);
  }
  if (!Number.isFinite(options.width) || options.width <= 0) {
    throw new Error(`Invalid --width: ${options.width}`);
  }
  return [options.width];
}

export function summarizeProbeMeasurements(measurements, driftThreshold = 1) {
  const normalized = measurements.map((measurement) => ({
    ...measurement,
    absDelta: Math.abs(measurement.delta ?? Number.POSITIVE_INFINITY),
  }));
  const worst = normalized.reduce((selected, candidate) => {
    if (!selected || candidate.absDelta > selected.absDelta) {
      return candidate;
    }
    return selected;
  }, null);
  const firstDrift = normalized.find((measurement) => measurement.absDelta > driftThreshold) ?? null;

  return {
    driftThreshold,
    count: normalized.length,
    driftCount: normalized.filter((measurement) => measurement.absDelta > driftThreshold).length,
    allWithinThreshold: normalized.every((measurement) => measurement.absDelta <= driftThreshold),
    firstDriftWidth: firstDrift?.width ?? null,
    worstWidth: worst?.width ?? null,
    worstAbsDelta: worst?.absDelta ?? null,
    worstDelta: worst?.delta ?? null,
    worstPlanned: worst?.planned ?? null,
    worstActual: worst?.actual ?? null,
  };
}

export function selectProbeDebugWidth(measurements, explicitWidth = null) {
  if (explicitWidth != null) {
    const normalizedExplicit = Number.parseFloat(String(explicitWidth));
    if (!Number.isFinite(normalizedExplicit) || normalizedExplicit <= 0) {
      throw new Error(`Invalid --debug-width: ${explicitWidth}`);
    }
    return normalizedExplicit;
  }

  const summary = summarizeProbeMeasurements(measurements);
  if (summary.worstWidth != null) {
    return summary.worstWidth;
  }
  if (measurements[0]?.width != null) {
    return measurements[0].width;
  }
  throw new Error("Cannot choose debug width from an empty measurement set.");
}

export function shouldCleanupProbeScratchWorkspace(options, createdWorkspaceId) {
  return Boolean(createdWorkspaceId) && options.keepWorkspace !== true;
}

function dedupeProbeWidths(values) {
  const seen = new Set();
  const out = [];
  for (const value of values) {
    const normalized = Number.parseFloat(value.toFixed(4));
    const key = normalized.toString();
    if (seen.has(key)) continue;
    seen.add(key);
    out.push(normalized);
  }
  return out;
}
