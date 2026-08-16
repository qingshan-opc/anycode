//! Skill App static assets, bindings, and state (ADR 020).

use crate::api::state::AppState;
use anycode_tools::{load_skill_surface, skill_ui_dir, SkillCatalog, SkillSurface};
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path as FsPath, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillAppBinding {
    pub skill_id: String,
    pub slot: String,
    pub enabled: bool,
    pub title: Option<String>,
    pub icon: Option<String>,
    pub has_ui: bool,
    pub surface: Option<SkillSurface>,
}

#[derive(Debug, Deserialize)]
pub struct BindSkillAppBody {
    pub skill_id: String,
    #[serde(default = "default_dock")]
    pub slot: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_dock() -> String {
    "dock".into()
}
fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize)]
pub struct SkillAppStateBody {
    #[serde(default)]
    pub state: Option<Value>,
    #[serde(default)]
    pub brief: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub struct AssetQuery {
    #[serde(default)]
    pub path: Option<String>,
}

fn home_skill_roots(project_root: Option<&FsPath>) -> Vec<PathBuf> {
    let mut extra = Vec::new();
    if let Some(pr) = project_root {
        extra.push(pr.join("skills"));
        extra.push(pr.join(".anycode/skills"));
    }
    anycode_tools::default_skill_roots(&extra, dirs::home_dir().as_deref())
}

fn resolve_skill_root(skill_id: &str, project_root: Option<&FsPath>) -> Option<PathBuf> {
    let roots = home_skill_roots(project_root);
    let catalog = SkillCatalog::scan(&roots, None, 120_000, true);
    catalog.resolve_skill_root(skill_id, project_root)
}

fn mime_for(path: &FsPath) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "yaml" | "yml" => "text/plain; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}

fn serve_file(path: &FsPath) -> Response {
    let Ok(bytes) = std::fs::read(path) else {
        return (StatusCode::NOT_FOUND, "unreadable").into_response();
    };
    let mut res = Response::new(Body::from(bytes));
    *res.status_mut() = StatusCode::OK;
    res.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static(mime_for(path)),
    );
    res.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    res.headers_mut().insert(
        header::HeaderName::from_static("x-content-type-options"),
        header::HeaderValue::from_static("nosniff"),
    );
    res
}

/// GET /api/skill-apps/{skill_id}/asset?path=index.html
pub async fn get_skill_app_asset(
    State(state): State<AppState>,
    Path(skill_id): Path<String>,
    Query(q): Query<AssetQuery>,
) -> Response {
    if !SkillCatalog::is_valid_skill_id(&skill_id) {
        return (StatusCode::BAD_REQUEST, "invalid skill_id").into_response();
    }
    let rel = q
        .path
        .as_deref()
        .unwrap_or("index.html")
        .trim_start_matches('/');
    let project_root = project_root_for_skill(&state, &skill_id).await;
    let Some(root) = resolve_skill_root(&skill_id, project_root.as_deref()) else {
        return (StatusCode::NOT_FOUND, "skill not found").into_response();
    };
    let Some(ui) = skill_ui_dir(&root) else {
        return (StatusCode::NOT_FOUND, "skill has no ui/").into_response();
    };
    let Ok(canon_ui) = ui.canonicalize() else {
        return (StatusCode::NOT_FOUND, "ui missing").into_response();
    };
    let candidate = ui.join(rel);
    let Ok(canon) = candidate.canonicalize() else {
        return (StatusCode::NOT_FOUND, "asset not found").into_response();
    };
    if !canon.starts_with(&canon_ui) {
        return (StatusCode::FORBIDDEN, "path escape").into_response();
    }
    serve_file(&canon)
}

