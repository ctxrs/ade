use chrono::Utc;
use ctx_session_tools::SESSION_IMAGE_BLOB_MAX_BYTES;
use sha2::Digest;
use tokio::fs::File;

use crate::daemon::{CoreHandle, DaemonState};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ImageBlobStoreError {
    PayloadTooLarge,
    UnsupportedMediaType,
    Internal,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum BlobReadError {
    NotFound,
    Internal,
}

#[derive(Debug, Clone)]
pub struct StoredImageBlob {
    pub blob_id: String,
    pub sha256: String,
    pub bytes: i64,
    pub mime_type: String,
    pub name: Option<String>,
}

pub struct OpenedBlob {
    pub file: File,
    pub mime_type: String,
    pub name: Option<String>,
}

fn blobs_dir(data_root: &std::path::Path) -> std::path::PathBuf {
    data_root.join("blobs")
}

pub(in crate::daemon) async fn store_image_blob_for_state(
    state: &DaemonState,
    bytes: &[u8],
    mime_type: &str,
    name: Option<&str>,
) -> Result<StoredImageBlob, ImageBlobStoreError> {
    if bytes.len() > SESSION_IMAGE_BLOB_MAX_BYTES {
        return Err(ImageBlobStoreError::PayloadTooLarge);
    }
    if !mime_type.starts_with("image/") {
        return Err(ImageBlobStoreError::UnsupportedMediaType);
    }

    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    let sha256 = hex::encode(hasher.finalize());
    let blob_id = uuid::Uuid::new_v4().to_string();

    let dir = blobs_dir(&state.core.data_root);
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|_| ImageBlobStoreError::Internal)?;
    let path = dir.join(&blob_id);
    let tmp = dir.join(format!("{blob_id}.tmp"));

    tokio::fs::write(&tmp, bytes)
        .await
        .map_err(|_| ImageBlobStoreError::Internal)?;
    tokio::fs::rename(&tmp, &path)
        .await
        .map_err(|_| ImageBlobStoreError::Internal)?;

    state
        .global_store()
        .insert_blob(
            &blob_id,
            &sha256,
            bytes.len() as i64,
            mime_type,
            name,
            Utc::now(),
        )
        .await
        .map_err(|_| ImageBlobStoreError::Internal)?;

    Ok(StoredImageBlob {
        blob_id,
        sha256,
        bytes: bytes.len() as i64,
        mime_type: mime_type.to_string(),
        name: name.map(str::to_string),
    })
}

impl CoreHandle {
    pub async fn store_image_blob(
        &self,
        bytes: &[u8],
        mime_type: &str,
        name: Option<&str>,
    ) -> Result<StoredImageBlob, ImageBlobStoreError> {
        store_image_blob_for_state(self.state.as_ref(), bytes, mime_type, name).await
    }

    pub async fn open_blob_for_read(&self, id: &str) -> Result<OpenedBlob, BlobReadError> {
        let Some((_sha256, mime_type, _bytes, name, _created_at)) = self
            .get_blob(id)
            .await
            .map_err(|_| BlobReadError::Internal)?
        else {
            return Err(BlobReadError::NotFound);
        };

        let path = blobs_dir(self.data_root()).join(id);
        let file = File::open(&path)
            .await
            .map_err(|_| BlobReadError::NotFound)?;
        Ok(OpenedBlob {
            file,
            mime_type,
            name,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::io::ErrorKind;

    use super::*;
    use crate::test_support::TestDaemon;
    use tokio::io::AsyncReadExt;

    async fn test_core() -> (tempfile::TempDir, CoreHandle) {
        let data_dir = tempfile::tempdir().expect("tempdir");
        let daemon =
            TestDaemon::new_for_test(data_dir.path().to_path_buf(), "http://127.0.0.1:0".into())
                .await
                .expect("test daemon");
        (data_dir, daemon.handle().core())
    }

    #[tokio::test]
    async fn store_image_blob_writes_bytes_and_metadata() {
        let (_data_dir, core) = test_core().await;
        let stored = core
            .store_image_blob(b"png-bytes", "image/png", Some("image.png"))
            .await
            .expect("store image blob");

        assert_eq!(
            stored.sha256,
            hex::encode(sha2::Sha256::digest(b"png-bytes"))
        );
        assert_eq!(stored.bytes, 9);
        assert_eq!(stored.mime_type, "image/png");
        assert_eq!(stored.name.as_deref(), Some("image.png"));

        let metadata = core
            .get_blob(&stored.blob_id)
            .await
            .expect("metadata lookup")
            .expect("stored metadata");
        assert_eq!(metadata.0, stored.sha256);
        assert_eq!(metadata.1, "image/png");
        assert_eq!(metadata.2, 9);
        assert_eq!(metadata.3.as_deref(), Some("image.png"));

        let path = blobs_dir(core.data_root()).join(&stored.blob_id);
        assert_eq!(
            tokio::fs::read(path).await.expect("blob bytes"),
            b"png-bytes"
        );
    }

    #[tokio::test]
    async fn store_image_blob_rejects_invalid_inputs() {
        let (_data_dir, core) = test_core().await;
        let too_large = vec![0u8; SESSION_IMAGE_BLOB_MAX_BYTES + 1];
        assert!(matches!(
            core.store_image_blob(&too_large, "image/png", None).await,
            Err(ImageBlobStoreError::PayloadTooLarge)
        ));
        assert!(matches!(
            core.store_image_blob(b"text", "text/plain", None).await,
            Err(ImageBlobStoreError::UnsupportedMediaType)
        ));
    }

    #[tokio::test]
    async fn open_blob_for_read_returns_file_and_metadata() {
        let (_data_dir, core) = test_core().await;
        let stored = core
            .store_image_blob(b"gif-bytes", "image/gif", Some("quoted\"name.gif"))
            .await
            .expect("store image blob");

        let opened = core
            .open_blob_for_read(&stored.blob_id)
            .await
            .expect("open blob");

        assert_eq!(opened.mime_type, "image/gif");
        assert_eq!(opened.name.as_deref(), Some("quoted\"name.gif"));
        let mut file = opened.file;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).await.expect("opened bytes");
        assert_eq!(bytes, b"gif-bytes");
    }

    #[tokio::test]
    async fn open_blob_for_read_classifies_missing_metadata_as_not_found() {
        let (_data_dir, core) = test_core().await;
        assert!(matches!(
            core.open_blob_for_read("missing").await,
            Err(BlobReadError::NotFound)
        ));
    }

    #[tokio::test]
    async fn open_blob_for_read_classifies_missing_backing_file_as_not_found() {
        let (_data_dir, core) = test_core().await;
        let stored = core
            .store_image_blob(b"jpeg-bytes", "image/jpeg", None)
            .await
            .expect("store image blob");
        let path = blobs_dir(core.data_root()).join(&stored.blob_id);
        match tokio::fs::remove_file(path).await {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => panic!("remove blob file: {error}"),
        }

        assert!(matches!(
            core.open_blob_for_read(&stored.blob_id).await,
            Err(BlobReadError::NotFound)
        ));
    }
}
