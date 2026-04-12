use std::fs as stdfs;
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use ctx_bundled_assets as bundled_assets;
use ctx_runtime_assets::download_managed_artifact;
use ctx_sandbox_contract::shared_vm_guest_host_share_path;
use flate2::read::GzDecoder;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use tar::{Archive, Builder};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::{fs, sync::Mutex};

use crate::{
    command_output_message, command_output_with_timeout, observe_log, observe_phase,
    observe_progress, sandbox_container_command, sandbox_engine_ready, sha256_hex_file,
    HarnessSetupLogLevel, HarnessSetupObserver, HarnessSetupPhase, HarnessSetupProgressUpdate,
    ManagedArtifactDownloadReporter, ManagedDownloadAggregate, SandboxCommandMode,
    DEFAULT_CONTAINER_IMAGE, SANDBOX_IMAGE_LOAD_TIMEOUT, SANDBOX_OP_TIMEOUT,
};

fn image_load_heartbeat_interval() -> Duration {
    if cfg!(test) {
        Duration::from_millis(100)
    } else {
        Duration::from_secs(5)
    }
}

fn image_load_poll_interval() -> Duration {
    if cfg!(test) {
        Duration::from_millis(25)
    } else {
        Duration::from_secs(1)
    }
}

fn image_post_load_visibility_timeout() -> Duration {
    if cfg!(test) {
        Duration::from_millis(500)
    } else {
        Duration::from_secs(10)
    }
}

fn format_image_load_elapsed(elapsed: Duration) -> String {
    let secs = elapsed.as_secs();
    let minutes = secs / 60;
    let seconds = secs % 60;
    if minutes == 0 {
        format!("{seconds}s")
    } else {
        format!("{minutes}m {seconds}s")
    }
}

pub fn default_container_image() -> &'static str {
    DEFAULT_CONTAINER_IMAGE
}

pub fn is_default_container_image(image: &str) -> bool {
    image.trim() == DEFAULT_CONTAINER_IMAGE
}

pub fn bundled_default_container_image_tar() -> Option<PathBuf> {
    bundled_assets::bundled_ctx_harness_image_tar(DEFAULT_CONTAINER_IMAGE)
}

pub async fn prefetch_container_startup_artifacts_with_observer(
    data_root: &Path,
    mode: &SandboxCommandMode,
    image: &str,
    observer: Option<&dyn HarnessSetupObserver>,
) -> Result<()> {
    prefetch_container_startup_artifacts_with_source_override(
        data_root, mode, image, None, observer,
    )
    .await
}

async fn prefetch_container_startup_artifacts_with_source_override(
    data_root: &Path,
    _mode: &SandboxCommandMode,
    image: &str,
    source_override: Option<&bundled_assets::ManagedArtifactSource>,
    observer: Option<&dyn HarnessSetupObserver>,
) -> Result<()> {
    let image = image.trim();
    if image.is_empty() {
        anyhow::bail!("image is required");
    }
    let prefetch_default_image_tar =
        image == DEFAULT_CONTAINER_IMAGE && bundled_default_container_image_tar().is_none();
    let image_tar_download = async {
        if prefetch_default_image_tar {
            return ensure_managed_default_container_image_tar_with_override(
                data_root,
                source_override,
                observer,
                Some(ManagedDownloadAggregate::default()),
            )
            .await
            .map(Some);
        }
        Ok(None)
    };
    let prefetched_image_tar = image_tar_download.await?;
    if let Some(prefetched_image_tar) = prefetched_image_tar {
        observe_log(
            observer,
            HarnessSetupPhase::ImageCheck,
            HarnessSetupLogLevel::Info,
            &format!(
                "prefetched default harness image tar at {}",
                prefetched_image_tar.display()
            ),
        );
    }
    Ok(())
}