async fn project_root_for_skill(state: &AppState, _skill_id: &str) -> Option<PathBuf> {
    // Prefer first project that has this skill bound; else None (user-global skills).
    let row: Option<String> = sqlx::query_scalar(
        "SELECT project_id FROM project_skill_apps WHERE skill_id = ? AND enabled = 1 LIMIT 1",
    )
    .bind(_skill_id)
    .fetch_optional(state.db.pool())
    .await
    .ok()
    .flatten();
    if let Some(pid) = row {
        if let Ok(Some(p)) = state.db.get_project(&pid).await {
            return Some(PathBuf::from(p.root_path));
        }
    }
    None
}

pub async fn list_project_skill_apps(
    State(state): State<AppState>,
    Path(project_id): Path<String>,
) -> impl IntoResponse {
    match load_bindings(&state, &project_id).await {
        Ok(rows) => Json(json!({ "apps": rows })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn load_bindings(state: &AppState, project_id: &str) -> anyhow::Result<Vec<SkillAppBinding>> {
    let rows: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT skill_id, slot, enabled FROM project_skill_apps WHERE project_id = ? ORDER BY skill_id",
    )
    .bind(project_id)
    .fetch_all(state.db.pool())
    .await?;

    let project_root = state
        .db
        .get_project(project_id)
        .await?
        .map(|p| PathBuf::from(p.root_path));

    let mut out = Vec::new();
    for (skill_id, slot, enabled) in rows {
        let surface = resolve_skill_root(&skill_id, project_root.as_deref())
            .and_then(|r| load_skill_surface(&r));
        let (title, icon) = surface
            .as_ref()
            .map(|s| (Some(s.title.clone()), s.icon.clone()))
            .unwrap_or((None, None));
        out.push(SkillAppBinding {
            skill_id,
            slot,
            enabled: enabled != 0,
            title,
            icon,
            has_ui: surface.is_some(),
            surface,
        });
    }
    Ok(out)
}

pub async fn put_project_skill_app(
    State(state): State<AppState>,
    Path(project_id): Path<String>,
    Json(body): Json<BindSkillAppBody>,
) -> impl IntoResponse {
    if !SkillCatalog::is_valid_skill_id(&body.skill_id) {
        return (StatusCode::BAD_REQUEST, "invalid skill_id").into_response();
    }
    let slot = match body.slot.as_str() {
        "dock" | "conversation" | "project" => body.slot.clone(),
        _ => "dock".into(),
    };
    let r = sqlx::query(
        r#"
        INSERT INTO project_skill_apps (project_id, skill_id, slot, enabled, updated_at)
        VALUES (?, ?, ?, ?, datetime('now'))
        ON CONFLICT(project_id, skill_id) DO UPDATE SET
          slot = excluded.slot,
          enabled = excluded.enabled,
          updated_at = datetime('now')
        "#,
    )
    .bind(&project_id)
    .bind(&body.skill_id)
    .bind(&slot)
    .bind(if body.enabled { 1 } else { 0 })
    .execute(state.db.pool())
    .await;
    match r {
        Ok(_) => {
            Json(json!({ "ok": true, "skill_id": body.skill_id, "slot": slot })).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub async fn delete_project_skill_app(
    State(state): State<AppState>,
    Path((project_id, skill_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let r = sqlx::query("DELETE FROM project_skill_apps WHERE project_id = ? AND skill_id = ?")
        .bind(&project_id)
        .bind(&skill_id)
        .execute(state.db.pool())
        .await;
    match r {
        Ok(_) => Json(json!({ "ok": true })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub async fn get_skill_app_state(
    State(state): State<AppState>,
    Path((project_id, skill_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let row: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT state_json, brief_json FROM skill_app_state WHERE project_id = ? AND skill_id = ?",
    )
    .bind(&project_id)
    .bind(&skill_id)
    .fetch_optional(state.db.pool())
    .await
    .ok()
    .flatten();
    let (state_v, brief_v) = match row {
        Some((s, b)) => (
            serde_json::from_str(&s).unwrap_or(json!({})),
            b.and_then(|t| serde_json::from_str(&t).ok()),
        ),
        None => (json!({}), None),
    };
    mirror_state_file(&project_id, &skill_id, &state_v, brief_v.as_ref());
    Json(json!({
        "project_id": project_id,
        "skill_id": skill_id,
        "state": state_v,
        "brief": brief_v,
    }))
    .into_response()
}

pub async fn put_skill_app_state(
    State(state): State<AppState>,
    Path((project_id, skill_id)): Path<(String, String)>,
    Json(body): Json<SkillAppStateBody>,
) -> impl IntoResponse {
    match upsert_state(&state, &project_id, &skill_id, body.state, body.brief).await {
        Ok((state_v, brief_v)) => Json(json!({
            "ok": true,
            "state": state_v,
            "brief": brief_v,
        }))
        .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn upsert_state(
    state: &AppState,
    project_id: &str,
    skill_id: &str,
    state_patch: Option<Value>,
    brief_patch: Option<Value>,
) -> anyhow::Result<(Value, Option<Value>)> {
    let existing: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT state_json, brief_json FROM skill_app_state WHERE project_id = ? AND skill_id = ?",
    )
    .bind(project_id)
    .bind(skill_id)
    .fetch_optional(state.db.pool())
    .await?;
    let mut state_v: Value = existing
        .as_ref()
        .and_then(|(s, _)| serde_json::from_str(s).ok())
        .unwrap_or(json!({}));
    let mut brief_v: Option<Value> = existing
        .as_ref()
        .and_then(|(_, b)| b.as_ref().and_then(|t| serde_json::from_str(t).ok()));
    if let Some(s) = state_patch {
        state_v = s;
    }
    if let Some(b) = brief_patch {
        brief_v = Some(b);
    }
    let brief_str = brief_v.as_ref().and_then(|v| serde_json::to_string(v).ok());
    sqlx::query(
        r#"
        INSERT INTO skill_app_state (project_id, skill_id, state_json, brief_json, updated_at)
        VALUES (?, ?, ?, ?, datetime('now'))
        ON CONFLICT(project_id, skill_id) DO UPDATE SET
          state_json = excluded.state_json,
          brief_json = excluded.brief_json,
          updated_at = datetime('now')
        "#,
    )
    .bind(project_id)
    .bind(skill_id)
    .bind(state_v.to_string())
    .bind(brief_str.as_deref())
    .execute(state.db.pool())
    .await?;
    mirror_state_file(project_id, skill_id, &state_v, brief_v.as_ref());
    Ok((state_v, brief_v))
}

fn mirror_state_file(project_id: &str, skill_id: &str, state: &Value, brief: Option<&Value>) {
    let Some(home) = dirs::home_dir() else {
        return;
    };
    let dir = home
        .join(".anycode/dashboard/skill-apps/state")
        .join(project_id);
    let _ = std::fs::create_dir_all(&dir);
    let payload = json!({ "state": state, "brief": brief });
    let _ = std::fs::write(
        dir.join(format!("{skill_id}.json")),
        serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".into()),
    );
}

pub async fn list_skill_apps_catalog(State(_state): State<AppState>) -> impl IntoResponse {
    let roots = home_skill_roots(None);
    let catalog = SkillCatalog::scan(&roots, None, 120_000, true);
    let apps: Vec<Value> = catalog
        .metas()
        .iter()
        .filter(|s| s.has_ui)
        .map(|s| {
            json!({
                "skill_id": s.id,
                "title": s.surface.as_ref().map(|x| &x.title).unwrap_or(&s.id),
                "icon": s.surface.as_ref().and_then(|x| x.icon.clone()),
                "default_slot": s.surface.as_ref().map(|x| x.default_slot.as_str()),
                "slots": s.surface.as_ref().map(|x| {
                    x.slots.iter().map(|sl| sl.as_str()).collect::<Vec<_>>()
                }),
            })
        })
        .collect();
    Json(json!({ "apps": apps }))
}

#[derive(Debug, Deserialize)]
pub struct PendingQuery {
    session_id: Option<String>,
    limit: Option<usize>,
}

pub async fn list_pending_skill_apps(Query(q): Query<PendingQuery>) -> impl IntoResponse {
    let rows = anycode_dashboard_ipc::skill_app_ipc::list_pending_for_session(
        q.session_id.as_deref(),
        q.limit.unwrap_or(20),
    );
    Json(json!({ "pending": rows }))
}

/// True when the prompt already carries a locked VisualBrief (host resume or
/// Skill App continuation). Those must not open another style picker.
pub fn prompt_already_has_visual_brief(prompt: &str) -> bool {
    let p = prompt.trim();
    p.contains("[Host VisualBrief")
        || p.contains("[SkillApp]")
        || p.starts_with("VisualBrief locked")
}

#[must_use]
pub fn prompt_with_visual_brief(user_prompt: &str, skill_id: &str, brief: &Value) -> String {
    let json = serde_json::to_string_pretty(brief).unwrap_or_else(|_| brief.to_string());
    format!(
        "{user_prompt}\n\n[Host VisualBrief for `{skill_id}` — the user already picked this skin in the Skill App. Follow brief.tokens (bg/ink/accent/fonts). Infer outline and page count from the user topic; create original main visuals (SVG/canvas/local ECharts). Do NOT copy all templates or pad to 12 pages. Do NOT call SkillAppPresent or AskUserQuestion for style.]\n```json\n{json}\n```"
    )
}

#[derive(Debug, Deserialize)]
pub struct SubmitBriefBody {
    pub skill_id: String,
    pub brief: Value,
    #[serde(default)]
    pub present_id: Option<String>,
}

pub async fn submit_skill_app_brief(
    State(state): State<AppState>,
    Path(project_id): Path<String>,
    Json(body): Json<SubmitBriefBody>,
) -> impl IntoResponse {
    if let Err(e) = upsert_state(
        &state,
        &project_id,
        &body.skill_id,
        None,
        Some(body.brief.clone()),
    )
    .await
    {
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }
    let mut submitted = false;
    if let Some(pid) = body.present_id.as_deref().filter(|s| !s.is_empty()) {
        if anycode_dashboard_ipc::skill_app_ipc::submit_brief(
            pid,
            &body.skill_id,
            body.brief.clone(),
        )
        .is_ok()
        {
            submitted = true;
        }
    }
    if !submitted {
        for p in anycode_dashboard_ipc::skill_app_ipc::list_pending_for_session(None, 50) {
            if p.skill_id == body.skill_id && p.wait_brief {
                if anycode_dashboard_ipc::skill_app_ipc::submit_brief(
                    &p.present_id,
                    &body.skill_id,
                    body.brief.clone(),
                )
                .is_ok()
                {
                    submitted = true;
                }
            }
        }
    }
    Json(json!({ "ok": true, "resumed": submitted })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn prompt_embeds_brief_and_forbids_present_tool() {
        let p = prompt_with_visual_brief(
            "做个ppt",
            "anycode-ppt",
            &json!({"family": "apple-keynote"}),
        );
        assert!(p.contains("Host VisualBrief"));
        assert!(p.contains("Do NOT call SkillAppPresent"));
        assert!(p.contains("Infer outline"));
        assert!(p.contains("apple-keynote"));
    }

    #[test]
    fn continuation_prompts_do_not_reopen_the_studio() {
        assert!(prompt_already_has_visual_brief(
            "做个ppt\n\n[Host VisualBrief for `anycode-ppt` — follow it]"
        ));
        assert!(prompt_already_has_visual_brief(
            "[SkillApp] VisualBrief locked for anycode-ppt"
        ));
        assert!(prompt_already_has_visual_brief(
            "VisualBrief locked. Style family = fde-editorial."
        ));
        assert!(!prompt_already_has_visual_brief("帮我做个ai投标的ppt"));
    }
}
