use chrono::{DateTime, Utc};
use ctx_settings_model::default_telemetry_endpoint;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

pub type TelemetryProperties = Map<String, Value>;

#[derive(Debug, Clone)]
pub struct TelemetryConfig {
    pub enabled: bool,
    pub endpoint: String,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            endpoint: default_telemetry_endpoint(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TelemetryPlane {
    Product,
    Incident,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TelemetryDelivery {
    Remote,
    LocalOnly,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TelemetryOriginRuntime {
    Web,
    Desktop,
    MobileShell,
    Daemon,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryEvent {
    pub event_id: String,
    pub event_name: String,
    pub event_version: u32,
    pub occurred_at: DateTime<Utc>,
    pub plane: TelemetryPlane,
    pub delivery: TelemetryDelivery,
    pub origin_runtime: TelemetryOriginRuntime,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin_install_id: Option<String>,
    pub app_version: String,
    pub os: String,
    pub arch: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub surface: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env_target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "TelemetryProperties::is_empty")]
    pub properties: TelemetryProperties,
}

impl TelemetryEvent {
    pub fn daemon_product(event_name: impl Into<String>) -> Self {
        Self::daemon_event(event_name, TelemetryPlane::Product)
    }

    pub fn daemon_incident(event_name: impl Into<String>) -> Self {
        Self::daemon_event(event_name, TelemetryPlane::Incident)
    }

    fn daemon_event(event_name: impl Into<String>, plane: TelemetryPlane) -> Self {
        Self {
            event_id: uuid::Uuid::new_v4().to_string(),
            event_name: event_name.into(),
            event_version: 1,
            occurred_at: Utc::now(),
            plane,
            delivery: TelemetryDelivery::Remote,
            origin_runtime: TelemetryOriginRuntime::Daemon,
            origin_install_id: None,
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            surface: None,
            env_target: None,
            source: None,
            properties: TelemetryProperties::new(),
        }
    }

    pub fn local_only(mut self) -> Self {
        self.delivery = TelemetryDelivery::LocalOnly;
        self
    }

    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        let source = source.into();
        if !source.trim().is_empty() {
            self.source = Some(source);
        }
        self
    }

    pub fn with_surface(mut self, surface: impl Into<String>) -> Self {
        let surface = surface.into();
        if !surface.trim().is_empty() {
            self.surface = Some(surface);
        }
        self
    }

    pub fn with_env_target(mut self, env_target: impl Into<String>) -> Self {
        let env_target = env_target.into();
        if !env_target.trim().is_empty() {
            self.env_target = Some(env_target);
        }
        self
    }

    pub fn with_property(mut self, key: impl Into<String>, value: Value) -> Self {
        let key = key.into();
        if key.trim().is_empty() {
            return self;
        }
        self.properties.insert(key, value);
        self
    }

    pub fn with_properties(mut self, properties: TelemetryProperties) -> Self {
        self.properties.extend(properties);
        self
    }

    pub fn workspace_registered() -> Self {
        Self::daemon_product("workspace_registered")
    }

    pub fn workspace_opened() -> Self {
        Self::daemon_product("workspace_opened")
    }

    pub fn session_started(
        provider_id: String,
        model_id: String,
        execution_environment: Option<String>,
        session_root_kind: Option<String>,
    ) -> Self {
        let mut event = Self::daemon_product("session_started")
            .with_property("provider_id", json!(provider_id))
            .with_property("model_id", json!(model_id));
        if let Some(env_target) = execution_environment {
            event = event.with_env_target(env_target.clone());
            event = event.with_property("execution_environment", json!(env_target));
        }
        if let Some(session_root_kind) = session_root_kind {
            event = event.with_property("session_root_kind", json!(session_root_kind));
        }
        event
    }

    pub fn session_completed(
        provider_id: String,
        model_id: String,
        execution_environment: Option<String>,
        session_root_kind: Option<String>,
        status: String,
        duration_ms: u64,
    ) -> Self {
        let mut event = Self::daemon_product("session_completed")
            .with_property("provider_id", json!(provider_id))
            .with_property("model_id", json!(model_id))
            .with_property("status", json!(status))
            .with_property("duration_ms", json!(duration_ms));
        if let Some(env_target) = execution_environment {
            event = event.with_env_target(env_target.clone());
            event = event.with_property("execution_environment", json!(env_target));
        }
        if let Some(session_root_kind) = session_root_kind {
            event = event.with_property("session_root_kind", json!(session_root_kind));
        }
        event
    }

    pub fn session_interrupt_latency(
        provider_id: String,
        model_id: String,
        execution_environment: Option<String>,
        session_root_kind: Option<String>,
        duration_ms: u64,
        duration_bucket: String,
    ) -> Self {
        let mut event = Self::daemon_product("session_interrupt_latency")
            .with_property("provider_id", json!(provider_id))
            .with_property("model_id", json!(model_id))
            .with_property("duration_ms", json!(duration_ms))
            .with_property("duration_bucket", json!(duration_bucket))
            .with_property("status", json!("interrupted"))
            .with_property("success", json!(true));
        if let Some(env_target) = execution_environment {
            event = event.with_env_target(env_target.clone());
            event = event.with_property("execution_environment", json!(env_target));
        }
        if let Some(session_root_kind) = session_root_kind {
            event = event.with_property("session_root_kind", json!(session_root_kind));
        }
        event
    }

    pub fn provider_call(
        provider_id: String,
        model_id: String,
        execution_environment: Option<String>,
        session_root_kind: Option<String>,
        success: bool,
        duration_ms: u64,
    ) -> Self {
        let mut event = Self::daemon_product("provider_call")
            .with_property("provider_id", json!(provider_id))
            .with_property("model_id", json!(model_id))
            .with_property("success", json!(success))
            .with_property("duration_ms", json!(duration_ms));
        if let Some(env_target) = execution_environment {
            event = event.with_env_target(env_target.clone());
            event = event.with_property("execution_environment", json!(env_target));
        }
        if let Some(session_root_kind) = session_root_kind {
            event = event.with_property("session_root_kind", json!(session_root_kind));
        }
        event
    }
}
