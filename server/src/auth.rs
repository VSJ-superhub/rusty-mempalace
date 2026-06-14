//! Bearer-token auth + wing-scoping middleware.
//!
//! Runs ahead of every `/api` route. It resolves the presented token into an
//! [`AccessScope`] and stashes it in the request extensions, so handlers can pull
//! it out with `Extension<AccessScope>` and never touch raw credentials.
//!
//! Exposure rules:
//! - No active tokens **and** loopback bind ⇒ anonymous request gets a synthetic
//!   local-admin scope (single-user laptop convenience).
//! - No active tokens on a non-loopback bind ⇒ rejected. (Startup also refuses to
//!   bind non-loopback without a token; this covers tokens revoked while running.)
//! - Any active token ⇒ a valid `Authorization: Bearer <secret>` is required.

use axum::{
    extract::{Request, State},
    middleware::Next,
    response::Response,
};
use yourmemory_core::access::{AccessScope, Grant, GrantLevel, GLOBAL_WING};

use crate::error::ApiError;
use crate::AppState;

const BEARER: &str = "Bearer ";

/// Synthetic scope for the no-token loopback case: full local admin.
fn local_admin_scope() -> AccessScope {
    AccessScope::new(
        0,
        "local".to_string(),
        vec![Grant { wing: GLOBAL_WING.to_string(), level: GrantLevel::Admin }],
    )
}

pub async fn require_scope(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    // Resolve the scope inside a tight block so the storage lock is never held
    // across the downstream `.await`.
    let scope = {
        let storage = state.storage.lock().expect("storage mutex poisoned");
        let active_tokens = storage.active_token_count()?;

        if active_tokens == 0 {
            if state.is_loopback {
                local_admin_scope()
            } else {
                return Err(ApiError::unauthorized(
                    "server has no access tokens; create one with `yourmemory token create`",
                ));
            }
        } else {
            let secret = bearer_secret(&req)?;
            match storage.resolve_scope(secret)? {
                Some(s) => s,
                None => {
                    // TODO(phase-2+): rate-limit + WAL-log repeated auth failures.
                    tracing::warn!("rejected request: invalid or revoked token");
                    return Err(ApiError::unauthorized("invalid or revoked token"));
                }
            }
        }
    };

    req.extensions_mut().insert(scope);
    Ok(next.run(req).await)
}

/// Extract the bearer secret from the `Authorization` header, or 401.
fn bearer_secret(req: &Request) -> Result<&str, ApiError> {
    let header = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .ok_or_else(|| ApiError::unauthorized("missing Authorization header"))?;
    let value = header
        .to_str()
        .map_err(|_| ApiError::unauthorized("malformed Authorization header"))?;
    let secret = value
        .strip_prefix(BEARER)
        .ok_or_else(|| ApiError::unauthorized("Authorization must be 'Bearer <token>'"))?
        .trim();
    if secret.is_empty() {
        return Err(ApiError::unauthorized("empty bearer token"));
    }
    Ok(secret)
}
