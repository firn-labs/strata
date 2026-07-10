//! Strata workflow engine.
//!
//! The middle layer of Strata: departments build flows in the visual editor
//! (frontend), the editor saves them here as JSON graph definitions, and
//! this service executes them — calling the core server's API for every
//! storage or metadata operation and logging each execution step.
//!
//! All routes and behavior live in the library (`lib.rs`); this binary
//! only binds the listener.

use std::sync::Arc;

use strata_workflow::{AppState, SERVICE, app};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let app = app(Arc::new(AppState::new()));

    let addr = std::env::var("STRATA_WORKFLOW_ADDR").unwrap_or_else(|_| "0.0.0.0:8081".into());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(%addr, "{SERVICE} listening");
    axum::serve(listener, app).await?;
    Ok(())
}
