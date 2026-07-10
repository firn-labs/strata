//! API error type with HTTP mappings, mirroring the core server's shape so
//! clients see one error format across services.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::engine::RunId;
use crate::flow::FlowId;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("flow {0} was not found")]
    FlowNotFound(FlowId),

    #[error("run {0} was not found")]
    RunNotFound(RunId),

    #[error("flow definition is invalid: {0}")]
    InvalidFlow(String),

    #[error("node {node:?} cannot start a run: {reason}")]
    NotATrigger { node: String, reason: &'static str },

    #[error("import is invalid: {0}")]
    InvalidImport(String),

    #[error(
        "import conflicts with existing flows ({0}); \
         re-run with ?on_conflict=replace or ?on_conflict=skip"
    )]
    ImportConflict(String),

    #[error("{0}")]
    Unauthenticated(&'static str),
}

impl ApiError {
    fn status_code(&self) -> StatusCode {
        match self {
            ApiError::FlowNotFound(_) => StatusCode::NOT_FOUND,
            ApiError::RunNotFound(_) => StatusCode::NOT_FOUND,
            ApiError::InvalidFlow(_) => StatusCode::UNPROCESSABLE_ENTITY,
            ApiError::NotATrigger { .. } => StatusCode::UNPROCESSABLE_ENTITY,
            ApiError::InvalidImport(_) => StatusCode::UNPROCESSABLE_ENTITY,
            ApiError::ImportConflict(_) => StatusCode::CONFLICT,
            ApiError::Unauthenticated(_) => StatusCode::UNAUTHORIZED,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status_code(),
            Json(json!({ "error": self.to_string() })),
        )
            .into_response()
    }
}
