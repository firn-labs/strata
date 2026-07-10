//! Strata core API server.
//!
//! Owns the storage abstraction and the system-of-record concerns:
//! documents, metadata, versioning, permissions, audit log, retention,
//! and search indexing. The workflow engine and the frontend talk to
//! this service over its HTTP API — nothing else touches storage.
//!
//! All routes and behavior live in the library (`lib.rs`); this binary
//! only binds the listener.

use std::sync::Arc;

use anyhow::Context;
use strata_common::BackendLocation;
use strata_server::{AppState, OperatorKey, SERVICE, StorageBackend, app};
use strata_storage::LocalFsProvider;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    // The operator-owned encryption key (STORE-04). Without one configured,
    // a fresh key is generated: fine while document metadata is in-memory
    // anyway, but blobs encrypted with it are unreadable after a restart.
    let operator_key = match std::env::var("STRATA_OPERATOR_KEY") {
        Ok(hex) => OperatorKey::from_hex(&hex)
            .context("STRATA_OPERATOR_KEY must be 64 hex characters (32 bytes)")?,
        Err(_) => {
            tracing::warn!(
                "STRATA_OPERATOR_KEY not set; using an ephemeral key — \
                 encrypted blobs will be unreadable after a restart"
            );
            OperatorKey::generate()
        }
    };

    let storage_root =
        std::env::var("STRATA_STORAGE_ROOT").unwrap_or_else(|_| "./data/blobs".into());
    tracing::info!(root = %storage_root, "attaching local-fs storage backend");
    let backends = vec![StorageBackend {
        name: "local".into(),
        location: BackendLocation::Internal,
        provider: Arc::new(LocalFsProvider::new(storage_root)),
    }];

    let app = app(Arc::new(AppState::with_storage(backends, operator_key)));

    let addr = std::env::var("STRATA_SERVER_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".into());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(%addr, "{SERVICE} listening");
    axum::serve(listener, app).await?;
    Ok(())
}