pub async fn prefetch_container_image_with_observer(
    data_root: &Path,
    mode: &SandboxCommandMode,
    image: &str,
    observer: Option<&dyn HarnessSetupObserver>,
) -> Result<()> {
    let image = image.trim();
    if image.is_empty() {
        anyhow::bail!("image is required");
    }
    prefetch_container_startup_artifacts_with_source_override(
        data_root, mode, image, None, observer,
    )
    .await?;
    if !sandbox_engine_ready(data_root, mode).await.unwrap_or(false) {
        anyhow::bail!("native sandbox container runtime is not reachable for image prewarm");
    }
    observe_phase(
        observer,
        HarnessSetupPhase::ImageCheck,
        "checking harness image availability",
    );
    if container_image_present(data_root, mode, image).await? {
        observe_log(
            observer,
            HarnessSetupPhase::ImageCheck,
            HarnessSetupLogLevel::Info,
            "harness image already present",
        );
        return Ok(());
    }
    observe_phase(
        observer,
        HarnessSetupPhase::ImageLoad,
        "loading harness image into local sandbox runtime",
    );
    ensure_container_image_available(data_root, mode, image, observer).await
}

pub async fn prefetch_container_image(
    data_root: &Path,
    mode: &SandboxCommandMode,
    image: &str,
) -> Result<()> {
    prefetch_container_image_with_observer(data_root, mode, image, None).await
}

pub async fn container_image_present(
    data_root: &Path,
    mode: &SandboxCommandMode,
    image: &str,
) -> Result<bool> {
    let image = image.trim();
    if image.is_empty() {
        anyhow::bail!("image is required");
    }
    let mut cmd = sandbox_container_command(data_root, mode)?;
    cmd.arg("image").arg("inspect").arg(image);
    let output = command_output_with_timeout(cmd, SANDBOX_OP_TIMEOUT).await?;
    if output.status.success() {
        return Ok(true);
    }
    match output.status.code() {
        Some(1) => Ok(false),
        _ => anyhow::bail!(
            "container image inspect failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ),
    }
}

pub async fn ensure_container_image_available(
    data_root: &Path,
    mode: &SandboxCommandMode,
    image: &str,
    observer: Option<&dyn HarnessSetupObserver>,
) -> Result<()> {
    let image = image.trim();
    if image.is_empty() {
        anyhow::bail!("image is required");
    }

    if container_image_present(data_root, mode, image).await? {
        return Ok(());
    }

    if image == DEFAULT_CONTAINER_IMAGE {
        let image_tar = if let Some(tar) = bundled_assets::bundled_ctx_harness_image_tar(image) {
            observe_log(
                observer,
                HarnessSetupPhase::ImageLoad,
                HarnessSetupLogLevel::Info,
                &format!(
                    "loading default harness image from bundled tar {}",
                    tar.display()
                ),
            );
            tar
        } else {
            let managed_tar =
                ensure_managed_default_container_image_tar(data_root, observer).await?;
            observe_log(
                observer,
                HarnessSetupPhase::ImageLoad,
                HarnessSetupLogLevel::Info,
                &format!(
                    "loading default harness image from managed cache {}",
                    managed_tar.display()
                ),
            );
            managed_tar
        };
        load_container_image_tar(data_root, mode, &image_tar, image, observer).await?;
        return Ok(());
    }

    anyhow::bail!(
        "container image '{}' is not present; registry pulls are disabled, so the image must already exist in the local sandbox runtime",
        image
    );
}

pub async fn force_reload_default_container_image(
    data_root: &Path,
    mode: &SandboxCommandMode,
    observer: Option<&dyn HarnessSetupObserver>,
) -> Result<()> {
    let image_tar = if let Some(tar) =
        bundled_assets::bundled_ctx_harness_image_tar(DEFAULT_CONTAINER_IMAGE)
    {
        observe_log(
            observer,
            HarnessSetupPhase::ImageLoad,
            HarnessSetupLogLevel::Info,
            &format!(
                "reloading default harness image from bundled tar {}",
                tar.display()
            ),
        );
        tar
    } else {
        let managed_tar = ensure_managed_default_container_image_tar(data_root, observer).await?;
        observe_log(
            observer,
            HarnessSetupPhase::ImageLoad,
            HarnessSetupLogLevel::Info,
            &format!(
                "reloading default harness image from managed cache {}",
                managed_tar.display()
            ),
        );
        managed_tar
    };
    load_container_image_tar(
        data_root,
        mode,
        &image_tar,
        DEFAULT_CONTAINER_IMAGE,
        observer,
    )
    .await
}

fn managed_default_container_image_tar_path(data_root: &Path, sha256: &str) -> PathBuf {
    data_root
        .join("managed")
        .join("images")
        .join("ctx-harness")
        .join("linux")
        .join(std::env::consts::ARCH)
        .join(format!("sha256-{}.tar", sha256.trim().to_ascii_lowercase()))
}

fn normalized_shared_vm_container_image_tar_path(data_root: &Path, sha256: &str) -> PathBuf {
    data_root
        .join("managed")
        .join("images")
        .join("docker-archive")
        .join(format!("sha256-{}.tar", sha256.trim().to_ascii_lowercase()))
}

pub fn managed_default_image_install_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn shared_vm_image_archive_normalization_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

async fn ensure_managed_default_container_image_tar(
    data_root: &Path,
    observer: Option<&dyn HarnessSetupObserver>,
) -> Result<PathBuf> {
    ensure_managed_default_container_image_tar_with_override(data_root, None, observer, None).await
}

async fn ensure_managed_default_container_image_tar_with_override(
    data_root: &Path,
    source_override: Option<&bundled_assets::ManagedArtifactSource>,
    observer: Option<&dyn HarnessSetupObserver>,
    download_aggregate: Option<ManagedDownloadAggregate>,
) -> Result<PathBuf> {
    let source = match source_override.cloned() {
        Some(source) => source,
        None => bundled_assets::managed_ctx_harness_image_source(DEFAULT_CONTAINER_IMAGE)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "default harness image is missing from bundle and runtime lock managed sources"
                )
            })?,
    };
    ensure_managed_default_container_image_tar_with_source(
        data_root,
        &source,
        observer,
        download_aggregate,
    )
    .await
}

