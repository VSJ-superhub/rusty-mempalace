//! Scoped REST handlers. Every handler receives the resolved [`AccessScope`] (from
//! the auth middleware) and defers all visibility decisions to `core`'s scoped
//! query functions — no handler hand-rolls a wing check.

use axum::{
    extract::{Path, Query, State},
    http::header,
    response::{IntoResponse, Response},
    Extension, Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use yourmemory_core::access::{AccessScope, Grant, GrantLevel};
use yourmemory_core::storage::{
    AuditEntry, Drawer, GraphData, PalaceStats, Room, RoomStat, SearchHit, Wing, WriteSession,
};

use crate::error::ApiError;
use crate::AppState;

/// Cap on drawer nodes returned by the graph endpoint, to bound payload size.
const GRAPH_DRAWER_CAP: usize = 500;
/// Audit page defaults/caps.
const AUDIT_DEFAULT_LIMIT: usize = 200;
const AUDIT_MAX_LIMIT: usize = 2000;

/// Pagination defaults/caps so a huge room can't return unbounded rows.
const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 500;

#[derive(Debug, Deserialize)]
pub struct PageParams {
    limit: Option<usize>,
    offset: Option<usize>,
}

impl PageParams {
    fn limit(&self) -> usize {
        self.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
    }
    fn offset(&self) -> usize {
        self.offset.unwrap_or(0)
    }
}

#[derive(Debug, Serialize)]
pub struct Page<T> {
    items: Vec<T>,
    limit: usize,
    offset: usize,
    /// Number of items in this page. `count == limit` hints the client there may be more.
    count: usize,
}

/// GET /api/wings — wings the token may read.
pub async fn list_wings(
    State(state): State<AppState>,
    Extension(scope): Extension<AccessScope>,
) -> Result<Json<Vec<Wing>>, ApiError> {
    let storage = state.storage.lock().expect("storage mutex poisoned");
    Ok(Json(storage.list_wings_scoped(&scope)?))
}

/// GET /api/wings/:wing_id/rooms — rooms in a wing, or 404 if out of scope.
pub async fn list_rooms(
    State(state): State<AppState>,
    Extension(scope): Extension<AccessScope>,
    Path(wing_id): Path<i64>,
) -> Result<Json<Vec<Room>>, ApiError> {
    let storage = state.storage.lock().expect("storage mutex poisoned");
    match storage.list_rooms_scoped(&scope, wing_id)? {
        Some(rooms) => Ok(Json(rooms)),
        None => Err(ApiError::not_found()),
    }
}

/// GET /api/rooms/:room_id/drawers?limit=&offset= — paginated, scoped to the room's wing.
pub async fn list_drawers(
    State(state): State<AppState>,
    Extension(scope): Extension<AccessScope>,
    Path(room_id): Path<i64>,
    Query(page): Query<PageParams>,
) -> Result<Json<Page<Drawer>>, ApiError> {
    let (limit, offset) = (page.limit(), page.offset());
    let storage = state.storage.lock().expect("storage mutex poisoned");
    match storage.drawers_by_room_scoped(&scope, room_id, limit, offset)? {
        Some(items) => {
            let count = items.len();
            Ok(Json(Page { items, limit, offset, count }))
        }
        None => Err(ApiError::not_found()),
    }
}

/// GET /api/drawers/:id — a single drawer, or 404 if missing or out of scope.
pub async fn get_drawer(
    State(state): State<AppState>,
    Extension(scope): Extension<AccessScope>,
    Path(id): Path<i64>,
) -> Result<Json<Drawer>, ApiError> {
    let storage = state.storage.lock().expect("storage mutex poisoned");
    match storage.get_drawer_scoped(&scope, id)? {
        Some(drawer) => Ok(Json(drawer)),
        None => Err(ApiError::not_found()),
    }
}

/// GET /api/graph — knowledge-graph nodes + edges, scoped, capped for payload size.
pub async fn graph(
    State(state): State<AppState>,
    Extension(scope): Extension<AccessScope>,
) -> Result<Json<GraphData>, ApiError> {
    let storage = state.storage.lock().expect("storage mutex poisoned");
    Ok(Json(storage.graph_scoped(&scope, GRAPH_DRAWER_CAP)?))
}

#[derive(Debug, Deserialize)]
pub struct AuditParams {
    op: Option<String>,
    from: Option<String>,
    to: Option<String>,
    wing: Option<String>,
    limit: Option<usize>,
}

impl AuditParams {
    fn limit(&self) -> usize {
        self.limit.unwrap_or(AUDIT_DEFAULT_LIMIT).clamp(1, AUDIT_MAX_LIMIT)
    }
}

/// GET /api/audit — WAL entries, scoped, with op/from/to/wing/limit filters.
pub async fn audit(
    State(state): State<AppState>,
    Extension(scope): Extension<AccessScope>,
    Query(p): Query<AuditParams>,
) -> Result<Json<Vec<AuditEntry>>, ApiError> {
    let storage = state.storage.lock().expect("storage mutex poisoned");
    let entries = storage.audit_scoped(
        &scope,
        p.op.as_deref(),
        p.from.as_deref(),
        p.to.as_deref(),
        p.wing.as_deref(),
        p.limit(),
    )?;
    Ok(Json(entries))
}

/// GET /api/audit/export — same filters as /api/audit, returned as a CSV download.
pub async fn audit_export(
    State(state): State<AppState>,
    Extension(scope): Extension<AccessScope>,
    Query(p): Query<AuditParams>,
) -> Result<Response, ApiError> {
    let entries = {
        let storage = state.storage.lock().expect("storage mutex poisoned");
        storage.audit_scoped(
            &scope,
            p.op.as_deref(),
            p.from.as_deref(),
            p.to.as_deref(),
            p.wing.as_deref(),
            p.limit(),
        )?
    };

    let mut csv = String::from("timestamp,operation,table,record_id,wing,preview\n");
    for e in &entries {
        csv.push_str(&format!(
            "{},{},{},{},{},{}\n",
            csv_field(&e.created_at),
            csv_field(&e.operation),
            csv_field(&e.table_name),
            e.record_id,
            csv_field(e.wing.as_deref().unwrap_or("")),
            csv_field(&e.preview),
        ));
    }

    Ok((
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()),
            (header::CONTENT_DISPOSITION, "attachment; filename=\"audit.csv\"".to_string()),
        ],
        csv,
    )
        .into_response())
}

