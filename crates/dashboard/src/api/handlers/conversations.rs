use super::*;

pub async fn start_project_conversation(
    State(state): State<AppState>,
    Path(project_id): Path<String>,
    Json(body): Json<crate::schema::StartConversationRequest>,
) -> impl IntoResponse {
    if !crate::task_trigger::triggers_allowed(&state.host) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({
                "error": "UI trigger run is disabled for this binding. Use loopback or set ANYCODE_DASHBOARD_TRIGGER_RUN_REMOTE=1."
            })),
        )
            .into_response();
    }

    let mut trigger_req = crate::task_trigger::TriggerRunRequest {
        prompt: body.prompt.clone(),
        kind: body.kind.clone(),
        goal: body.goal.clone(),
        agent: body.agent.clone(),
        skills: body.skills.clone(),
    };
    crate::task_trigger::normalize_trigger_request(&mut trigger_req);
    if let Err(e) = crate::task_trigger::validate_request(&trigger_req) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response();
    }

    let project = match state.db.get_project(&project_id).await {
        Ok(Some(p)) => p,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "project not found" })),
            )
                .into_response()
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response()
        }
    };

    let prompt = body.prompt.trim();
    let prompt_for_chat = crate::task_trigger::prompt_with_skills(prompt, body.skills.as_deref());
    let title = body
        .title
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| truncate_field(s, 120))
        .unwrap_or_else(|| truncate_field(prompt, 120));
    let prompt_preview = truncate_field(prompt, 240);
    // Video attachment (P1.2): bake frames + transcript appendix before either
    // dispatch path. Title/preview above intentionally use the raw prompt.
    let mut vision_images = body.vision_images.clone();
    let prompt_video;
    let prompt_for_chat_video;
    let (prompt, prompt_for_chat) = if let Some(vref) = body
        .video_ref
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        match crate::control::video_frames::resolve_video_attachment(vref) {
            Ok(outcome) => {
                if !outcome.frames.is_empty() {
                    vision_images
                        .get_or_insert_with(Vec::new)
                        .extend(outcome.frames);
                }
                prompt_video = match &outcome.appendix {
                    Some(ap) => format!("{prompt}\n\n{ap}"),
                    None => prompt.to_string(),
                };
                prompt_for_chat_video =
                    crate::task_trigger::prompt_with_skills(&prompt_video, body.skills.as_deref());
                (prompt_video.as_str(), prompt_for_chat_video.as_str())
            }
            Err(e) => {
                return (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response();
            }
        }
    } else {
        (prompt, prompt_for_chat.as_str())
    };
    let agent_type = body
        .agent
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let resolved_agent =
        crate::control::agent_resolve::resolve_web_chat_agent(agent_type.as_deref());
    // Slash-mode agent override (`/目标` etc.): this task only. The session row
    // keeps its default routing; only the dispatched turn runs under the override.
    let agent_ephemeral = body.agent_ephemeral.unwrap_or(false) && agent_type.is_some();
    let session_agent_type = if agent_ephemeral {
        None
    } else {
        agent_type.clone()
    };

    let root_path = std::path::PathBuf::from(&project.root_path);
    if let Err(e) = crate::task_trigger::validate_trigger_skills_for_project(
        body.skills.as_deref(),
        body.agent.as_deref(),
        &root_path,
    )
    .await
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response();
    }

    let (root, _created_root) = match super::chat_util::ensure_chat_project_root(
        &state.db,
        &project_id,
        None,
        &root_path,
        "conversation_start",
    )
    .await
    {
        Ok(v) => v,
        Err(e) => {
            return (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response();
        }
    };

    if body.recycle_session {
        if let Ok(Some(recycled)) = state
            .db
            .find_recyclable_web_chat_session(&project_id, session_agent_type.as_deref())
            .await
        {
            let session_id = recycled.id.clone();
            if let Err(e) = state
                .db
                .reopen_session_for_chat(
                    &session_id,
                    Some(title.as_str()),
                    Some(prompt_preview.as_str()),
                )
                .await
            {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": e.to_string() })),
                )
                    .into_response();
            }
            let session_agent = recycled.agent_type.trim();
            if agent_type.as_deref().is_some() && agent_type.as_deref() != Some(session_agent) {
                if agent_ephemeral {
                    // 仅本次任务: 不写库; runtime 按请求 agent 自重建.
                    state.web_chat.evict(&session_id).await;
                    state.chat_runtime.evict(&session_id).await;
                } else {
                    if let Err(e) = state
                        .db
                        .update_session_agent(&session_id, agent_type.as_deref())
                        .await
                    {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({ "error": e.to_string() })),
                        )
                            .into_response();
                    }
                    state.web_chat.evict(&session_id).await;
                    state.chat_runtime.evict(&session_id).await;
                }
            }
            match crate::control::web_chat_dispatch::dispatch_web_chat_prompt(
                &state,
                &project_id,
                &session_id,
                &root,
                Some(resolved_agent.as_str()),
                prompt,
                &prompt_for_chat,
                vision_images.as_deref(),
                body.text_files.as_deref(),
                body.lang.as_deref(),
                true,
                "conversation_recycled",
                body.composer_mode.as_deref(),
            )
            .await
            {
                Ok((session, chat)) => {
                    return Json(json!({
                        "session": session,
                        "chat": chat,
                        "recycled": true,
                    }))
                    .into_response();
                }
                Err((status, error)) => {
                    return (
                        status,
                        Json(json!({ "error": error, "session_id": session_id })),
                    )
                        .into_response();
                }
            }
        }
    }

    let kind = "repl";
    let session = match state
        .db
        .create_planned_session(CreateSessionRequest {
            project_id: project_id.clone(),
            kind: kind.to_string(),
            task_id: None,
            title: title.clone(),
            prompt_preview: Some(prompt_preview.clone()),
            agent_type: session_agent_type.clone(),
            model: None,
            metadata_json: Some(r#"{"source":"conversations_start"}"#.to_string()),
        })
        .await
    {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    match crate::control::web_chat_dispatch::dispatch_web_chat_prompt(
        &state,
        &project_id,
        &session.id,
        &root,
        Some(resolved_agent.as_str()),
        prompt,
        &prompt_for_chat,
        vision_images.as_deref(),
        body.text_files.as_deref(),
        body.lang.as_deref(),
        false,
        "conversation_started",
        body.composer_mode.as_deref(),
    )
    .await
    {
        Ok((session, chat)) => {
            Json(json!({ "session": session, "chat": chat, "recycled": false })).into_response()
        }
        Err((status, error)) => (
            status,
            Json(json!({ "error": error, "session_id": session.id })),
        )
            .into_response(),
    }
}

fn truncate_field(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect()
}
