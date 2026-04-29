export class RelayHttpError extends Error {
  constructor(
    public readonly status: number,
    public readonly code: string,
    message: string,
  ) {
    super(message);
  }
}

export function jsonError(error: unknown): Response {
  if (error instanceof RelayHttpError) {
    return Response.json({ code: error.code, error: error.message }, { status: error.status });
  }
  return Response.json(
    { code: "internal_error", error: "relay request failed" },
    { status: 500 },
  );
}

export function requireString(value: unknown, name: string): string {
  if (typeof value !== "string" || value.trim() === "") {
    throw new RelayHttpError(500, "missing_config", `${name} is required`);
  }
  return value.trim();
}
