//! API error type with HTTP mappings.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;
use strata_common::{
    DocumentAction, DocumentId, DocumentStatus, DossierAction, DossierEntryId, DossierId,
};

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// Also returned when a document exists but the caller may not view it,
    /// so the API does not leak which IDs exist.
    #[error("document {0} was not found")]
    DocumentNotFound(DocumentId),

    #[error("action {action:?} is not permitted on a {status} document")]
    Forbidden {
        action: DocumentAction,
        status: DocumentStatus,
    },

    #[error("a {from} document cannot become {to}")]
    InvalidTransition {
        from: DocumentStatus,
        to: DocumentStatus,
    },

    /// Also returned when a dossier exists but the caller may not view it,
    /// mirroring the document rule.
    #[error("dossier {0} was not found")]
    DossierNotFound(DossierId),

    #[error("action {0:?} is not permitted on this dossier")]
    DossierForbidden(DossierAction),

    #[error("dossier entry {0} was not found")]
    EntryNotFound(DossierEntryId),

    #[error("document {document} is already filed in dossier {dossier}")]
    DocumentAlreadyFiled {
        document: DocumentId,
        dossier: DossierId,
    },

    #[error("{0}")]
    Unauthenticated(&'static str),
}

impl ApiError {
    fn status_code(&self) -> StatusCode {
        match self {
            ApiError::DocumentNotFound(_) => StatusCode::NOT_FOUND,
            ApiError::Forbidden { .. } => StatusCode::FORBIDDEN,
            ApiError::InvalidTransition { .. } => StatusCode::CONFLICT,
            ApiError::DossierNotFound(_) => StatusCode::NOT_FOUND,
            ApiError::DossierForbidden(_) => StatusCode::FORBIDDEN,
            ApiError::EntryNotFound(_) => StatusCode::NOT_FOUND,
            ApiError::DocumentAlreadyFiled { .. } => StatusCode::CONFLICT,
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