pub async fn ensure_managed_default_container_image_tar_with_source(
    data_root: &Path,
    source: &bundled_assets::ManagedArtifactSource,
    observer: Option<&dyn HarnessSetupObserver>,
    download_aggregate: Option<ManagedDownloadAggregate>,
) -> Result<PathBuf> {
    let _install_guard = managed_default_image_install_lock().lock().await;

    let final_tar = managed_default_container_image_tar_path(data_root, &source.sha256);
    if final_tar.exists() {
        let digest = sha256_hex_file(&final_tar)
            .await
            .with_context(|| format!("computing sha256 for {}", final_tar.display()))?;
        if digest.eq_ignore_ascii_case(source.sha256.trim()) {
            return Ok(final_tar);
        }
        observe_log(
            observer,
            HarnessSetupPhase::ImageLoad,
            HarnessSetupLogLevel::Warn,
            &format!(
                "managed image cache checksum mismatch for {}; re-downloading",
                final_tar.display()
            ),
        );
        let _ = fs::remove_file(&final_tar).await;
    }

    let Some(parent) = final_tar.parent() else {
        anyhow::bail!(
            "managed image cache path has no parent: {}",
            final_tar.display()
        );
    };
    fs::create_dir_all(parent)
        .await
        .with_context(|| format!("creating {}", parent.display()))?;
    let tmp_tar = final_tar.with_extension("download");

    observe_log(
        observer,
        HarnessSetupPhase::ArtifactDownload,
        HarnessSetupLogLevel::Info,
        &format!("downloading default harness image from {}", source.uri),
    );
    download_managed_artifact(
        &source.uri,
        &tmp_tar,
        Some(ManagedArtifactDownloadReporter::new(
            observer,
            download_aggregate,
            HarnessSetupPhase::ArtifactDownload,
            "Harness image",
        )),
    )
    .await?;

    let digest = sha256_hex_file(&tmp_tar)
        .await
        .with_context(|| format!("computing sha256 for {}", tmp_tar.display()))?;
    if !digest.eq_ignore_ascii_case(source.sha256.trim()) {
        let _ = fs::remove_file(&tmp_tar).await;
        anyhow::bail!(
            "managed harness image checksum mismatch: expected {}, got {}",
            source.sha256.trim(),
            digest
        );
    }
    fs::rename(&tmp_tar, &final_tar).await.with_context(|| {
        format!(
            "moving managed image tar into place: {} -> {}",
            tmp_tar.display(),
            final_tar.display()
        )
    })?;
    Ok(final_tar)
}

