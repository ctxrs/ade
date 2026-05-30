import type { TelemetryRow } from "./telemetry-ingest";

export interface TelemetryDatabase {
  insertTelemetryRows(rows: readonly TelemetryRow[]): Promise<Set<string>>;
}

export type TelemetryDatabaseFactory = (
  databaseUrl: string,
) => Promise<TelemetryDatabase> | TelemetryDatabase;

type NeonParam = string | number | boolean | null;
type NeonRow = Record<string, unknown>;
type NeonQueryFunction = {
  query<T extends NeonRow = NeonRow>(
    query: string,
    params?: readonly NeonParam[],
  ): Promise<T[]>;
};
type NeonModule = {
  neon(connectionString: string): NeonQueryFunction;
};

const TELEMETRY_COLUMNS = [
  "event_id",
  "install_id_hash",
  "broker_install_id_hash",
  "origin_install_id_hash",
  "occurred_at",
  "event_name",
  "event_version",
  "plane",
  "broker_runtime",
  "origin_runtime",
  "source",
  "analytics_environment",
  "traffic_class",
  "app_version",
  "os",
  "arch",
  "surface",
  "env_target",
  "provider_id",
  "model_id",
  "duration_ms",
  "duration_bucket",
  "status",
  "success",
  "session_root_kind",
  "properties",
] as const;

type TelemetryColumn = typeof TELEMETRY_COLUMNS[number];

export async function createNeonTelemetryDatabase(
  databaseUrl: string,
): Promise<TelemetryDatabase> {
  const neonModule = await import("@neondatabase/serverless") as NeonModule;
  return new NeonTelemetryDatabase(neonModule.neon(databaseUrl));
}

class NeonTelemetryDatabase implements TelemetryDatabase {
  constructor(private readonly sql: NeonQueryFunction) {}

  async insertTelemetryRows(rows: readonly TelemetryRow[]): Promise<Set<string>> {
    if (rows.length === 0) return new Set();

    const params: NeonParam[] = [];
    const valuesSql = rows.map((row) => {
      const placeholders = TELEMETRY_COLUMNS.map((column) => {
        params.push(valueForColumn(row, column));
        const placeholder = `$${params.length}`;
        if (column === "occurred_at") return `${placeholder}::timestamptz`;
        if (column === "properties") return `${placeholder}::jsonb`;
        return placeholder;
      });
      return `(${placeholders.join(", ")})`;
    });
    const query = `
      INSERT INTO telemetry_event (${TELEMETRY_COLUMNS.join(", ")})
      VALUES ${valuesSql.join(", ")}
      ON CONFLICT (event_id) DO NOTHING
      RETURNING event_id
    `;
    const inserted = await this.sql.query<{ event_id: unknown }>(query, params);
    return new Set(
      inserted
        .map((row) => row.event_id)
        .filter((eventId): eventId is string => typeof eventId === "string"),
    );
  }
}

function valueForColumn(row: TelemetryRow, column: TelemetryColumn): NeonParam {
  switch (column) {
    case "event_id":
      return row.event_id;
    case "install_id_hash":
      return row.install_id_hash;
    case "broker_install_id_hash":
      return row.broker_install_id_hash;
    case "origin_install_id_hash":
      return row.origin_install_id_hash;
    case "occurred_at":
      return row.occurred_at;
    case "event_name":
      return row.event_name;
    case "event_version":
      return row.event_version;
    case "plane":
      return row.plane;
    case "broker_runtime":
      return row.broker_runtime;
    case "origin_runtime":
      return row.origin_runtime;
    case "source":
      return row.source;
    case "analytics_environment":
      return row.analytics_environment;
    case "traffic_class":
      return row.traffic_class;
    case "app_version":
      return row.app_version;
    case "os":
      return row.os;
    case "arch":
      return row.arch;
    case "surface":
      return row.surface;
    case "env_target":
      return row.env_target;
    case "provider_id":
      return row.provider_id;
    case "model_id":
      return row.model_id;
    case "duration_ms":
      return row.duration_ms;
    case "duration_bucket":
      return row.duration_bucket;
    case "status":
      return row.status;
    case "success":
      return row.success;
    case "session_root_kind":
      return row.session_root_kind;
    case "properties":
      return JSON.stringify(row.properties);
  }
}
