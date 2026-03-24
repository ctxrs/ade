use super::*;
use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::sync::Mutex as StdMutex;

use fs2::FileExt;

const MANAGED_ARTIFACT_RETRY_COUNT: u32 = 4;
const MANAGED_ARTIFACT_DISK_HEADROOM_BYTES: u64 = 64 * 1024 * 1024;

fn managed_artifact_connect_timeout() -> Duration {
    Duration::from_secs(20)
}

fn managed_artifact_no_progress_timeout() -> Duration {
    if cfg!(test) {
        Duration::from_millis(250)
    } else {
        Duration::from_secs(30)
    }
}

fn managed_artifact_retry_backoff(attempt: u32) -> Duration {
    if cfg!(test) {
        Duration::from_millis(20 * attempt as u64)
    } else {
        Duration::from_secs(attempt as u64)
    }
}

pub(crate) fn managed_artifact_partial_path(final_path: &Path) -> PathBuf {
    let file_name = final_path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("managed-artifact");
    final_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!(".{file_name}.partial"))
}

pub(crate) fn managed_artifact_lock_path(final_path: &Path) -> PathBuf {
    let file_name = final_path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("managed-artifact");
    final_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!(".{file_name}.lock"))
}

pub(crate) struct ManagedArtifactFileLockGuard {
    _file: std::fs::File,
}

pub(crate) async fn acquire_managed_artifact_file_lock(
    lock_path: &Path,
    artifact_label: &str,
    observer: Option<&dyn HarnessSetupObserver>,
    phase: HarnessSetupPhase,
) -> Result<ManagedArtifactFileLockGuard> {
    let Some(parent) = lock_path.parent() else {
        anyhow::bail!(
            "managed artifact lock path missing parent: {}",
            lock_path.display()
        );
    };
    fs::create_dir_all(parent)
        .await
        .with_context(|| format!("creating {}", parent.display()))?;

    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(lock_path)
        .with_context(|| format!("opening managed artifact lock {}", lock_path.display()))?;

    match file.try_lock_exclusive() {
        Ok(()) => Ok(ManagedArtifactFileLockGuard { _file: file }),
        Err(err) if err.kind() == ErrorKind::WouldBlock => {
            observe_log(
                observer,
                phase,
                HarnessSetupLogLevel::Info,
                &format!("waiting for another ctx process to finish {artifact_label} preparation"),
            );
            let lock_path = lock_path.to_path_buf();
            let file = tokio::task::spawn_blocking(move || -> Result<std::fs::File> {
                file.lock_exclusive().with_context(|| {
                    format!("locking managed artifact lock {}", lock_path.display())
                })?;
                Ok(file)
            })
            .await
            .context("joining managed artifact lock task")??;
            Ok(ManagedArtifactFileLockGuard { _file: file })
        }
        Err(err) => Err(err)
            .with_context(|| format!("locking managed artifact lock {}", lock_path.display())),
    }
}

async fn ensure_managed_artifact_free_space(
    parent: &Path,
    artifact_label: &str,
    bytes_to_write: u64,
) -> Result<()> {
    let required_bytes = bytes_to_write.saturating_add(MANAGED_ARTIFACT_DISK_HEADROOM_BYTES);
    let parent = parent.to_path_buf();
    let parent_for_check = parent.clone();
    let available_bytes =
        tokio::task::spawn_blocking(move || fs2::available_space(&parent_for_check))
            .await
            .context("joining managed artifact free-space check")?
            .with_context(|| format!("checking free space for {}", parent.display()))?;
    if available_bytes < required_bytes {
        anyhow::bail!(
            "insufficient disk space for {artifact_label} download in {}: need {} free, found {}",
            parent.display(),
            format_byte_count(required_bytes),
            format_byte_count(available_bytes)
        );
    }
    Ok(())
}

