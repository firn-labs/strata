//! Strata core API server.
//!
//! Owns the storage abstraction and the system-of-record concerns:
//! documents, metadata, versioning, permissions, audit log, retention,
//! and search indexing. The workflow engine and the frontend talk to
//! this service over its HTTP API — nothing else touches storage.

use axum::{Json, Router, routing::get};
use strata_common::{Health, HealthStatus};

const SERVICE: &str = "strata-server";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let app = Router::new().route("/healthz", get(healthz));

    let addr = std::env::var("STRATA_SERVER_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".into());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(%addr, "{SERVICE} listening");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn healthz() -> Json<Health> {
    Json(Health {
        service: SERVICE,
        version: env!("CARGO_PKG_VERSION"),
        status: HealthStatus::Ok,
    })
}