async fn prepare_container_image_tar_for_load(
    data_root: &Path,
    mode: &SandboxCommandMode,
    tar: &Path,
    observer: Option<&dyn HarnessSetupObserver>,
) -> Result<PathBuf> {
    if !matches!(mode, SandboxCommandMode::SharedVm { .. }) {
        return Ok(tar.to_path_buf());
    }

    let _guard = shared_vm_image_archive_normalization_lock().lock().await;
    let source_sha = sha256_hex_file(tar)
        .await
        .with_context(|| format!("computing sha256 for {}", tar.display()))?;
    let normalized_tar = normalized_shared_vm_container_image_tar_path(data_root, &source_sha);
    if normalized_tar.exists() {
        return Ok(normalized_tar);
    }

    let tar_path = tar.to_path_buf();
    let normalized_tar_path = normalized_tar.clone();
    let normalized = tokio::task::spawn_blocking(move || {
        normalize_oci_archive_to_docker_archive(&tar_path, &normalized_tar_path)
    })
    .await
    .context("joining shared VM image archive normalization task")??;

    if normalized {
        observe_log(
            observer,
            HarnessSetupPhase::ImageLoad,
            HarnessSetupLogLevel::Info,
            &format!(
                "normalized shared VM image archive to docker-archive {}",
                normalized_tar.display()
            ),
        );
        return Ok(normalized_tar);
    }

    Ok(tar.to_path_buf())
}

fn normalize_oci_archive_to_docker_archive(source: &Path, dest: &Path) -> Result<bool> {
    let Some(parent) = dest.parent() else {
        anyhow::bail!("normalized archive path has no parent: {}", dest.display());
    };
    stdfs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let staging_root = parent.join(format!(
        ".docker-archive-normalize-{}-{}",
        std::process::id(),
        nonce
    ));
    let extract_root = staging_root.join("extract");
    let docker_root = staging_root.join("docker-archive");
    let temp_tar = staging_root.join("normalized.tar");
    stdfs::create_dir_all(&extract_root)
        .with_context(|| format!("creating {}", extract_root.display()))?;

    let result = (|| -> Result<bool> {
        let source_file =
            stdfs::File::open(source).with_context(|| format!("opening {}", source.display()))?;
        let mut archive = Archive::new(source_file);
        archive
            .unpack(&extract_root)
            .with_context(|| format!("unpacking {}", source.display()))?;

        if !extract_root.join("oci-layout").is_file()
            || !extract_root.join("blobs").join("sha256").is_dir()
        {
            return Ok(false);
        }

        build_docker_archive_from_oci_extract(&extract_root, &docker_root)?;
        write_directory_to_tar(&docker_root, &temp_tar)?;
        stdfs::rename(&temp_tar, dest).with_context(|| {
            format!(
                "moving normalized docker archive into place: {} -> {}",
                temp_tar.display(),
                dest.display()
            )
        })?;
        Ok(true)
    })();

    let _ = stdfs::remove_dir_all(&staging_root);
    result
}

