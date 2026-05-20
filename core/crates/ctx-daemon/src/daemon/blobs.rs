pub use ctx_session_artifacts::{
    BlobReadError, ImageBlobStoreError, StoredImageBlob, SESSION_IMAGE_BLOB_MAX_BYTES,
};
use tokio::fs::File;

use crate::daemon::{CoreHandle, DaemonState};

pub struct OpenedBlob {
    pub file: File,
    pub mime_type: String,
    pub name: Option<String>,
}

pub(in crate::daemon) async fn store_image_blob_for_state(
    state: &DaemonState,
    bytes: &[u8],
    mime_type: &str,
    name: Option<&str>,
) -> Result<StoredImageBlob, ImageBlobStoreError> {
    ctx_session_artifacts::store_image_blob(
        &state.core.data_root,
        state.global_store(),
        bytes,
        mime_type,
        name,
    )
    .await
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
        let resolved =
            ctx_session_artifacts::resolve_blob_for_read(self.data_root(), self.global_store(), id)
                .await?;
        let file = File::open(&resolved.path)
            .await
            .map_err(|_| BlobReadError::NotFound)?;
        Ok(OpenedBlob {
            file,
            mime_type: resolved.mime_type,
            name: resolved.name,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::io::ErrorKind;

    use super::*;
    use crate::test_support::TestDaemon;
    use sha2::Digest;
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

        let path = ctx_session_artifacts::blobs_dir(core.data_root()).join(&stored.blob_id);
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
        let path = ctx_session_artifacts::blobs_dir(core.data_root()).join(&stored.blob_id);
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
