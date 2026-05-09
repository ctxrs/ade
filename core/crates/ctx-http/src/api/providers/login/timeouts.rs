use std::time::Duration;

const GEMINI_LOGIN_TIMEOUT_DEFAULT: Duration = Duration::from_secs(300);
const QWEN_LOGIN_TIMEOUT_DEFAULT: Duration = Duration::from_secs(300);
const AMP_LOGIN_TIMEOUT_DEFAULT: Duration = Duration::from_secs(300);
const MISTRAL_LOGIN_TIMEOUT_DEFAULT: Duration = Duration::from_secs(300);

pub(in crate::api::providers) fn gemini_login_timeout() -> Duration {
    login_timeout_from_env(
        "CTX_GEMINI_LOGIN_TIMEOUT_SECS",
        GEMINI_LOGIN_TIMEOUT_DEFAULT,
    )
}

pub(in crate::api::providers) fn qwen_login_timeout() -> Duration {
    login_timeout_from_env("CTX_QWEN_LOGIN_TIMEOUT_SECS", QWEN_LOGIN_TIMEOUT_DEFAULT)
}

pub(in crate::api::providers) fn amp_login_timeout() -> Duration {
    login_timeout_from_env("CTX_AMP_LOGIN_TIMEOUT_SECS", AMP_LOGIN_TIMEOUT_DEFAULT)
}

pub(in crate::api::providers) fn mistral_login_timeout() -> Duration {
    login_timeout_from_env(
        "CTX_MISTRAL_LOGIN_TIMEOUT_SECS",
        MISTRAL_LOGIN_TIMEOUT_DEFAULT,
    )
}

fn login_timeout_from_env(env_key: &str, default: Duration) -> Duration {
    let seconds = std::env::var(env_key)
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default.as_secs());
    Duration::from_secs(seconds)
}
