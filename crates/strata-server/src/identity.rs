//! Caller identity — development placeholder.
//!
//! Until SSO/OIDC integration lands, the caller identifies itself with plain
//! headers: `x-strata-user` (required) and `x-strata-groups` (optional,
//! comma-separated). This keeps every permission check real and testable
//! today; swapping this extractor for token validation later changes nothing
//! in the handlers, because they only ever see an [`Actor`].

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use strata_common::Actor;

use crate::error::ApiError;

pub const USER_HEADER: &str = "x-strata-user";
pub const GROUPS_HEADER: &str = "x-strata-groups";

/// The authenticated caller, extracted from every request that needs one.
pub struct Principal(pub Actor);

impl<S: Send + Sync> FromRequestParts<S> for Principal {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let user = parts
            .headers
            .get(USER_HEADER)
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .filter(|u| !u.is_empty())
            .ok_or(ApiError::Unauthenticated(
                "missing x-strata-user header (placeholder auth until OIDC lands)",
            ))?
            .to_owned();

        let groups = parts
            .headers
            .get(GROUPS_HEADER)
            .and_then(|v| v.to_str().ok())
            .map(|list| {
                list.split(',')
                    .map(str::trim)
                    .filter(|g| !g.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();

        Ok(Principal(Actor { user, groups }))
    }
}
