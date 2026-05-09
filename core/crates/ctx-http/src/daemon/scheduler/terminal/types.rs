use serde_json::Value;

pub(crate) struct InterruptedTurnTerminalization<'a> {
    pub(crate) reason: &'a str,
    pub(crate) provider_cancelled: bool,
    pub(crate) emit_interrupt_event: bool,
}

pub(crate) struct FailedTurnTerminalization<'a> {
    pub(crate) message: &'a str,
    pub(crate) reason: Option<&'a str>,
    pub(crate) details: Option<Value>,
    pub(crate) kind: Option<Value>,
}
