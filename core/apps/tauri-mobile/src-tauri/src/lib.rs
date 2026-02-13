#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
  if let Err(err) = tauri::Builder::default()
    .setup(|app| {
      if cfg!(debug_assertions) {
        app.handle().plugin(
          tauri_plugin_log::Builder::default()
            .level(log::LevelFilter::Info)
            .build(),
        )?;
      }
      Ok(())
    })
    .run(tauri::generate_context!()) {
      eprintln!("error while running tauri application: {err}");
      std::process::exit(1);
    }
}