async fn verify_managed_artifact_checksum(path: &Path, expected_sha256: &str) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let digest = updates::sha256_hex_file(path)
        .await
        .with_context(|| format!("computing sha256 for {}", path.display()))?;
    Ok(digest.eq_ignore_ascii_case(expected_sha256.trim()))
}

pub(crate) async fn finalize_managed_artifact_download(
    tmp_path: &Path,
    final_path: &Path,
    expected_sha256: &str,
    artifact_label: &str,
) -> Result<()> {
    let digest = updates::sha256_hex_file(tmp_path)
        .await
        .with_context(|| format!("computing sha256 for {}", tmp_path.display()))?;
    if !digest.eq_ignore_ascii_case(expected_sha256.trim()) {
        let _ = fs::remove_file(tmp_path).await;
        anyhow::bail!(
            "{artifact_label} checksum mismatch: expected {}, got {}",
            expected_sha256.trim(),
            digest
        );
    }

    match fs::rename(tmp_path, final_path).await {
        Ok(()) => Ok(()),
        Err(rename_err) => {
            if verify_managed_artifact_checksum(final_path, expected_sha256).await? {
                let _ = fs::remove_file(tmp_path).await;
                return Ok(());
            }
            Err(rename_err).with_context(|| {
                format!(
                    "moving {artifact_label} into place: {} -> {}",
                    tmp_path.display(),
                    final_path.display()
                )
            })
        }
    }
}

fn format_byte_count(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let value = bytes as f64;
    if value >= GB {
        format!("{:.1} GB", value / GB)
    } else if value >= MB {
        format!("{:.1} MB", value / MB)
    } else if value >= KB {
        format!("{:.1} KB", value / KB)
    } else {
        format!("{bytes} B")
    }
}

fn format_duration_compact(duration: Duration) -> String {
    if duration.as_millis() < 1000 {
        format!("{}ms", duration.as_millis())
    } else {
        format!("{}s", duration.as_secs())
    }
}

fn managed_artifact_error_is_retryable(err: &anyhow::Error) -> bool {
    let rendered = format!("{err:#}");
    !rendered.contains("insufficient disk space")
        && !rendered.contains("managed artifact server did not provide content length")
        && !rendered.contains("managed artifact total size is unavailable")
}

#[derive(Debug, Default)]
struct ManagedDownloadAggregateState {
    downloads: BTreeMap<String, ManagedDownloadArtifactState>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ManagedDownloadAggregate {
    inner: Arc<StdMutex<ManagedDownloadAggregateState>>,
}

#[derive(Debug, Clone, Default)]
struct ManagedDownloadArtifactState {
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
    bytes_per_sec: Option<u64>,
    finished: bool,
}

impl ManagedDownloadAggregate {
    fn update(
        &self,
        artifact: &str,
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
        bytes_per_sec: Option<u64>,
        finished: bool,
    ) -> Option<HarnessSetupDownloadStatus> {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let entry = inner.downloads.entry(artifact.to_string()).or_default();
        entry.downloaded_bytes = downloaded_bytes;
        entry.total_bytes = total_bytes;
        entry.bytes_per_sec = bytes_per_sec;
        entry.finished = finished;

        let all_finished = inner.downloads.values().all(|download| download.finished);
        if all_finished {
            return None;
        }

        let downloaded_total = inner
            .downloads
            .values()
            .map(|download| download.downloaded_bytes)
            .sum();
        let total_bytes = inner.downloads.values().try_fold(0u64, |acc, download| {
            download.total_bytes.map(|value| acc.saturating_add(value))
        });
        let bytes_per_sec = inner
            .downloads
            .values()
            .filter_map(|download| download.bytes_per_sec)
            .fold(None, |acc: Option<u64>, value| {
                Some(acc.unwrap_or(0u64).saturating_add(value))
            });
        Some(HarnessSetupDownloadStatus {
            artifact: "Required artifacts".to_string(),
            downloaded_bytes: downloaded_total,
            total_bytes,
            bytes_per_sec,
        })
    }
}

#[derive(Clone)]
pub(crate) struct ManagedArtifactDownloadReporter<'a> {
    observer: Option<&'a dyn HarnessSetupObserver>,
    aggregate: Option<ManagedDownloadAggregate>,
    phase: HarnessSetupPhase,
    artifact: String,
}

