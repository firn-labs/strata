//! API error type with HTTP mappings.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use jiff::Timestamp;
use serde_json::json;
use strata_common::{
    Confidentiality, DocumentAction, DocumentId, DocumentStatus, DossierAction, DossierEntryId,
    DossierId,
};
use strata_storage::StorageError;

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

    #[error(
        "a deletion deadline can only be set at or after archive time; document is still {status}"
    )]
    RetentionBeforeArchive { status: DocumentStatus },

    #[error("deletion of document {document} is blocked until {until} (PRESERVE-06)")]
    DeletionBlocked {
        document: DocumentId,
        until: Timestamp,
    },

    #[error("document {0} has no stored content")]
    NoContent(DocumentId),

    #[error("no attached storage backend may hold {tier} documents (STORE-04)")]
    NoPlacementBackend { tier: Confidentiality },

    /// A document's blob lives on a backend that is no longer attached —
    /// an operator configuration error, surfaced rather than masked.
    #[error("storage backend {0} is not attached but still holds document blobs")]
    BackendDetached(String),

    /// Stored bytes could not be decrypted — wrong operator key or a
    /// tampered/corrupted blob.
    #[error("stored content of document {0} could not be decrypted")]
    UnreadableBlob(DocumentId),

    #[error("storage backend failure: {0}")]
    Storage(#[from] StorageError),

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
            ApiError::RetentionBeforeArchive { .. } => StatusCode::CONFLICT,
            ApiError::DeletionBlocked { .. } => StatusCode::CONFLICT,
            ApiError::NoContent(_) => StatusCode::NOT_FOUND,
            ApiError::NoPlacementBackend { .. } => StatusCode::CONFLICT,
            ApiError::BackendDetached(_) => StatusCode::INTERNAL_SERVER_ERROR,
            ApiError::UnreadableBlob(_) => StatusCode::INTERNAL_SERVER_ERROR,
            ApiError::Storage(_) => StatusCode::INTERNAL_SERVER_ERROR,
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
