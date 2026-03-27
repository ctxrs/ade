pub(super) fn desktop_dev_instance_id() -> &'static str {
    option_env!("CTX_DEV_INSTANCE_ID").unwrap_or("unknown")
}