fn build_docker_archive_from_oci_extract(extract_root: &Path, docker_root: &Path) -> Result<()> {
    stdfs::create_dir_all(docker_root)
        .with_context(|| format!("creating {}", docker_root.display()))?;

    let manifest_path = extract_root.join("manifest.json");
    let manifest: Value = serde_json::from_slice(
        &stdfs::read(&manifest_path)
            .with_context(|| format!("reading {}", manifest_path.display()))?,
    )
    .with_context(|| format!("parsing {}", manifest_path.display()))?;
    let manifest_entries = manifest
        .as_array()
        .context("manifest.json did not contain an array")?;
    let manifest_entry = manifest_entries
        .first()
        .context("manifest.json did not contain any image entries")?;

    let config_rel = manifest_entry
        .get("Config")
        .and_then(Value::as_str)
        .context("manifest.json entry missing Config")?;
    let repo_tags = manifest_entry
        .get("RepoTags")
        .and_then(Value::as_array)
        .context("manifest.json entry missing RepoTags")?
        .iter()
        .filter_map(Value::as_str)
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let layer_rels = manifest_entry
        .get("Layers")
        .and_then(Value::as_array)
        .context("manifest.json entry missing Layers")?
        .iter()
        .filter_map(Value::as_str)
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();

    let config_source = extract_root.join(config_rel);
    let config_stem = config_source
        .file_name()
        .and_then(|value| value.to_str())
        .context("config path in manifest.json had no file name")?;
    let config_name = format!("{config_stem}.json");
    stdfs::copy(&config_source, docker_root.join(&config_name)).with_context(|| {
        format!(
            "copying image config {} into docker archive root",
            config_source.display()
        )
    })?;

    let mut docker_layers = Vec::with_capacity(layer_rels.len());
    for (index, layer_rel) in layer_rels.iter().enumerate() {
        let layer_source = extract_root.join(layer_rel);
        let mut hasher = Sha256::new();
        hasher.update(index.to_string().as_bytes());
        hasher.update(b":");
        hasher.update(layer_rel.as_bytes());
        let layer_id = hex::encode(hasher.finalize());
        let layer_root = docker_root.join(&layer_id);
        stdfs::create_dir_all(&layer_root)
            .with_context(|| format!("creating {}", layer_root.display()))?;
        write_layer_tar_payload(&layer_source, &layer_root.join("layer.tar"))?;
        stdfs::write(layer_root.join("VERSION"), b"1.0")
            .with_context(|| format!("writing {}/VERSION", layer_root.display()))?;
        stdfs::write(layer_root.join("json"), b"{}")
            .with_context(|| format!("writing {}/json", layer_root.display()))?;
        docker_layers.push(format!("{layer_id}/layer.tar"));
    }

    let docker_manifest = vec![json!({
        "Config": config_name.clone(),
        "RepoTags": repo_tags.clone(),
        "Layers": docker_layers,
    })];
    stdfs::write(
        docker_root.join("manifest.json"),
        serde_json::to_vec(&docker_manifest).context("serializing docker archive manifest.json")?,
    )
    .with_context(|| format!("writing {}/manifest.json", docker_root.display()))?;

    let config_id = config_name.trim_end_matches(".json");
    let mut repositories = Map::new();
    for repo_tag in repo_tags {
        if let Some((repo, tag)) = split_repo_tag(&repo_tag) {
            let entry = repositories
                .entry(repo.to_string())
                .or_insert_with(|| Value::Object(Map::new()));
            let Some(tags) = entry.as_object_mut() else {
                anyhow::bail!("docker repositories entry for {repo} was not an object");
            };
            tags.insert(tag.to_string(), Value::String(config_id.to_string()));
        }
    }
    stdfs::write(
        docker_root.join("repositories"),
        serde_json::to_vec(&Value::Object(repositories))
            .context("serializing docker archive repositories")?,
    )
    .with_context(|| format!("writing {}/repositories", docker_root.display()))?;

    Ok(())
}

fn split_repo_tag(reference: &str) -> Option<(&str, &str)> {
    let (repo, tag) = reference.rsplit_once(':')?;
    if repo.is_empty() || tag.is_empty() || tag.contains('/') {
        return None;
    }
    Some((repo, tag))
}

fn write_layer_tar_payload(source: &Path, dest: &Path) -> Result<()> {
    let mut prefix = [0u8; 2];
    let mut source_probe =
        stdfs::File::open(source).with_context(|| format!("opening {}", source.display()))?;
    let prefix_len = source_probe
        .read(&mut prefix)
        .with_context(|| format!("reading {}", source.display()))?;
    drop(source_probe);

    let source_file =
        stdfs::File::open(source).with_context(|| format!("opening {}", source.display()))?;
    let mut reader: Box<dyn Read> = if prefix_len == 2 && prefix == [0x1f, 0x8b] {
        Box::new(GzDecoder::new(BufReader::new(source_file)))
    } else {
        Box::new(BufReader::new(source_file))
    };
    let mut output =
        stdfs::File::create(dest).with_context(|| format!("creating {}", dest.display()))?;
    std::io::copy(&mut reader, &mut output).with_context(|| {
        format!(
            "writing docker archive layer payload {} from {}",
            dest.display(),
            source.display()
        )
    })?;
    output
        .flush()
        .with_context(|| format!("flushing {}", dest.display()))?;
    Ok(())
}

fn write_directory_to_tar(source_root: &Path, tar_path: &Path) -> Result<()> {
    let tar_file = stdfs::File::create(tar_path)
        .with_context(|| format!("creating {}", tar_path.display()))?;
    let mut builder = Builder::new(tar_file);
    append_directory_tree(&mut builder, source_root, source_root)?;
    builder
        .finish()
        .with_context(|| format!("finalizing {}", tar_path.display()))?;
    Ok(())
}

