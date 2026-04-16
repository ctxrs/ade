use super::*;

fn desktop_logs_dir() -> Result<PathBuf> {
    Ok(desktop_local_data_root()?.join("logs"))
}

fn desktop_log_path() -> Result<PathBuf> {
    Ok(desktop_logs_dir()?.join("desktop.log"))
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn append_native_desktop_log_line(message: &str, level: &str) -> Result<()> {
    if cfg!(test) {
        return Ok(());
    }
    let dir = desktop_logs_dir()?;
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating desktop log dir at {}", dir.display()))?;
    let path = desktop_log_path()?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("opening desktop log at {}", path.display()))?;
    writeln!(file, "{} [{}] {}", now_ms(), level, message)
        .with_context(|| format!("writing desktop log at {}", path.display()))?;
    file.flush()
        .with_context(|| format!("flushing desktop log at {}", path.display()))?;
    Ok(())
}

fn log_native_desktop_line(level: &str, message: &str) {
    if let Err(err) = append_native_desktop_log_line(message, level) {
        eprintln!("desktop_startup: log_write_failed level={level} error={err:#}");
    }
    eprintln!("{message}");
}

pub(super) fn log_desktop_startup(message: &str) {
    log_native_desktop_line("info", message);
}

pub(super) fn log_desktop_startup_error(message: &str) {
    log_native_desktop_line("error", message);
}
