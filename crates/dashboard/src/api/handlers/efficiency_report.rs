use super::*;

/// GET /reports/efficiency/latest — 最新一份每周效能报告（无则 404）。
pub async fn get_latest_efficiency_report(State(_state): State<AppState>) -> impl IntoResponse {
    match crate::observability::efficiency_report::read_latest_report() {
        Ok(Some(report)) => Json(json!({ "report": report })).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "no efficiency report generated yet" })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}
