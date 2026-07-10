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

use strata_server::{AppState, SERVICE, app};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let app = app(Arc::new(AppState::new()));

    let addr = std::env::var("STRATA_SERVER_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".into());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(%addr, "{SERVICE} listening");
    axum::serve(listener, app).await?;
    Ok(())
}
