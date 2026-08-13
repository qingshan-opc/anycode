//! P1.8: persist transcript diagrams and serve them to the `/diagram/{id}`
//! full-address page. POST lives under /api (authed, UI persist-on-render);
//! the source read is top-level and unauthenticated so LAN colleagues with a
//! share link can open the page.

use super::*;
use crate::db::DiagramRecord;

#[derive(Deserialize)]
pub struct PostDiagramBody {
    pub session_id: Option<String>,
    pub kind: String,
    pub source: String,
    pub title: Option<String>,
}

pub async fn post_diagram(
    State(state): State<AppState>,
    Json(body): Json<PostDiagramBody>,
) -> impl IntoResponse {
    let kind = body.kind.trim().to_string();
    match state
        .db
        .diagrams_save(
            body.session_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty()),
            &kind,
            &body.source,
            body.title
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty()),
        )
        .await
    {
        Ok(id) => Json(json!({ "id": id, "url": format!("/diagram/{id}") })).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// Top-level (non-/api) read for the share page — intentionally outside the
/// auth middleware: opening a shared diagram link must not require a token.
/// Read-only, id-addressed, source was already shown in a transcript. State
/// arrives by capture (the top-level router is `Router<()>`), not `State<>`.
pub async fn get_diagram_source_shared(
    state: AppState,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> impl IntoResponse {
    let id = id.trim();
    if !id.starts_with("dgm_") || id.len() > 64 {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "invalid diagram id" })),
        )
            .into_response();
    }
    match state.db.diagrams_get(id).await {
        Ok(Some(DiagramRecord {
            id,
            session_id,
            kind,
            source,
            title,
            created_at,
        })) => Json(json!({
            "id": id,
            "session_id": session_id,
            "kind": kind,
            "source": source,
            "title": title,
            "created_at": created_at,
        }))
        .into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "diagram not found" })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}
