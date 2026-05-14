use serde_json::Value;

pub struct InterruptedTurnTerminalization<'a> {
    pub reason: &'a str,
    pub provider_cancelled: bool,
    pub emit_interrupt_event: bool,
}

pub struct FailedTurnTerminalization<'a> {
    pub message: &'a str,
    pub reason: Option<&'a str>,
    pub details: Option<Value>,
    pub kind: Option<Value>,
}
