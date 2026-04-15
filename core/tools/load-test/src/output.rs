use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::metrics::Summary;

#[derive(Serialize)]
pub(crate) struct EventRecord<'a> {
    pub(crate) ts_ms: u128,
    pub(crate) kind: &'a str,
    pub(crate) value_ms: Option<f64>,
    pub(crate) detail: Option<String>,
}

pub(crate) struct DaemonRunOutput {
    pub(crate) summary_path: PathBuf,
    pub(crate) summary: Summary,
    pub(crate) tasks: usize,
    pub(crate) sessions: usize,
}

#[derive(Clone)]
pub(crate) struct EventWriter {
    writer: Arc<Mutex<BufWriter<File>>>,
}

impl EventWriter {
    pub(crate) fn new(path: PathBuf) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("creating event dir")?;
        }
        let file = File::create(path).context("creating event log")?;
        Ok(Self {
            writer: Arc::new(Mutex::new(BufWriter::new(file))),
        })
    }

    pub(crate) fn write(&mut self, record: EventRecord<'_>) -> Result<()> {
        let line = serde_json::to_string(&record)?;
        let mut guard = self.writer.lock().unwrap();
        guard.write_all(line.as_bytes())?;
        guard.write_all(b"\n")?;
        Ok(())
    }
}

pub(crate) fn resolve_out_dir(name: &str, override_dir: Option<&PathBuf>) -> Result<PathBuf> {
    if let Some(dir) = override_dir {
        return Ok(dir.clone());
    }
    let stamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    Ok(PathBuf::from(format!("/tmp/ctx-load-test/{name}_{stamp}")))
}
