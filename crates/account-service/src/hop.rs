//! Ticket-hop SSO from lingxi-accounts (WeChat QR / in-WeChat OAuth).
//! Plants anyCode portal session token. `/m` and `/console` are allowed `next` paths.

use crate::api::AppState;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::Value;

const AUD: &str = "anycode";
const DEFAULT_NEXT: &str = "/console";

#[derive(Debug, Deserialize)]
pub struct HopLoginQuery {
    pub next: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct HopCallbackQuery {
    pub ticket: Option<String>,
    pub next: Option<String>,
}

pub fn safe_next(raw: Option<&str>) -> String {
    let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return DEFAULT_NEXT.into();
    };
    if is_safe_app_next(raw) {
        raw.to_string()
    } else {
        DEFAULT_NEXT.into()
    }
}

fn is_safe_app_next(raw: &str) -> bool {
    if !raw.starts_with('/')
        || raw.starts_with("//")
        || raw.contains('\n')
        || raw.contains('\r')
        || raw.contains("..")
    {
        return false;
    }
    raw == "/m"
        || raw.starts_with("/m/")
        || raw.starts_with("/m?")
        || raw == "/console"
        || raw.starts_with("/console/")
        || raw.starts_with("/console?")
}

fn accounts_origin(state: &AppState) -> String {
    state.config.accounts_url.trim_end_matches('/').to_string()
}

pub async fn hop_login(State(state): State<AppState>, Query(q): Query<HopLoginQuery>) -> Response {
    let accounts = accounts_origin(&state);
    let next = safe_next(q.next.as_deref());
    let dest = format!(
        "{accounts}/api/v1/sso/start?aud={AUD}&next={}",
        urlencoding::encode(&next)
    );
    Redirect::temporary(&dest).into_response()
}

pub async fn hop_callback(
    State(state): State<AppState>,
    Query(q): Query<HopCallbackQuery>,
) -> Response {
    let Some(ticket) = q.ticket.as_deref().map(str::trim).filter(|s| !s.is_empty()) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "missing ticket" })),
        )
            .into_response();
    };
    let next = safe_next(q.next.as_deref());
    let accounts = accounts_origin(&state);

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .user_agent(concat!("anycode-account/", env!("CARGO_PKG_VERSION")))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("http client: {e}") })),
            )
                .into_response();
        }
    };

    let resp = match client
        .post(format!("{accounts}/api/v1/sso/consume"))
        .json(&serde_json::json!({ "ticket": ticket, "aud": AUD }))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "lingxi sso consume request failed");
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "error": "accounts unavailable" })),
            )
                .into_response();
        }
    };

    let status = resp.status();
    let body: Value = match resp.json().await {
        Ok(v) => v,
        Err(_) => serde_json::json!({}),
    };

    if status.as_u16() == 403 {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "audience mismatch" })),
        )
            .into_response();
    }
    if !status.is_success() {
        tracing::warn!(%status, body = %body, "lingxi sso consume rejected");
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "ticket expired or already used" })),
        )
            .into_response();
    }

    let lingxi_user_id = body
        .get("user_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if lingxi_user_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "missing user_id" })),
        )
            .into_response();
    }

    let email = body
        .get("email")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_lowercase());
    let phone = body
        .get("phone")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let token = match crate::store::login_or_provision_lingxi(
        &state.db,
        &lingxi_user_id,
        email.as_deref(),
        phone.as_deref(),
    )
    .await
    {
        Ok((_user, token)) => token,
        Err(e) => {
            tracing::error!(error = %e, "lingxi hop provision failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "provision failed" })),
            )
                .into_response();
        }
    };

    let portal = state.config.portal_url.trim_end_matches('/');
    let dest = format!(
        "{portal}/login?lx_token={}&next={}",
        urlencoding::encode(&token),
        urlencoding::encode(&next)
    );
    Redirect::temporary(&dest).into_response()
}

mod urlencoding {
    /// Minimal query-component encoding (RFC 3986 unreserved stay literal).
    pub fn encode(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        for b in s.bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    out.push(b as char);
                }
                _ => out.push_str(&format!("%{b:02X}")),
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::safe_next;

    #[test]
    fn safe_next_defaults_and_rejects_open_redirect() {
        assert_eq!(safe_next(None), "/console");
        assert_eq!(safe_next(Some("")), "/console");
        assert_eq!(safe_next(Some("/console/plans")), "/console/plans");
        assert_eq!(safe_next(Some("/m")), "/m");
        assert_eq!(safe_next(Some("/m?d=dev1")), "/m?d=dev1");
        assert_eq!(safe_next(Some("//evil.com")), "/console");
        assert_eq!(safe_next(Some("https://evil.com")), "/console");
        assert_eq!(safe_next(Some("/m/../../evil")), "/console");
    }
}
