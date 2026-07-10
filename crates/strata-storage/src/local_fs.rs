//! Local-filesystem storage provider.

use std::path::PathBuf;

use strata_common::DocumentId;

use crate::{Result, StorageError, StorageProvider};

/// Stores document blobs as files under a root directory.
///
/// Blobs are sharded into subdirectories by the first two characters of the
/// document ID so a single directory never accumulates millions of entries.
pub struct LocalFsProvider {
    root: PathBuf,
}

impl LocalFsProvider {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn blob_path(&self, id: DocumentId) -> PathBuf {
        let name = id.to_string();
        self.root.join(&name[..2]).join(name)
    }
}

#[async_trait::async_trait]
impl StorageProvider for LocalFsProvider {
    fn name(&self) -> &'static str {
        "local-fs"
    }

    async fn put(&self, id: DocumentId, bytes: &[u8]) -> Result<()> {
        let path = self.blob_path(id);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&path, bytes).await?;
        tracing::debug!(%id, path = %path.display(), "stored blob");
        Ok(())
    }

    async fn get(&self, id: DocumentId) -> Result<Vec<u8>> {
        match tokio::fs::read(self.blob_path(id)).await {
            Ok(bytes) => Ok(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(StorageError::NotFound(id)),
            Err(e) => Err(e.into()),
        }
    }

    async fn delete(&self, id: DocumentId) -> Result<()> {
        match tokio::fs::remove_file(self.blob_path(id)).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(StorageError::NotFound(id)),
            Err(e) => Err(e.into()),
        }
    }

    async fn exists(&self, id: DocumentId) -> Result<bool> {
        Ok(tokio::fs::try_exists(self.blob_path(id)).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn roundtrip_put_get_delete() {
        let dir = tempfile::tempdir().unwrap();
        let store = LocalFsProvider::new(dir.path());
        let id = DocumentId::new();

        store.put(id, b"hello strata").await.unwrap();
        assert!(store.exists(id).await.unwrap());
        assert_eq!(store.get(id).await.unwrap(), b"hello strata");

        store.delete(id).await.unwrap();
        assert!(!store.exists(id).await.unwrap());
        assert!(matches!(
            store.get(id).await,
            Err(StorageError::NotFound(_))
        ));
    }
}