impl<'a> ManagedArtifactDownloadReporter<'a> {
    pub(crate) fn new(
        observer: Option<&'a dyn HarnessSetupObserver>,
        aggregate: Option<ManagedDownloadAggregate>,
        phase: HarnessSetupPhase,
        artifact: impl Into<String>,
    ) -> Self {
        Self {
            observer,
            aggregate,
            phase,
            artifact: artifact.into(),
        }
    }

    fn emit_progress(
        &self,
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
        bytes_per_sec: Option<u64>,
        finished: bool,
    ) {
        let active_download = if let Some(aggregate) = &self.aggregate {
            aggregate.update(
                &self.artifact,
                downloaded_bytes,
                total_bytes,
                bytes_per_sec,
                finished,
            )
        } else if finished {
            None
        } else {
            Some(HarnessSetupDownloadStatus {
                artifact: self.artifact.clone(),
                downloaded_bytes,
                total_bytes,
                bytes_per_sec,
            })
        };
        observe_progress(
            self.observer,
            HarnessSetupProgressUpdate {
                phase: self.phase,
                active_download,
            },
        );
    }
}

pub(crate) async fn download_managed_artifact(
    url: &str,
    dest: &Path,
    reporter: Option<ManagedArtifactDownloadReporter<'_>>,
) -> Result<()> {
    let Some(parent) = dest.parent() else {
        anyhow::bail!("download destination missing parent: {}", dest.display());
    };
    fs::create_dir_all(parent)
        .await
        .with_context(|| format!("creating {}", parent.display()))?;
    for attempt in 1..=MANAGED_ARTIFACT_RETRY_COUNT {
        let attempt_res: Result<()> = async {
            let client = reqwest::Client::builder()
                .connect_timeout(managed_artifact_connect_timeout())
                .build()
                .context("building reqwest client for managed artifact download")?;
            let existing_len = fs::metadata(dest).await.map(|meta| meta.len()).unwrap_or(0);
            let mut request = client.get(url);
            if existing_len > 0 {
                request = request.header(reqwest::header::RANGE, format!("bytes={existing_len}-"));
            }
            let response = request
                .send()
                .await
                .with_context(|| format!("downloading managed artifact: {url}"))?;
            let status = response.status();
            if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
                let _ = fs::remove_file(dest).await;
                anyhow::bail!("server rejected ranged resume request");
            }
            let response = response
                .error_for_status()
                .with_context(|| format!("managed artifact download http error: {url}"))?;
            let content_length = response.content_length().ok_or_else(|| {
                anyhow::anyhow!("managed artifact server did not provide content length: {url}")
            })?;
            let (resumed, total_opt) =
                crate::installer::resolve_download_resume(existing_len, status, Some(content_length));
            let total_bytes = total_opt.ok_or_else(|| {
                anyhow::anyhow!("managed artifact total size is unavailable: {url}")
            })?;

            let download_start_bytes = if resumed { existing_len } else { 0 };
            if let Some(reporter) = reporter.as_ref() {
                observe_phase(
                    reporter.observer,
                    reporter.phase,
                    "downloading required artifacts",
                );
                if attempt > 1 {
                    observe_log(
                        reporter.observer,
                        reporter.phase,
                        HarnessSetupLogLevel::Info,
                        &format!(
                            "retrying {} download (attempt {attempt}/{MANAGED_ARTIFACT_RETRY_COUNT})",
                            reporter.artifact
                        ),
                    );
                }
                let size_suffix = format!(" ({})", format_byte_count(total_bytes));
                if resumed {
                    observe_log(
                        reporter.observer,
                        reporter.phase,
                        HarnessSetupLogLevel::Info,
                        &format!(
                            "resuming {} download from {}{}",
                            reporter.artifact,
                            format_byte_count(existing_len),
                            size_suffix
                        ),
                    );
                } else {
                    observe_log(
                        reporter.observer,
                        reporter.phase,
                        HarnessSetupLogLevel::Info,
                        &format!("starting {} download{}", reporter.artifact, size_suffix),
                    );
                    if existing_len > 0 {
                        observe_log(
                            reporter.observer,
                            reporter.phase,
                            HarnessSetupLogLevel::Warn,
                            &format!(
                                "{} server does not support resume; restarting from byte 0",
                                reporter.artifact
                            ),
                        );
                    }
                }
                reporter.emit_progress(download_start_bytes, Some(total_bytes), None, false);
            }

            let bytes_to_write = if resumed {
                total_bytes.saturating_sub(existing_len)
            } else {
                total_bytes
            };
            ensure_managed_artifact_free_space(
                parent,
                reporter
                    .as_ref()
                    .map(|value| value.artifact.as_str())
                    .unwrap_or("managed artifact"),
                bytes_to_write,
            )
            .await?;

            let mut stream = response.bytes_stream();
            let mut file = if resumed {
                fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(dest)
                    .await
                    .with_context(|| format!("opening {} for append", dest.display()))?
            } else {
                fs::File::create(dest)
                    .await
                    .with_context(|| format!("creating {}", dest.display()))?
            };
            let started = tokio::time::Instant::now();
            let mut downloaded_bytes = download_start_bytes;
            let mut attempt_downloaded_bytes = 0u64;
            let mut next_progress_pct = if total_bytes == 0 {
                100
            } else {
                (((downloaded_bytes as f64 / total_bytes as f64) * 100.0).floor() as u64 / 10 + 1)
                    * 10
            };
            let mut last_progress_snapshot = tokio::time::Instant::now();
            let mut last_progress_log = tokio::time::Instant::now();
            loop {
                let next_chunk =
                    tokio::time::timeout(managed_artifact_no_progress_timeout(), stream.next())
                        .await;
                let chunk = match next_chunk {
                    Ok(Some(chunk)) => chunk,
                    Ok(None) => break,
                    Err(_) => {
                        anyhow::bail!(
                            "no download progress from {url} for {}",
                            format_duration_compact(managed_artifact_no_progress_timeout())
                        );
                    }
                };
                let chunk = chunk.with_context(|| format!("reading download stream from {url}"))?;
                file.write_all(&chunk)
                    .await
                    .with_context(|| format!("writing {}", dest.display()))?;
                downloaded_bytes = downloaded_bytes.saturating_add(chunk.len() as u64);
                attempt_downloaded_bytes =
                    attempt_downloaded_bytes.saturating_add(chunk.len() as u64);

                let elapsed = started.elapsed();
                let bytes_per_sec = if elapsed.as_secs_f64() > 0.0 {
                    Some((attempt_downloaded_bytes as f64 / elapsed.as_secs_f64()).round() as u64)
                } else {
                    None
                };

                if let Some(reporter) = reporter.as_ref() {
                    let now = tokio::time::Instant::now();
                    let should_emit_snapshot = now.duration_since(last_progress_snapshot)
                        >= Duration::from_secs(1)
                        || downloaded_bytes >= total_bytes;
                    if should_emit_snapshot {
                        reporter.emit_progress(
                            downloaded_bytes,
                            Some(total_bytes),
                            bytes_per_sec,
                            false,
                        );
                        last_progress_snapshot = now;
                    }

                    let pct = if total_bytes == 0 {
                        100
                    } else {
                        ((downloaded_bytes as f64 / total_bytes as f64) * 100.0).floor() as u64
                    };
                    let should_emit_log = if pct >= next_progress_pct {
                        next_progress_pct = ((pct / 10) + 1) * 10;
                        true
                    } else {
                        now.duration_since(last_progress_log) >= Duration::from_secs(15)
                    };

                    if should_emit_log {
                        observe_log(
                            reporter.observer,
                            reporter.phase,
                            HarnessSetupLogLevel::Info,
                            &format!(
                                "{} download {}% ({} / {})",
                                reporter.artifact,
                                pct.min(100),
                                format_byte_count(downloaded_bytes),
                                format_byte_count(total_bytes),
                            ),
                        );
                        last_progress_log = now;
                    }
                }
            }
            file.flush()
                .await
                .with_context(|| format!("flushing {}", dest.display()))?;
            if let Some(reporter) = reporter.as_ref() {
                let elapsed = started.elapsed();
                let bytes_per_sec = if elapsed.as_secs_f64() > 0.0 {
                    Some((attempt_downloaded_bytes as f64 / elapsed.as_secs_f64()).round() as u64)
                } else {
                    None
                };
                reporter.emit_progress(downloaded_bytes, Some(total_bytes), bytes_per_sec, true);
                observe_log(
                    reporter.observer,
                    reporter.phase,
                    HarnessSetupLogLevel::Info,
                    &format!(
                        "{} download complete ({})",
                        reporter.artifact,
                        format_byte_count(total_bytes)
                    ),
                );
            }
            Ok(())
        }
        .await;

        match attempt_res {
            Ok(()) => return Ok(()),
            Err(err)
                if attempt < MANAGED_ARTIFACT_RETRY_COUNT
                    && managed_artifact_error_is_retryable(&err) =>
            {
                if let Some(reporter) = reporter.as_ref() {
                    observe_log(
                        reporter.observer,
                        reporter.phase,
                        HarnessSetupLogLevel::Warn,
                        &format!(
                            "{} download attempt {attempt}/{MANAGED_ARTIFACT_RETRY_COUNT} failed: {err:#}",
                            reporter.artifact
                        ),
                    );
                }
                tokio::time::sleep(managed_artifact_retry_backoff(attempt)).await;
            }
            Err(err) => return Err(err),
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn managed_download_aggregate_combines_parallel_artifact_progress() {
        let aggregate = ManagedDownloadAggregate::default();

        let first = aggregate
            .update("Podman runtime", 10, Some(40), Some(3), false)
            .expect("first aggregate snapshot");
        assert_eq!(first.artifact, "Required artifacts");
        assert_eq!(first.downloaded_bytes, 10);
        assert_eq!(first.total_bytes, Some(40));
        assert_eq!(first.bytes_per_sec, Some(3));

        let combined = aggregate
            .update("Harness image", 5, Some(20), Some(2), false)
            .expect("combined aggregate snapshot");
        assert_eq!(combined.downloaded_bytes, 15);
        assert_eq!(combined.total_bytes, Some(60));
        assert_eq!(combined.bytes_per_sec, Some(5));

        let still_running = aggregate
            .update("Podman runtime", 40, Some(40), Some(4), true)
            .expect("remaining artifact should keep aggregate active");
        assert_eq!(still_running.downloaded_bytes, 45);
        assert_eq!(still_running.total_bytes, Some(60));
        assert_eq!(still_running.bytes_per_sec, Some(6));

        let finished = aggregate.update("Harness image", 20, Some(20), Some(2), true);
        assert!(
            finished.is_none(),
            "aggregate should clear once all downloads finish"
        );
    }

    #[tokio::test]
    async fn finalize_managed_artifact_download_tolerates_parallel_committers() {
        let temp = tempfile::tempdir().expect("tempdir");
        let final_path = temp.path().join("podman-machine");
        let first_tmp = temp.path().join("podman-machine.download-first");
        let second_tmp = temp.path().join("podman-machine.download-second");
        let payload = b"shared-machine-cache";

        fs::write(&first_tmp, payload)
            .await
            .expect("write first tmp payload");
        fs::write(&second_tmp, payload)
            .await
            .expect("write second tmp payload");
        let expected_sha256 = updates::sha256_hex_file(&first_tmp)
            .await
            .expect("compute tmp checksum");

        let (first, second) = tokio::join!(
            finalize_managed_artifact_download(
                &first_tmp,
                &final_path,
                &expected_sha256,
                "managed podman machine cache"
            ),
            finalize_managed_artifact_download(
                &second_tmp,
                &final_path,
                &expected_sha256,
                "managed podman machine cache"
            ),
        );

        first.expect("first finalization should succeed");
        second.expect("second finalization should succeed");
        assert!(
            verify_managed_artifact_checksum(&final_path, &expected_sha256)
                .await
                .expect("verify final checksum"),
            "final cache artifact should exist with the expected checksum"
        );
        assert!(
            !first_tmp.exists(),
            "first tmp path should be consumed during finalization"
        );
        assert!(
            !second_tmp.exists(),
            "second tmp path should be cleaned up during finalization"
        );
    }

    async fn read_http_request(socket: &mut tokio::net::TcpStream) -> String {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 1024];
        loop {
            let read = socket.read(&mut chunk).await.expect("read request");
            if read == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..read]);
            if buf.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        String::from_utf8_lossy(&buf).to_string()
    }

    fn parse_range_start(request: &str) -> Option<u64> {
        request.lines().find_map(|line| {
            let lower = line.to_ascii_lowercase();
            let value = lower.strip_prefix("range: bytes=")?;
            let (start, _) = value.split_once('-')?;
            start.trim().parse::<u64>().ok()
        })
    }

    async fn write_http_response(
        socket: &mut tokio::net::TcpStream,
        status_line: &str,
        extra_headers: &[String],
        body: &[u8],
    ) {
        let mut response = format!(
            "HTTP/1.1 {status_line}\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n",
            body.len()
        );
        for header in extra_headers {
            response.push_str(header);
            response.push_str("\r\n");
        }
        response.push_str("\r\n");
        socket
            .write_all(response.as_bytes())
            .await
            .expect("write response headers");
        if !body.is_empty() {
            socket.write_all(body).await.expect("write response body");
        }
        let _ = socket.shutdown().await;
    }

    #[tokio::test]
    async fn download_managed_artifact_resumes_existing_partial_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        let dest = temp.path().join("artifact.partial");
        let payload = b"managed-artifact-payload".to_vec();
        let existing_len = 7usize;
        fs::write(&dest, &payload[..existing_len])
            .await
            .expect("seed partial file");

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("listener addr");
        let requests = Arc::new(AtomicUsize::new(0));
        let requests_for_server = Arc::clone(&requests);
        let payload_for_server = payload.clone();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept connection");
            requests_for_server.fetch_add(1, Ordering::SeqCst);
            let request = read_http_request(&mut socket).await;
            assert_eq!(
                parse_range_start(&request),
                Some(existing_len as u64),
                "resume request should start from the on-disk partial length"
            );
            let remaining = &payload_for_server[existing_len..];
            write_http_response(
                &mut socket,
                "206 Partial Content",
                &[
                    "Accept-Ranges: bytes".to_string(),
                    format!(
                        "Content-Range: bytes {}-{}/{}",
                        existing_len,
                        payload_for_server.len() - 1,
                        payload_for_server.len()
                    ),
                ],
                remaining,
            )
            .await;
        });

        download_managed_artifact(&format!("http://{addr}/artifact"), &dest, None)
            .await
            .expect("resume download should succeed");
        server.await.expect("server task");
        assert_eq!(
            fs::read(&dest).await.expect("read final payload"),
            payload,
            "download should append the remaining bytes to the stable partial file"
        );
        assert_eq!(requests.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn download_managed_artifact_retries_with_range_resume_after_truncated_response() {
        let temp = tempfile::tempdir().expect("tempdir");
        let dest = temp.path().join("artifact.partial");
        let payload = b"managed-artifact-retry-payload".to_vec();
        let first_chunk_len = 9usize;

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("listener addr");
        let requests = Arc::new(AtomicUsize::new(0));
        let requests_for_server = Arc::clone(&requests);
        let payload_for_server = payload.clone();
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut socket, _) = listener.accept().await.expect("accept connection");
                let request_index = requests_for_server.fetch_add(1, Ordering::SeqCst);
                let request = read_http_request(&mut socket).await;
                if request_index == 0 {
                    assert!(
                        parse_range_start(&request).is_none(),
                        "first request should start from byte 0"
                    );
                    let headers = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        payload_for_server.len()
                    );
                    socket
                        .write_all(headers.as_bytes())
                        .await
                        .expect("write first response headers");
                    socket
                        .write_all(&payload_for_server[..first_chunk_len])
                        .await
                        .expect("write first truncated chunk");
                    let _ = socket.shutdown().await;
                } else {
                    assert_eq!(
                        parse_range_start(&request),
                        Some(first_chunk_len as u64),
                        "retry should resume from the previously written partial bytes"
                    );
                    let remaining = &payload_for_server[first_chunk_len..];
                    write_http_response(
                        &mut socket,
                        "206 Partial Content",
                        &[
                            "Accept-Ranges: bytes".to_string(),
                            format!(
                                "Content-Range: bytes {}-{}/{}",
                                first_chunk_len,
                                payload_for_server.len() - 1,
                                payload_for_server.len()
                            ),
                        ],
                        remaining,
                    )
                    .await;
                }
            }
        });

        download_managed_artifact(&format!("http://{addr}/artifact"), &dest, None)
            .await
            .expect("retrying download should succeed");
        server.await.expect("server task");
        assert_eq!(
            fs::read(&dest).await.expect("read completed payload"),
            payload,
            "retry path should preserve the first partial bytes and resume with HTTP Range"
        );
        assert_eq!(requests.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn download_managed_artifact_fails_disk_preflight_before_writing() {
        let temp = tempfile::tempdir().expect("tempdir");
        let dest = temp.path().join("artifact.partial");
        let available = fs2::available_space(temp.path()).expect("available space");

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("listener addr");
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept connection");
            let _request = read_http_request(&mut socket).await;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                available
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("write oversized response");
            let _ = socket.shutdown().await;
        });

        let err = download_managed_artifact(&format!("http://{addr}/artifact"), &dest, None)
            .await
            .expect_err("disk-space preflight should fail before any writes");
        server.await.expect("server task");
        assert!(
            format!("{err:#}").contains("insufficient disk space"),
            "expected a clear disk-space error, got: {err:#}"
        );
        assert!(
            !dest.exists(),
            "preflight failure should happen before the download target is created"
        );
    }

    #[tokio::test]
    async fn managed_artifact_file_lock_waits_for_existing_holder() {
        let temp = tempfile::tempdir().expect("tempdir");
        let lock_path = temp.path().join("artifact.lock");
        let first = acquire_managed_artifact_file_lock(
            &lock_path,
            "test artifact",
            None,
            HarnessSetupPhase::ArtifactDownload,
        )
        .await
        .expect("acquire first lock");

        let acquired = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let acquired_for_waiter = Arc::clone(&acquired);
        let lock_path_for_waiter = lock_path.clone();
        let waiter = tokio::spawn(async move {
            let _second = acquire_managed_artifact_file_lock(
                &lock_path_for_waiter,
                "test artifact",
                None,
                HarnessSetupPhase::ArtifactDownload,
            )
            .await
            .expect("acquire second lock");
            acquired_for_waiter.store(true, Ordering::SeqCst);
        });

        tokio::time::sleep(Duration::from_millis(40)).await;
        assert!(
            !acquired.load(Ordering::SeqCst),
            "second file lock should wait while the first holder is alive"
        );

        drop(first);
        tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("second lock should eventually acquire")
            .expect("waiter should finish cleanly");
        assert!(acquired.load(Ordering::SeqCst));
    }
}
