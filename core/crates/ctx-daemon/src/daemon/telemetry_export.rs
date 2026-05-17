use crate::daemon::CoreHandle;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelemetryExportErrorKind {
    NotFound,
}

#[derive(Debug, Clone)]
pub struct TelemetryExportError {
    kind: TelemetryExportErrorKind,
}

impl TelemetryExportError {
    fn not_found() -> Self {
        Self {
            kind: TelemetryExportErrorKind::NotFound,
        }
    }

    pub fn kind(&self) -> TelemetryExportErrorKind {
        self.kind
    }
}

impl CoreHandle {
    pub async fn read_perf_telemetry_export_for_date(
        &self,
        date: &str,
    ) -> Result<Vec<u8>, TelemetryExportError> {
        let path =
            ctx_observability::perf_telemetry::perf_log_path_for_date(self.data_root(), date);
        tokio::fs::read(&path)
            .await
            .map_err(|_| TelemetryExportError::not_found())
    }
}
