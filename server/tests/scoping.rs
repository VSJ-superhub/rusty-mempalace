//! HTTP-level proof that wing scoping is enforced: a token scoped to one wing
//! cannot see another wing's data through any endpoint, and auth is required once
//! tokens exist.

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use tower::ServiceExt; // oneshot

use yourmemory_core::access::{Grant, GrantLevel};
use yourmemory_core::storage::{Confidence, NewDrawer, Source, SqliteStorage, Storage};
use yourmemory_server::{build_router, AppState};

struct Fixture {
    app: axum::Router,
    secret: String,
    eng_wing_id: i64,
    legal_wing_id: i64,
    legal_room_id: i64,
    legal_drawer_id: i64,
}

fn fixture() -> Fixture {
    let s = SqliteStorage::open_in_memory().unwrap();

    let eng = s.create_wing("engineering", None).unwrap();
    let eng_room = s.create_room(eng.id, "auth", None).unwrap();
    s.store_drawer(&NewDrawer {
        wing_id: eng.id, room_id: eng_room.id,
        content: "eng secret".into(), compressed_content: None,
        confidence: Confidence::High, source: Source::User,
    }).unwrap();

    let legal = s.create_wing("legal", None).unwrap();
    let legal_room = s.create_room(legal.id, "contracts", None).unwrap();
    let legal_drawer = s.store_drawer(&NewDrawer {
        wing_id: legal.id, room_id: legal_room.id,
        content: "legal secret".into(), compressed_content: None,
        confidence: Confidence::High, source: Source::User,
    }).unwrap();

    let (_token, secret) = s
        .create_access_token(
            "eng-only",
            &[Grant { wing: "engineering".into(), level: GrantLevel::Read }],
        )
        .unwrap();

    let state = AppState {
        storage: Arc::new(Mutex::new(s)),
        is_loopback: true,
        db_path: std::path::PathBuf::from(":memory:"),
    };
    Fixture {
        app: build_router(state),
        secret,
        eng_wing_id: eng.id,
        legal_wing_id: legal.id,
        legal_room_id: legal_room.id,
        legal_drawer_id: legal_drawer.id,
    }
}

async fn get(app: &axum::Router, uri: &str, bearer: Option<&str>) -> (StatusCode, String) {
    let mut builder = Request::builder().uri(uri).method("GET");
    if let Some(b) = bearer {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {b}"));
    }
    let resp = app.clone().oneshot(builder.body(Body::empty()).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

#[tokio::test]
async fn lists_only_readable_wings() {
    let f = fixture();
    let (status, body) = get(&f.app, "/api/wings", Some(&f.secret)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("engineering"), "body: {body}");
    assert!(!body.contains("legal"), "leaked legal wing: {body}");
}

#[tokio::test]
async fn readable_wing_rooms_visible() {
    let f = fixture();
    let (status, _) = get(&f.app, &format!("/api/wings/{}/rooms", f.eng_wing_id), Some(&f.secret)).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn out_of_scope_wing_rooms_404() {
    let f = fixture();
    let (status, _) = get(&f.app, &format!("/api/wings/{}/rooms", f.legal_wing_id), Some(&f.secret)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn out_of_scope_room_drawers_404() {
    let f = fixture();
    let (status, _) = get(&f.app, &format!("/api/rooms/{}/drawers", f.legal_room_id), Some(&f.secret)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn out_of_scope_drawer_404() {
    let f = fixture();
    let (status, _) = get(&f.app, &format!("/api/drawers/{}", f.legal_drawer_id), Some(&f.secret)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn missing_token_is_401_when_tokens_exist() {
    let f = fixture();
    let (status, _) = get(&f.app, "/api/wings", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn invalid_token_is_401() {
    let f = fixture();
    let (status, _) = get(&f.app, "/api/wings", Some("not-a-real-secret")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn graph_scoped_to_readable_wings() {
    let f = fixture();
    let (status, body) = get(&f.app, "/api/graph", Some(&f.secret)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("engineering"), "body: {body}");
    assert!(!body.contains("legal"), "graph leaked legal wing: {body}");
}

#[tokio::test]
async fn heatmap_scoped_to_readable_wings() {
    let f = fixture();
    let (status, body) = get(&f.app, "/api/heatmap", Some(&f.secret)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("engineering"), "body: {body}");
    assert!(!body.contains("legal"), "heatmap leaked legal wing: {body}");
}

#[tokio::test]
async fn search_scoped_to_readable_wings() {
    // The FTS engine is the highest-risk leak vector: raw `search_drawers` ignores
    // scope entirely, so `/api/search` must filter to readable wings. Both drawers
    // contain the word "secret"; an engineering token must see only its own hit.
    let f = fixture();
    let (status, body) = get(&f.app, "/api/search?q=secret", Some(&f.secret)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("eng secret"), "missing own hit: {body}");
    assert!(!body.contains("legal"), "search leaked legal wing: {body}");
}

#[tokio::test]
async fn empty_query_search_is_ok_and_empty() {
    // The probe renders before the user types: empty `q` is not an error, just no hits.
    let f = fixture();
    let (status, body) = get(&f.app, "/api/search?q=", Some(&f.secret)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "[]", "empty query should return no hits: {body}");
}

#[tokio::test]
async fn sessions_scoped_to_readable_wings() {
    // Sessions are reconstructed from the WAL via `audit_scoped`; the legal drawer's
    // write must not surface in an engineering-scoped session diff.
    let f = fixture();
    let (status, body) = get(&f.app, "/api/sessions?gap=30&limit=50", Some(&f.secret)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("engineering"), "missing own activity: {body}");
    assert!(!body.contains("legal"), "sessions leaked legal wing: {body}");
}

#[tokio::test]
async fn health_is_non_admin_and_scoped() {
    // Unlike /api/stats (admin-only, see below), /api/health answers for any valid
    // scope — and its counts come from the scoped heatmap, so an engineering token
    // sees only its own single drawer, not the legal wing's.
    let f = fixture();
    let (status, body) = get(&f.app, "/api/health", Some(&f.secret)).await;
    assert_eq!(status, StatusCode::OK, "health must not require admin: {body}");
    assert!(body.contains("\"drawer_count\":1"), "leaked cross-wing counts: {body}");
}

#[tokio::test]
async fn stats_requires_admin() {
    let f = fixture();
    // The fixture token is engineering:read — not a global admin.
    let (status, _) = get(&f.app, "/api/stats", Some(&f.secret)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn tokens_admin_requires_admin() {
    let f = fixture();
    let (status, _) = get(&f.app, "/api/tokens", Some(&f.secret)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}
