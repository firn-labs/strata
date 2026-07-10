//! Strata workflow engine.
//!
//! The middle layer of Strata: departments build flows in the visual editor
//! (frontend), the editor saves them here as JSON graph definitions, and
//! this service executes them — calling the core server's API for every
//! storage or metadata operation and logging each execution step.

mod flow;

use axum::{Json, Router, routing::get};
use strata_common::{Health, HealthStatus};

const SERVICE: &str = "strata-workflow";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let app = Router::new().route("/healthz", get(healthz));

    let addr = std::env::var("STRATA_WORKFLOW_ADDR").unwrap_or_else(|_| "0.0.0.0:8081".into());
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