fn append_directory_tree(
    builder: &mut Builder<stdfs::File>,
    source_root: &Path,
    current: &Path,
) -> Result<()> {
    let mut entries = stdfs::read_dir(current)
        .with_context(|| format!("reading {}", current.display()))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .with_context(|| format!("collecting entries for {}", current.display()))?;
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let path = entry.path();
        let rel = path.strip_prefix(source_root).with_context(|| {
            format!(
                "stripping {} from {}",
                source_root.display(),
                path.display()
            )
        })?;
        let file_type = entry
            .file_type()
            .with_context(|| format!("reading type for {}", path.display()))?;
        if file_type.is_dir() {
            builder
                .append_dir(rel, &path)
                .with_context(|| format!("adding directory {} to archive", rel.display()))?;
            append_directory_tree(builder, source_root, &path)?;
        } else {
            builder
                .append_path_with_name(&path, rel)
                .with_context(|| format!("adding file {} to archive", rel.display()))?;
        }
    }

    Ok(())
}

async fn read_child_pipe<R>(mut reader: R) -> std::io::Result<Vec<u8>>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut buf = Vec::new();
    reader.read_to_end(&mut buf).await?;
    Ok(buf)
}

async fn load_container_image_tar(
    data_root: &Path,
    mode: &SandboxCommandMode,
    tar: &Path,
    image: &str,
    observer: Option<&dyn HarnessSetupObserver>,
) -> Result<()> {
    let prepared_tar = prepare_container_image_tar_for_load(data_root, mode, tar, observer).await?;
    let mut cmd = sandbox_container_command(data_root, mode)?;
    let guest_tar_path = matches!(mode, SandboxCommandMode::SharedVm { .. })
        .then(|| shared_vm_guest_host_share_path(data_root, &prepared_tar))
        .flatten();
    let stream_tar_over_stdin =
        matches!(mode, SandboxCommandMode::SharedVm { .. }) && guest_tar_path.is_none();
    cmd.arg("load");
    if let Some(guest_tar_path) = guest_tar_path.as_ref() {
        cmd.arg("-i").arg(guest_tar_path);
    } else if !stream_tar_over_stdin {
        cmd.arg("-i").arg(&prepared_tar);
    }
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    if stream_tar_over_stdin {
        observe_log(
            observer,
            HarnessSetupPhase::ImageLoad,
            HarnessSetupLogLevel::Info,
            &format!(
                "streaming harness image tar into shared VM because {} is outside the shared data root",
                prepared_tar.display()
            ),
        );
        cmd.stdin(Stdio::piped());
    }
    cmd.kill_on_drop(true);
    let mut child = cmd.spawn().with_context(|| {
        format!(
            "spawning container image load for {}",
            prepared_tar.display()
        )
    })?;
    let stdin_task = if stream_tar_over_stdin {
        let mut stdin = child
            .stdin
            .take()
            .context("container image load stdin was not captured")?;
        let tar_path = prepared_tar.to_path_buf();
        Some(tokio::spawn(async move {
            let mut file = fs::File::open(&tar_path).await?;
            tokio::io::copy(&mut file, &mut stdin).await?;
            stdin.shutdown().await
        }))
    } else {
        None
    };
    let stdout = child
        .stdout
        .take()
        .context("container image load stdout was not captured")?;
    let stderr = child
        .stderr
        .take()
        .context("container image load stderr was not captured")?;
    let stdout_task = tokio::spawn(read_child_pipe(stdout));
    let stderr_task = tokio::spawn(read_child_pipe(stderr));
    let deadline = tokio::time::Instant::now() + SANDBOX_IMAGE_LOAD_TIMEOUT;
    let started = tokio::time::Instant::now();
    let mut last_heartbeat = started;

    let output = loop {
        if let Some(status) = child
            .try_wait()
            .context("polling container image load process")?
        {
            let stdout = stdout_task
                .await
                .context("joining container image load stdout capture")??;
            let stderr = stderr_task
                .await
                .context("joining container image load stderr capture")??;
            if let Some(stdin_task) = stdin_task {
                match stdin_task
                    .await
                    .context("joining container image load stdin stream")?
                {
                    Ok(()) => {}
                    Err(err) if err.kind() == std::io::ErrorKind::BrokenPipe => {}
                    Err(err) => {
                        return Err(err)
                            .context("streaming container image tar to sandbox CLI stdin");
                    }
                }
            }
            break std::process::Output {
                status,
                stdout,
                stderr,
            };
        }

        if tokio::time::Instant::now() >= deadline {
            let _ = child.start_kill();
            let status = child
                .wait()
                .await
                .context("waiting for timed out container image load")?;
            let stdout = stdout_task
                .await
                .context("joining timed out container image load stdout capture")??;
            let stderr = stderr_task
                .await
                .context("joining timed out container image load stderr capture")??;
            if let Some(stdin_task) = stdin_task {
                match stdin_task
                    .await
                    .context("joining timed out container image load stdin stream")?
                {
                    Ok(()) => {}
                    Err(err) if err.kind() == std::io::ErrorKind::BrokenPipe => {}
                    Err(err) => {
                        return Err(err).context(
                            "streaming container image tar to timed out sandbox CLI stdin",
                        );
                    }
                }
            }
            let output = std::process::Output {
                status,
                stdout,
                stderr,
            };
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            if stderr.is_empty() {
                anyhow::bail!(
                    "container image load timed out after {}s",
                    SANDBOX_IMAGE_LOAD_TIMEOUT.as_secs()
                );
            }
            anyhow::bail!(
                "container image load timed out after {}s: {stderr}",
                SANDBOX_IMAGE_LOAD_TIMEOUT.as_secs()
            );
        }

        let now = tokio::time::Instant::now();
        if now.duration_since(last_heartbeat) >= image_load_heartbeat_interval() {
            observe_log(
                observer,
                HarnessSetupPhase::ImageLoad,
                HarnessSetupLogLevel::Info,
                &format!(
                    "still loading harness image into local sandbox runtime ({} elapsed)",
                    format_image_load_elapsed(started.elapsed())
                ),
            );
            observe_progress(
                observer,
                HarnessSetupProgressUpdate {
                    phase: HarnessSetupPhase::ImageLoad,
                    active_download: None,
                },
            );
            last_heartbeat = now;
        }

        tokio::time::sleep(image_load_poll_interval()).await;
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            anyhow::bail!("container image load failed (status: {})", output.status);
        }
        anyhow::bail!("container image load failed: {stderr}");
    }
    let load_message = command_output_message(&output);
    let visibility_deadline = tokio::time::Instant::now() + image_post_load_visibility_timeout();
    while tokio::time::Instant::now() < visibility_deadline {
        if container_image_present(data_root, mode, image).await? {
            return Ok(());
        }
        tokio::time::sleep(image_load_poll_interval()).await;
    }
    if load_message.is_empty() {
        anyhow::bail!(
            "container image load reported success but image '{}' is still missing after {}s",
            image,
            image_post_load_visibility_timeout().as_secs()
        );
    }
    anyhow::bail!(
        "container image load reported success but image '{}' is still missing after {}s: {}",
        image,
        image_post_load_visibility_timeout().as_secs(),
        load_message
    );
}

