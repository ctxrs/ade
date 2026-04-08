use std::fs;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};

fn main() -> Result<()> {
    let mut check = false;
    for arg in std::env::args().skip(1) {
        if arg == "--check" {
            check = true;
            continue;
        }
        bail!("unsupported argument: {arg}");
    }

    let manifest_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").context("missing CARGO_MANIFEST_DIR")?);
    let output_path = manifest_dir
        .join("../../apps/web/src/generated/desktop-ipc.ts")
        .canonicalize()
        .unwrap_or_else(|_| manifest_dir.join("../../apps/web/src/generated/desktop-ipc.ts"));
    let generated = ctx_desktop_ipc::typescript_declarations();

    if check {
        let existing = fs::read_to_string(&output_path)
            .with_context(|| format!("reading {}", output_path.display()))?;
        if existing != generated {
            bail!(
                "desktop IPC TypeScript bindings are stale: regenerate {}",
                output_path.display()
            );
        }
        return Ok(());
    }

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    fs::write(&output_path, generated)
        .with_context(|| format!("writing {}", output_path.display()))?;
    Ok(())
}
