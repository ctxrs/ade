use super::*;

pub async fn self_update_daemon(
    channel: &str,
    base_url: &str,
    current_version: &str,
    yes: bool,
    check_only: bool,
) -> Result<()> {
    let channel = normalize_release_channel(channel)?;
    let Some(platform) = platform_key() else {
        anyhow::bail!(
            "unsupported platform for self-update: {}/{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        );
    };

    let current_version =
        normalize_version_str(current_version).context("parsing current version")?;

    let manifest = fetch_latest_manifest(base_url, &channel).await?;
    let latest_version = normalize_version_str(&manifest.latest_version)
        .with_context(|| format!("parsing latest_version: {}", manifest.latest_version))?;

    let update_available = latest_version > current_version;
    println!(
        "Current version: {}\nLatest version:  {}\nChannel:         {}\nUpdate:          {}",
        current_version,
        latest_version,
        &channel,
        if update_available {
            "available"
        } else {
            "none"
        }
    );

    if !update_available || check_only {
        return Ok(());
    }

    let platform_entry = manifest
        .platforms
        .get(platform)
        .with_context(|| format!("manifest missing platform entry: {platform}"))?;
    let artifact = platform_entry
        .daemon
        .as_ref()
        .context("manifest missing daemon artifact for this platform")?;

    if !yes {
        if !atty::is(atty::Stream::Stdin) {
            anyhow::bail!("refusing to self-update non-interactively without --yes");
        }
        use std::io::Write;
        print!("Proceed to download and replace this binary? [y/N] ");
        std::io::stdout().flush().ok();
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).ok();
        let ok = matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes");
        if !ok {
            println!("Aborted.");
            return Ok(());
        }
    }

    let download_url = resolve_release_artifact_url(base_url, &artifact.url_path)?;
    let tmp_dir = std::env::temp_dir().join("ctx-self-update");
    tokio::fs::create_dir_all(&tmp_dir).await.ok();
    let tmp_path = tmp_dir.join("ctx.new");

    println!("Downloading: {download_url}");
    download_to_path(&download_url, &tmp_path).await?;

    let got = sha256_hex_file(&tmp_path).await?;
    if !got.eq_ignore_ascii_case(&artifact.sha256) {
        anyhow::bail!(
            "checksum mismatch for downloaded binary: expected {}, got {}",
            artifact.sha256,
            got
        );
    }

    let current_exe = std::env::current_exe().context("resolving current executable path")?;
    atomic_replace_exe(&current_exe, &tmp_path).await?;

    println!(
        "Updated successfully. New binary is in place at {}",
        current_exe.display()
    );
    Ok(())
}
