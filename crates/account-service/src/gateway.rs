//! In-process hosted chat proxy (`/v1/chat/completions`) for DeepSeek.
//! Registered before the portal SPA so `/v1/*` is not swallowed as static files.

use crate::api::{json_error, AppState};
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use bytes::Bytes;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::Value;
use std::sync::OnceLock;

#[derive(Deserialize)]
pub struct GatewayAuthBody {
    pub api_key: Option<String>,
    pub access_token: Option<String>,
    pub model_id: String,
    #[serde(default)]
    pub exclude_account_ids: Vec<String>,
}

#[derive(Deserialize)]
struct ChatRequest {
    pub model: String,
    #[serde(flatten)]
    pub rest: Value,
}

fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

pub async fn authorize_http(
    State(state): State<AppState>,
    Json(body): Json<GatewayAuthBody>,
) -> impl IntoResponse {
    match authorize_hosted(&state, body).await {
        Ok(v) => Json(v).into_response(),
        Err((status, msg)) => json_error(status, &msg).into_response(),
    }
}

pub async fn authorize_hosted(
    state: &AppState,
    body: GatewayAuthBody,
) -> Result<Value, (StatusCode, String)> {
    let org_id = if let Some(key) = body.api_key.as_deref() {
        crate::usage::resolve_org_by_api_key(&state.db, key)
            .await
            .ok()
            .flatten()
    } else if let Some(tok) = body.access_token.as_deref() {
        crate::usage::resolve_org_by_session(&state.db, tok)
            .await
            .ok()
            .flatten()
    } else {
        None
    };
    let Some(org_id) = org_id else {
        return Err((StatusCode::UNAUTHORIZED, "invalid credentials".into()));
    };
    if !crate::store::organization_has_verified_identity(&state.db, &org_id)
        .await
        .unwrap_or(false)
    {
        return Err((
            StatusCode::FORBIDDEN,
            "verified cloud identity required".into(),
        ));
    }
    let sub = crate::store::get_subscription(&state.db, &org_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let ent = crate::store::get_entitlements(&state.db, &org_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if !ent.hosted_models_enabled {
        return Err((
            StatusCode::FORBIDDEN,
            "hosted models not enabled for plan".into(),
        ));
    }
    if ent.credit_balance_fen <= 0 {
        return Err((
            StatusCode::PAYMENT_REQUIRED,
            "credit balance exhausted — top up or subscribe to continue".into(),
        ));
    }
    if body.model_id != "auto" && !crate::usage::is_allowed_hosted_model(&body.model_id) {
        return Err((StatusCode::BAD_REQUEST, "model not supported".into()));
    }
    crate::quota::check_call_quota(&state.db, &org_id)
        .await
        .map_err(|e| (StatusCode::TOO_MANY_REQUESTS, e.to_string()))?;
    let models = crate::models_catalog::list_models(&state.db, &sub.plan)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let allowed = if body.model_id == "auto" {
        models.iter().any(|m| m.id == "auto" && m.available)
    } else {
        models.iter().any(|m| m.id == body.model_id && m.available)
    };
    if !allowed {
        return Err((StatusCode::FORBIDDEN, "model not available for plan".into()));
    }
    let resolved_model_id = crate::usage::resolve_model_id(&state.db, &sub.plan, &body.model_id)
        .await
        .map_err(|e| (StatusCode::FORBIDDEN, e.to_string()))?;
    let upstream = crate::models_catalog::get_model_upstream(&state.db, &resolved_model_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let Some((provider_id, upstream_model)) = upstream else {
        return Err((StatusCode::NOT_FOUND, "model not found".into()));
    };
    if provider_id.eq_ignore_ascii_case("agnes") {
        return Err((
            StatusCode::BAD_REQUEST,
            "agnes is no longer supported".into(),
        ));
    }

    let mut auth_json = serde_json::json!({
        "organization_id": org_id,
        "provider_id": provider_id,
        "upstream_model": upstream_model,
        "requested_model_id": body.model_id,
        "resolved_model_id": resolved_model_id,
        "token_limit": ent.token_limit,
        "tokens_used": ent.tokens_used,
        "credit_balance_fen": ent.credit_balance_fen,
        "calls_remaining": ent.calls_remaining,
        "quota_resets_at": ent.quota_resets_at,
    });

    if let Some(secret) = state.config.upstream_key_encryption_secret.as_deref() {
        match crate::upstream_pool::select_upstream_credential(
            &state.db,
            secret,
            &provider_id,
            &body.exclude_account_ids,
        )
        .await
        {
            Ok(Some(cred)) => {
                auth_json["upstream_account_id"] = cred.account_id.into();
                auth_json["upstream_api_key"] = cred.api_key.into();
                auth_json["upstream_base_url"] = cred.base_url.into();
                return Ok(auth_json);
            }
            Ok(None) => {}
            Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
        }
    }
    if let Some((key, url)) = env_deepseek_fallback() {
        auth_json["upstream_api_key"] = key.into();
        auth_json["upstream_base_url"] = url.into();
        return Ok(auth_json);
    }
    Err((
        StatusCode::SERVICE_UNAVAILABLE,
        "no available upstream account in pool".into(),
    ))
}

fn env_deepseek_fallback() -> Option<(String, String)> {
    let key = std::env::var("DEEPSEEK_API_KEY")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())?;
    Some((key, crate::upstream_pool::default_deepseek_base_url()))
}

fn extract_auth(headers: &HeaderMap) -> (Option<String>, Option<String>) {
    let auth = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.trim().to_string());
    if let Some(ref a) = auth {
        if a.starts_with("ackey_") {
            return (Some(a.clone()), None);
        }
        return (None, auth);
    }
    (None, None)
}

fn wants_stream(rest: &Value) -> bool {
    rest.get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

pub async fn chat_completions(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    match chat_completions_inner(state, headers, body).await {
        Ok(resp) => resp,
        Err((status, msg)) => json_error(status, &msg).into_response(),
    }
}

async fn chat_completions_inner(
    state: AppState,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, (StatusCode, String)> {
    let req: ChatRequest =
        serde_json::from_slice(&body).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    if wants_stream(&req.rest) {
        chat_completions_stream(state, headers, req).await
    } else {
        chat_completions_json(state, headers, req).await
    }
}

async fn chat_completions_json(
    state: AppState,
    headers: HeaderMap,
    req: ChatRequest,
) -> Result<Response, (StatusCode, String)> {
    let (api_key, access_token) = extract_auth(&headers);
    let (resp_body, status) = forward_chat(&state, &api_key, &access_token, &req, false).await?;
    if !status.is_success() {
        return Err((
            StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
            resp_body.to_string(),
        ));
    }
    Ok(Json(resp_body).into_response())
}

async fn chat_completions_stream(
    state: AppState,
    headers: HeaderMap,
    req: ChatRequest,
) -> Result<Response, (StatusCode, String)> {
    let (api_key, access_token) = extract_auth(&headers);
    let (upstream_resp, upstream_account_id, model_id, api_key_owned, access_token_owned) =
        open_upstream_stream(&state, &api_key, &access_token, &req).await?;

    let status = upstream_resp.status();
    if !status.is_success() {
        let text = upstream_resp.text().await.unwrap_or_default();
        return Err((
            StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
            text,
        ));
    }

    let state_clone = state.clone();
    let byte_stream = upstream_resp
        .bytes_stream()
        .map(|chunk| chunk.map_err(|e| std::io::Error::other(e.to_string())));
    let (tee_tx, mut tee_rx) = tokio::sync::mpsc::unbounded_channel::<bytes::Bytes>();
    let tracked = byte_stream.map(move |item| {
        if let Ok(ref b) = item {
            let _ = tee_tx.send(b.clone());
        }
        item
    });

    tokio::spawn(async move {
        let mut buf = String::new();
        let mut prompt_tokens = 0i64;
        let mut completion_tokens = 0i64;
        while let Some(chunk) = tee_rx.recv().await {
            if let Ok(text) = std::str::from_utf8(&chunk) {
                buf.push_str(text);
                ingest_sse_usage(&mut buf, &mut prompt_tokens, &mut completion_tokens);
            }
        }
        ingest_sse_usage(&mut buf, &mut prompt_tokens, &mut completion_tokens);
        apply_usage_line(&buf, &mut prompt_tokens, &mut completion_tokens);
        if prompt_tokens > 0 || completion_tokens > 0 {
            report_usage_in_process(
                &state_clone,
                api_key_owned.as_deref(),
                access_token_owned.as_deref(),
                &model_id,
                prompt_tokens,
                completion_tokens,
                upstream_account_id.as_deref(),
            )
            .await;
        }
    });

    let status_code = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    Response::builder()
        .status(status_code)
        .header("content-type", "text/event-stream")
        .header("cache-control", "no-cache")
        .header("connection", "keep-alive")
        .body(Body::from_stream(tracked))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn open_upstream_stream(
    state: &AppState,
    api_key: &Option<String>,
    access_token: &Option<String>,
    req: &ChatRequest,
) -> Result<
    (
        reqwest::Response,
        Option<String>,
        String,
        Option<String>,
        Option<String>,
    ),
    (StatusCode, String),
> {
    let mut exclude: Vec<String> = Vec::new();
    for attempt in 0..3 {
        let auth = authorize_hosted(
            state,
            GatewayAuthBody {
                api_key: api_key.clone(),
                access_token: access_token.clone(),
                model_id: req.model.clone(),
                exclude_account_ids: exclude.clone(),
            },
        )
        .await?;
        let resolved_model_id = auth["resolved_model_id"]
            .as_str()
            .unwrap_or(&req.model)
            .to_string();
        let upstream_model = auth["upstream_model"]
            .as_str()
            .unwrap_or(&req.model)
            .to_string();
        let upstream_account_id = auth["upstream_account_id"].as_str().map(|s| s.to_string());
        let (upstream_key, upstream_url) = resolve_upstream(&auth)?;

        let mut forward = req.rest.as_object().cloned().unwrap_or_default();
        forward.insert("model".into(), upstream_model.into());
        forward.insert("stream".into(), true.into());
        forward.insert(
            "stream_options".into(),
            serde_json::json!({ "include_usage": true }),
        );

        let upstream_resp = http_client()
            .post(&upstream_url)
            .header("Content-Type", "application/json")
            .bearer_auth(&upstream_key)
            .json(&Value::Object(forward))
            .send()
            .await
            .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;

        let status = upstream_resp.status();
        if should_retry_upstream(status.as_u16()) {
            if let Some(ref account_id) = upstream_account_id {
                let _ = crate::upstream_pool::record_upstream_failure(
                    &state.db,
                    account_id,
                    Some(status.as_u16() as i32),
                    &format!("upstream status {status}"),
                )
                .await;
                exclude.push(account_id.clone());
            }
            if attempt + 1 < 3 {
                continue;
            }
        }
        return Ok((
            upstream_resp,
            upstream_account_id,
            resolved_model_id,
            api_key.clone(),
            access_token.clone(),
        ));
    }
    Err((
        StatusCode::SERVICE_UNAVAILABLE,
        "all upstream accounts exhausted".into(),
    ))
}

async fn forward_chat(
    state: &AppState,
    api_key: &Option<String>,
    access_token: &Option<String>,
    req: &ChatRequest,
    force_stream: bool,
) -> Result<(Value, reqwest::StatusCode), (StatusCode, String)> {
    let mut exclude: Vec<String> = Vec::new();
    for attempt in 0..3 {
        let auth = authorize_hosted(
            state,
            GatewayAuthBody {
                api_key: api_key.clone(),
                access_token: access_token.clone(),
                model_id: req.model.clone(),
                exclude_account_ids: exclude.clone(),
            },
        )
        .await?;
        let resolved_model_id = auth["resolved_model_id"]
            .as_str()
            .unwrap_or(&req.model)
            .to_string();
        let upstream_model = auth["upstream_model"]
            .as_str()
            .unwrap_or(&req.model)
            .to_string();
        let upstream_account_id = auth["upstream_account_id"].as_str().map(|s| s.to_string());
        let (upstream_key, upstream_url) = resolve_upstream(&auth)?;

        let mut forward = req.rest.as_object().cloned().unwrap_or_default();
        forward.insert("model".into(), upstream_model.into());
        if force_stream {
            forward.insert("stream".into(), true.into());
        }

        let upstream_resp = http_client()
            .post(&upstream_url)
            .bearer_auth(&upstream_key)
            .json(&Value::Object(forward))
            .send()
            .await
            .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;

        let status = upstream_resp.status();
        let resp_body: Value = upstream_resp
            .json()
            .await
            .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;

        if should_retry_upstream(status.as_u16()) {
            if let Some(ref account_id) = upstream_account_id {
                let _ = crate::upstream_pool::record_upstream_failure(
                    &state.db,
                    account_id,
                    Some(status.as_u16() as i32),
                    &resp_body.to_string(),
                )
                .await;
                exclude.push(account_id.clone());
            }
            if attempt + 1 < 3 {
                continue;
            }
        }

        let usage = resp_body.get("usage");
        let prompt_tokens = usage
            .and_then(|u| u.get("prompt_tokens"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let completion_tokens = usage
            .and_then(|u| u.get("completion_tokens"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        report_usage_in_process(
            state,
            api_key.as_deref(),
            access_token.as_deref(),
            &resolved_model_id,
            prompt_tokens,
            completion_tokens,
            upstream_account_id.as_deref(),
        )
        .await;
        return Ok((resp_body, status));
    }
    Err((
        StatusCode::SERVICE_UNAVAILABLE,
        "all upstream accounts exhausted".into(),
    ))
}

fn resolve_upstream(auth: &Value) -> Result<(String, String), (StatusCode, String)> {
    let key = auth["upstream_api_key"].as_str().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "no upstream key".to_string(),
        )
    })?;
    let url = auth["upstream_base_url"].as_str().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "no upstream url".to_string(),
        )
    })?;
    Ok((
        key.to_string(),
        crate::upstream_pool::normalize_chat_completions_url(url),
    ))
}

fn should_retry_upstream(status: u16) -> bool {
    status == 429 || status >= 500
}

fn ingest_sse_usage(buf: &mut String, prompt_tokens: &mut i64, completion_tokens: &mut i64) {
    while let Some(nl) = buf.find('\n') {
        let line: String = buf.drain(..=nl).collect();
        apply_usage_line(&line, prompt_tokens, completion_tokens);
    }
}

fn apply_usage_line(line: &str, prompt_tokens: &mut i64, completion_tokens: &mut i64) {
    let Some(data) = line.trim().strip_prefix("data:") else {
        return;
    };
    let data = data.trim();
    if data.is_empty() || data == "[DONE]" {
        return;
    }
    let Ok(val) = serde_json::from_str::<Value>(data) else {
        return;
    };
    let Some(u) = val.get("usage") else {
        return;
    };
    if let Some(n) = u.get("prompt_tokens").and_then(|v| v.as_i64()) {
        *prompt_tokens = n;
    }
    if let Some(n) = u.get("completion_tokens").and_then(|v| v.as_i64()) {
        *completion_tokens = n;
    }
}

async fn report_usage_in_process(
    state: &AppState,
    api_key: Option<&str>,
    access_token: Option<&str>,
    model_id: &str,
    prompt_tokens: i64,
    completion_tokens: i64,
    upstream_account_id: Option<&str>,
) {
    let org_id = if let Some(key) = api_key {
        crate::usage::resolve_org_by_api_key(&state.db, key)
            .await
            .ok()
            .flatten()
    } else if let Some(tok) = access_token {
        crate::usage::resolve_org_by_session(&state.db, tok)
            .await
            .ok()
            .flatten()
    } else {
        None
    };
    let Some(org_id) = org_id else {
        tracing::warn!("hosted usage dropped: no org for model {model_id}");
        return;
    };
    if let Err(e) = crate::usage::record_usage(
        &state.db,
        &org_id,
        model_id,
        prompt_tokens,
        completion_tokens,
        upstream_account_id,
    )
    .await
    {
        tracing::error!(
            org_id,
            model_id,
            prompt_tokens,
            completion_tokens,
            "hosted usage record failed: {e}"
        );
    }
    if let Some(account_id) = upstream_account_id {
        let _ = crate::upstream_pool::record_upstream_success(
            &state.db,
            account_id,
            prompt_tokens,
            completion_tokens,
        )
        .await;
    }
}

pub async fn list_models(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    match list_models_inner(&state, &headers).await {
        Ok(v) => Json(v).into_response(),
        Err((status, msg)) => json_error(status, &msg).into_response(),
    }
}

async fn list_models_inner(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Value, (StatusCode, String)> {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .ok_or((StatusCode::UNAUTHORIZED, "bearer required".into()))?;
    let org_id = if token.starts_with("ackey_") {
        crate::usage::resolve_org_by_api_key(&state.db, token)
            .await
            .ok()
            .flatten()
    } else {
        crate::usage::resolve_org_by_session(&state.db, token)
            .await
            .ok()
            .flatten()
    };
    let Some(org_id) = org_id else {
        return Err((StatusCode::UNAUTHORIZED, "invalid credentials".into()));
    };
    let sub = crate::store::get_subscription(&state.db, &org_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let models = crate::models_catalog::list_models(&state.db, &sub.plan)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let data: Vec<Value> = models
        .iter()
        .filter(|m| m.available)
        .map(|m| {
            serde_json::json!({
                "id": m.id,
                "object": "model",
                "owned_by": m.provider_id,
            })
        })
        .collect();
    Ok(serde_json::json!({ "object": "list", "data": data }))
}

#[cfg(test)]
mod tests {
    use super::{apply_usage_line, ingest_sse_usage, should_retry_upstream};

    #[test]
    fn retry_on_rate_limit_and_server_errors() {
        assert!(should_retry_upstream(429));
        assert!(should_retry_upstream(500));
        assert!(!should_retry_upstream(400));
        assert!(!should_retry_upstream(200));
    }

    #[test]
    fn sse_usage_reads_final_chunk_without_trailing_newline() {
        let mut prompt = 0i64;
        let mut completion = 0i64;
        let mut buf = String::from(
            "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\
             data: {\"choices\":[],\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":4}}",
        );
        ingest_sse_usage(&mut buf, &mut prompt, &mut completion);
        apply_usage_line(&buf, &mut prompt, &mut completion);
        assert_eq!(prompt, 12);
        assert_eq!(completion, 4);
    }
}