/// Quote a CSV field per RFC 4180 (wrap in quotes, double any embedded quotes).
fn csv_field(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// GET /api/heatmap — per-room aggregates for readable wings.
pub async fn heatmap(
    State(state): State<AppState>,
    Extension(scope): Extension<AccessScope>,
) -> Result<Json<Vec<RoomStat>>, ApiError> {
    let storage = state.storage.lock().expect("storage mutex poisoned");
    Ok(Json(storage.heatmap_scoped(&scope)?))
}

/// GET /api/health — lightweight "is it alive" summary for the UI banner.
///
/// Unlike `/api/stats` this needs no admin grant: it answers "is this thing alive
/// and did my last write land" for *any* valid scope, computed from the same scoped
/// per-room aggregates the heatmap uses, so it never leaks counts across wings.
pub async fn health(
    State(state): State<AppState>,
    Extension(scope): Extension<AccessScope>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let rooms = {
        let storage = state.storage.lock().expect("storage mutex poisoned");
        storage.heatmap_scoped(&scope)?
    };
    let room_count = rooms.len();
    let drawer_count: i64 = rooms.iter().map(|r| r.drawer_count).sum();
    let last_write: Option<&String> = rooms.iter().filter_map(|r| r.last_write.as_ref()).max();
    let db_size = std::fs::metadata(&state.db_path).map(|m| m.len()).unwrap_or(0);
    Ok(Json(json!({
        "ok": true,
        "version": env!("CARGO_PKG_VERSION"),
        "room_count": room_count,
        "drawer_count": drawer_count,
        "last_write": last_write,
        "db_size_bytes": db_size,
    })))
}

/// Search query params. `q` is the FTS query; `limit` is clamped like pagination.
#[derive(Debug, Deserialize)]
pub struct SearchParams {
    q: Option<String>,
    limit: Option<usize>,
}

/// GET /api/search?q=&limit= — the copyable retrieval probe. Runs the *real* FTS
/// engine (`search_drawers_scoped`) so a dev can see exactly what retrieval returns
/// for a query, scoped to readable wings. Empty `q` returns no hits (not an error),
/// so the UI can render the probe before the user types anything.
pub async fn search(
    State(state): State<AppState>,
    Extension(scope): Extension<AccessScope>,
    Query(p): Query<SearchParams>,
) -> Result<Json<Vec<SearchHit>>, ApiError> {
    let query = p.q.unwrap_or_default();
    if query.trim().is_empty() {
        return Ok(Json(Vec::new()));
    }
    let limit = p.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let storage = state.storage.lock().expect("storage mutex poisoned");
    Ok(Json(storage.search_drawers_scoped(&scope, &query, limit)?))
}

/// Session-diff query params. `gap` is the idle-minutes threshold that splits the
/// audit trail into sessions; `limit` caps how many recent sessions are returned.
#[derive(Debug, Deserialize)]
pub struct SessionParams {
    gap: Option<i64>,
    limit: Option<usize>,
}

/// Default idle gap (minutes) that separates one write session from the next.
const SESSION_DEFAULT_GAP_MIN: i64 = 30;
/// Defaults/caps for sessions returned and entries kept per session.
const SESSION_DEFAULT_LIMIT: usize = 25;
const SESSION_MAX_LIMIT: usize = 200;
const SESSION_ENTRY_CAP: usize = 200;

/// GET /api/sessions?gap=&limit= — the diff-between-sessions view. Reconstructs write
/// sessions from the *existing* WAL (no snapshot storage): clusters of content changes
/// separated by `gap` idle minutes, each rolled up to op/wing counts plus a capped
/// sample of the actual changes. Scoped exactly like the audit log.
pub async fn sessions(
    State(state): State<AppState>,
    Extension(scope): Extension<AccessScope>,
    Query(p): Query<SessionParams>,
) -> Result<Json<Vec<WriteSession>>, ApiError> {
    let gap = p.gap.unwrap_or(SESSION_DEFAULT_GAP_MIN).clamp(1, 24 * 60);
    let limit = p.limit.unwrap_or(SESSION_DEFAULT_LIMIT).clamp(1, SESSION_MAX_LIMIT);
    let storage = state.storage.lock().expect("storage mutex poisoned");
    Ok(Json(storage.sessions_scoped(&scope, gap, limit, SESSION_ENTRY_CAP)?))
}

/// GET /api/stats — palace-level stats. Requires a global admin grant.
pub async fn stats(
    State(state): State<AppState>,
    Extension(scope): Extension<AccessScope>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_admin(&scope)?;
    let stats: PalaceStats = {
        let storage = state.storage.lock().expect("storage mutex poisoned");
        storage.palace_stats()?
    };
    let db_size = std::fs::metadata(&state.db_path).map(|m| m.len()).unwrap_or(0);
    Ok(Json(json!({
        "total_drawers": stats.total_drawers,
        "total_relations": stats.total_relations,
        "total_wings": stats.total_wings,
        "last_compact": stats.last_compact,
        "db_size_bytes": db_size,
    })))
}

// ── Token administration (global admin only) ─────────────────────────────────

fn require_admin(scope: &AccessScope) -> Result<(), ApiError> {
    if scope.is_global_admin() {
        Ok(())
    } else {
        Err(ApiError::forbidden("requires a global admin grant (*:admin)"))
    }
}

#[derive(Debug, Serialize)]
pub struct TokenView {
    id: i64,
    label: String,
    created_at: String,
    last_used_at: Option<String>,
    revoked: bool,
    grants: Vec<GrantView>,
}

#[derive(Debug, Serialize)]
pub struct GrantView {
    wing: String,
    level: String,
}

fn grant_views(grants: Vec<Grant>) -> Vec<GrantView> {
    grants
        .into_iter()
        .map(|g| GrantView { wing: g.wing, level: g.level.as_str().to_string() })
        .collect()
}

/// GET /api/tokens — list tokens (masked; secrets are never returned).
pub async fn list_tokens(
    State(state): State<AppState>,
    Extension(scope): Extension<AccessScope>,
) -> Result<Json<Vec<TokenView>>, ApiError> {
    require_admin(&scope)?;
    let storage = state.storage.lock().expect("storage mutex poisoned");
    let mut out = Vec::new();
    for t in storage.list_access_tokens()? {
        out.push(TokenView {
            id: t.id,
            label: t.label.clone(),
            created_at: t.created_at.clone(),
            last_used_at: t.last_used_at.clone(),
            revoked: t.is_revoked(),
            grants: grant_views(storage.list_grants(t.id)?),
        });
    }
    Ok(Json(out))
}

#[derive(Debug, Deserialize)]
pub struct CreateTokenReq {
    label: String,
    /// Grants as `["wing:level", ...]`, level ∈ read|write|admin, wing `*` is global.
    grants: Vec<String>,
}

/// POST /api/tokens — create a token. Returns the secret exactly once.
pub async fn create_token(
    State(state): State<AppState>,
    Extension(scope): Extension<AccessScope>,
    Json(req): Json<CreateTokenReq>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_admin(&scope)?;
    if req.label.trim().is_empty() {
        return Err(ApiError::new(axum::http::StatusCode::BAD_REQUEST, "label is required"));
    }
    let grants = parse_grants(&req.grants)?;
    if grants.is_empty() {
        return Err(ApiError::new(
            axum::http::StatusCode::BAD_REQUEST,
            "at least one grant is required (e.g. \"engineering:read\")",
        ));
    }
    let storage = state.storage.lock().expect("storage mutex poisoned");
    let (token, secret) = storage.create_access_token(&req.label, &grants)?;
    Ok(Json(json!({
        "id": token.id,
        "label": token.label,
        "secret": secret,
        "grants": grant_views(storage.list_grants(token.id)?),
    })))
}

/// DELETE /api/tokens/:id — soft-revoke a token.
pub async fn revoke_token(
    State(state): State<AppState>,
    Extension(scope): Extension<AccessScope>,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_admin(&scope)?;
    let storage = state.storage.lock().expect("storage mutex poisoned");
    let revoked = storage.revoke_access_token(id)?;
    if !revoked {
        return Err(ApiError::not_found());
    }
    Ok(Json(json!({ "revoked": true, "id": id })))
}

#[derive(Debug, Deserialize)]
pub struct GrantReq {
    wing: String,
    level: String,
}

/// POST /api/tokens/:id/grants — add or update a wing grant on a token.
pub async fn add_grant(
    State(state): State<AppState>,
    Extension(scope): Extension<AccessScope>,
    Path(id): Path<i64>,
    Json(req): Json<GrantReq>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_admin(&scope)?;
    let level = GrantLevel::parse(&req.level)
        .ok_or_else(|| ApiError::new(axum::http::StatusCode::BAD_REQUEST, "level must be read|write|admin"))?;
    let storage = state.storage.lock().expect("storage mutex poisoned");
    if storage.get_access_token(id)?.is_none() {
        return Err(ApiError::not_found());
    }
    storage.set_grant(id, req.wing.trim(), level)?;
    Ok(Json(json!({ "ok": true, "grants": grant_views(storage.list_grants(id)?) })))
}

/// Parse `["wing:level", ...]` into grants, rejecting malformed entries.
fn parse_grants(specs: &[String]) -> Result<Vec<Grant>, ApiError> {
    let mut out = Vec::new();
    for spec in specs {
        let (wing, level) = spec
            .rsplit_once(':')
            .ok_or_else(|| ApiError::new(axum::http::StatusCode::BAD_REQUEST, format!("grant '{spec}' must be WING:LEVEL")))?;
        let level = GrantLevel::parse(level)
            .ok_or_else(|| ApiError::new(axum::http::StatusCode::BAD_REQUEST, format!("grant '{spec}': bad level")))?;
        let wing = wing.trim();
        if wing.is_empty() {
            return Err(ApiError::new(axum::http::StatusCode::BAD_REQUEST, format!("grant '{spec}': empty wing")));
        }
        out.push(Grant { wing: wing.to_string(), level });
    }
    Ok(out)
}