#[derive(Debug, Clone)]
pub struct ContainerImageStatus {
    pub present: bool,
    pub available: bool,
    pub error: Option<String>,
}

pub async fn container_image_status(
    data_root: &Path,
    mode: &SandboxCommandMode,
    image: &str,
) -> Result<ContainerImageStatus> {
    let image = image.trim();
    if image.is_empty() {
        anyhow::bail!("image is required");
    }
    let output = match sandbox_container_command(data_root, mode) {
        Ok(mut cmd) => {
            cmd.arg("image").arg("inspect").arg(image);
            command_output_with_timeout(cmd, SANDBOX_OP_TIMEOUT).await
        }
        Err(err) => {
            return Ok(ContainerImageStatus {
                present: false,
                available: false,
                error: Some(err.to_string()),
            });
        }
    };
    let output = match output {
        Ok(out) => out,
        Err(err) => {
            return Ok(ContainerImageStatus {
                present: false,
                available: false,
                error: Some(err.to_string()),
            });
        }
    };
    if output.status.success() {
        return Ok(ContainerImageStatus {
            present: true,
            available: true,
            error: None,
        });
    }
    match output.status.code() {
        Some(1) => Ok(ContainerImageStatus {
            present: false,
            available: true,
            error: None,
        }),
        _ => Ok(ContainerImageStatus {
            present: false,
            available: false,
            error: Some(String::from_utf8_lossy(&output.stderr).trim().to_string()),
        }),
    }
}

#[cfg(test)]
#[path = "image_tests.rs"]
mod tests;
