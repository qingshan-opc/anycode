//! Ticket-hop SSO from lingxi-accounts (WeChat QR / in-WeChat OAuth).
//! Plants anyCode portal session token. `/m` and `/console` are allowed `next` paths.

use crate::api::AppState;
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, StatusCode};
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
        || raw.contains('\\')
        || raw.chars().any(char::is_control)
        || raw.contains('%')
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
        tracing::warn!(%status, "lingxi sso consume rejected");
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

    let user = match crate::store::resolve_lingxi_user(
        &state.db,
        &lingxi_user_id,
        email.as_deref(),
        phone.as_deref(),
    )
    .await
    {
        Ok(user) => user,
        Err(e) => {
            if e.is::<crate::store::IdentityLinkRequired>() {
                return (StatusCode::CONFLICT, Json(serde_json::json!({"error":"existing account requires verified identity linking"}))).into_response();
            }
            tracing::error!("lingxi hop provision failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "provision failed" })),
            )
                .into_response();
        }
    };

    let browser = crate::auth::new_session_token();
    let code = match crate::store::create_portal_login_code(&state.db, &user.id, &browser).await {
        Ok(code) => code,
        Err(_) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error":"handoff unavailable"})),
            )
                .into_response()
        }
    };
    let portal = match reqwest::Url::parse(&state.config.portal_url) {
        Ok(url) => url,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let mut dest = portal.clone();
    dest.set_path("/login");
    dest.set_query(None);
    dest.set_fragment(None);
    dest.query_pairs_mut()
        .append_pair("lx_code", &code)
        .append_pair("next", &next);
    let mut response = Redirect::temporary(dest.as_str()).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    response
        .headers_mut()
        .insert(header::REFERRER_POLICY, "no-referrer".parse().unwrap());
    response.headers_mut().append(
        header::SET_COOKIE,
        proof_cookie(&portal, &browser, 90).parse().unwrap(),
    );
    response.into_response()
}

fn proof_cookie(portal: &reqwest::Url, value: &str, ttl: u32) -> String {
    format!(
        "anycode_hop_proof={value}; Path=/api/auth/hop; HttpOnly; SameSite=Lax; Max-Age={ttl}{}",
        if portal.scheme() == "https" {
            "; Secure"
        } else {
            ""
        }
    )
}
fn proof_cookie_value(headers: &HeaderMap) -> Option<&str> {
    let mut found = None;
    for line in headers.get_all(header::COOKIE) {
        for part in line.to_str().ok()?.split(';') {
            if let Some(value) = part.trim().strip_prefix("anycode_hop_proof=") {
                if found.is_some() || !valid_exchange(value) {
                    return None;
                }
                found = Some(value);
            }
        }
    }
    found
}
fn valid_exchange(raw: &str) -> bool {
    (32..=128).contains(&raw.len())
        && raw
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Exchange {
    code: String,
}
pub async fn hop_exchange(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Exchange>,
) -> Response {
    let portal = match reqwest::Url::parse(&state.config.portal_url) {
        Ok(url) => url,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    if headers.get(header::ORIGIN).and_then(|v| v.to_str().ok())
        != Some(portal.origin().ascii_serialization().as_str())
        || !valid_exchange(&body.code)
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Some(browser) = proof_cookie_value(&headers) else {
        return StatusCode::FORBIDDEN.into_response();
    };
    let token = match crate::store::consume_portal_login_code(&state.db, &body.code, browser).await
    {
        Ok(Some(token)) => token,
        Ok(None) => return StatusCode::UNAUTHORIZED.into_response(),
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let mut response = Json(serde_json::json!({"token":token})).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    response.headers_mut().append(
        header::SET_COOKIE,
        proof_cookie(&portal, "", 0).parse().unwrap(),
    );
    response
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
    use super::*;

    #[test]
    fn proof_is_host_only_and_exchange_requires_well_formed_values() {
        let portal = reqwest::Url::parse("https://anycode.work").unwrap();
        let value = proof_cookie(&portal, "opaque", 90);
        assert!(value.contains("HttpOnly") && value.contains("Secure"));
        assert!(!value.contains("Domain="));
        for raw in ["", "../auth", "a b", "a;Secure"] {
            assert!(!valid_exchange(raw));
        }
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            format!("anycode_hop_proof={}", "x".repeat(43))
                .parse()
                .unwrap(),
        );
        assert!(proof_cookie_value(&headers).is_some());
        headers.append(
            header::COOKIE,
            format!("anycode_hop_proof={}", "x".repeat(43))
                .parse()
                .unwrap(),
        );
        assert!(proof_cookie_value(&headers).is_none());
    }

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
