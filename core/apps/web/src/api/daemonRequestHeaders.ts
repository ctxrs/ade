type DaemonRequestHeadersOptions = {
  headers?: HeadersInit;
  token?: string | null;
  traceparent?: string | null;
  runId?: string | null;
};

export const requestHeadersToRecord = (headers?: HeadersInit): Record<string, string> => {
  const values: Record<string, string> = {};
  if (!headers) return values;
  if (headers instanceof Headers) {
    headers.forEach((value, key) => {
      values[key] = value;
    });
    return values;
  }
  if (Array.isArray(headers)) {
    for (const [key, value] of headers) {
      values[key] = value;
    }
    return values;
  }
  Object.assign(values, headers);
  return values;
};

export const buildDaemonRequestHeaders = ({
  headers,
  token,
  traceparent,
  runId,
}: DaemonRequestHeadersOptions): Record<string, string> => {
  const mergedHeaders = requestHeadersToRecord(headers);
  if (traceparent && !mergedHeaders.traceparent) {
    mergedHeaders.traceparent = traceparent;
  }
  if (runId && !mergedHeaders["x-ctx-run-id"]) {
    mergedHeaders["x-ctx-run-id"] = runId;
  }
  return {
    "content-type": "application/json",
    ...(token ? { authorization: `Bearer ${token}` } : {}),
    ...mergedHeaders,
  };
};
