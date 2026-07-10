//! Pluggable storage layer for Strata.
//!
//! The core API server never touches storage media directly — it talks to a
//! [`StorageProvider`]. Each supported medium (local filesystem, S3, SMB,
//! NFS, ...) is one implementation of that trait, selected and configured at
//! runtime. This is what makes the "Speicherverwaltung" layer swappable.
//!
//! Currently implemented providers:
//! - [`LocalFsProvider`] — stores blobs in a directory on the local
//!   filesystem (also covers NFS/SMB media mounted by the host).
//! - [`MemoryProvider`] — keeps blobs in process memory, for tests and
//!   local development.
//!
//! Planned providers: S3-compatible object storage, native SMB.

mod local_fs;
mod memory;

pub use local_fs::LocalFsProvider;
pub use memory::MemoryProvider;

use strata_common::DocumentId;

/// Errors a storage provider can produce.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("blob for document {0} was not found")]
    NotFound(DocumentId),
    #[error("storage backend I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("storage backend error: {0}")]
    Backend(String),
}

pub type Result<T> = std::result::Result<T, StorageError>;

/// A storage backend that can persist and retrieve document blobs.
///
/// Implementations must be safe to share across the async runtime.
/// Versioning, metadata, and permissions are *not* handled here — they are
/// the responsibility of the core server; providers only store bytes.
#[async_trait::async_trait]
pub trait StorageProvider: Send + Sync {
    /// Human-readable name of the backend (used in logs and health output).
    fn name(&self) -> &'static str;

    /// Persist the blob for a document, overwriting any existing blob.
    async fn put(&self, id: DocumentId, bytes: &[u8]) -> Result<()>;

    /// Retrieve the blob for a document.
    async fn get(&self, id: DocumentId) -> Result<Vec<u8>>;

    /// Delete the blob for a document. Deleting a missing blob is an error
    /// so that retention bookkeeping (Löschnachweis) stays truthful.
    async fn delete(&self, id: DocumentId) -> Result<()>;

    /// Check whether a blob exists for a document.
    async fn exists(&self, id: DocumentId) -> Result<bool>;
}
