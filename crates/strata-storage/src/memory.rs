//! In-memory storage provider.

use std::collections::HashMap;
use std::sync::Mutex;

use strata_common::DocumentId;

use crate::{Result, StorageError, StorageProvider};

/// Stores document blobs in process memory.
///
/// Nothing survives a restart — this backend exists for tests and local
/// development, where it stands in for any real medium (in particular for
/// exercising multi-backend placement, STORE-04, without external services).
#[derive(Default)]
pub struct MemoryProvider {
    blobs: Mutex<HashMap<DocumentId, Vec<u8>>>,
}

impl MemoryProvider {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl StorageProvider for MemoryProvider {
    fn name(&self) -> &'static str {
        "memory"
    }

    async fn put(&self, id: DocumentId, bytes: &[u8]) -> Result<()> {
        self.blobs
            .lock()
            .expect("blob map lock poisoned")
            .insert(id, bytes.to_vec());
        Ok(())
    }

    async fn get(&self, id: DocumentId) -> Result<Vec<u8>> {
        self.blobs
            .lock()
            .expect("blob map lock poisoned")
            .get(&id)
            .cloned()
            .ok_or(StorageError::NotFound(id))
    }

    async fn delete(&self, id: DocumentId) -> Result<()> {
        self.blobs
            .lock()
            .expect("blob map lock poisoned")
            .remove(&id)
            .map(|_| ())
            .ok_or(StorageError::NotFound(id))
    }

    async fn exists(&self, id: DocumentId) -> Result<bool> {
        Ok(self
            .blobs
            .lock()
            .expect("blob map lock poisoned")
            .contains_key(&id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn roundtrip_put_get_delete() {
        let store = MemoryProvider::new();
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
        assert!(matches!(
            store.delete(id).await,
            Err(StorageError::NotFound(_))
        ));
    }
}
