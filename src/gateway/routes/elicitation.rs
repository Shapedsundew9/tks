//! Elicitation REST & SSE route handlers (D-130..D-134, D-EA-6..D-EA-17, §6.2).

use std::convert::Infallible;
use std::time::Duration;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::http::{HeaderValue, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use serde_json::Value;
use uuid::Uuid;

use crate::elicitation::facilitator::{
    FacilitatorContext, FacilitatorProvider, OllamaFacilitatorProvider,
};
use crate::elicitation::session::SessionState;
use crate::elicitation::store::{
    AddTurnRequest, CreateSessionRequest, ElicitationError, ElicitationEvent, PatchTurnRequest,
};
use crate::elicitation::transcript::render_transcript;
use crate::gateway::AppState;

/// GET `/api/v1/elicitation/capabilities`
pub async fn get_capabilities(State(state): State<AppState>) -> impl IntoResponse {
    let cfg = &state.elicitation.config;
    let manifest = crate::elicitation::AsrLockManifest::load_default();

    let asr_available = if let Some(ref dir) = cfg.asr_assets_dir {
        manifest.is_dir_complete(dir)
    } else {
        false
    };

    let facilitator_json = match cfg.facilitator_provider {
        crate::elicitation::FacilitatorProviderKind::None => {
            serde_json::json!({ "provider": "none" })
        }
        crate::elicitation::FacilitatorProviderKind::Ollama => {
            serde_json::json!({
                "provider": "ollama",
                "model": cfg.facilitator_model
            })
        }
    };

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "asr": {
                "available": asr_available,
                "engine": "sherpa-onnx",
                "version": manifest.version,
                "assets_base": "/explorer/asr"
            },
            "facilitator": facilitator_json
        })),
    )
}

/// POST `/api/v1/elicitation/sessions`
pub async fn create_session(
    State(state): State<AppState>,
    Json(payload): Json<CreateSessionRequest>,
) -> Result<Response, (StatusCode, Json<Value>)> {
    match state
        .elicitation
        .create_session(payload, Some(&state.pool))
        .await
    {
        Ok(session) => Ok((StatusCode::CREATED, Json(session)).into_response()),
        Err(ElicitationError::CapacityExceeded(msg)) => Err((
            StatusCode::TOO_MANY_REQUESTS,
            Json(serde_json::json!({ "error": "ERR_CAPACITY_EXCEEDED", "message": msg })),
        )),
        Err(ElicitationError::Validation(msg)) => Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({ "error": "ERR_VALIDATION", "message": msg })),
        )),
        Err(ElicitationError::InvalidPriorTranscript(msg)) => Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({ "error": "ERR_INVALID_PRIOR_TRANSCRIPT", "message": msg })),
        )),
        Err(ElicitationError::NotFound(msg)) => Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "ERR_NOT_FOUND", "message": msg })),
        )),
        Err(ElicitationError::Database(msg)) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": "ERR_DATABASE", "message": msg })),
        )),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": "ERR_INTERNAL", "message": e.to_string() })),
        )),
    }
}

/// GET `/api/v1/elicitation/sessions/{elicitation_id}`
pub async fn get_session(
    State(state): State<AppState>,
    Path(elicitation_id): Path<Uuid>,
) -> Result<Response, (StatusCode, Json<Value>)> {
    match state.elicitation.get_session(elicitation_id).await {
        Ok(session) => Ok((StatusCode::OK, Json(session)).into_response()),
        Err(ElicitationError::NotFound(msg)) => Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "ERR_NOT_FOUND", "message": msg })),
        )),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": "ERR_INTERNAL", "message": e.to_string() })),
        )),
    }
}

/// POST `/api/v1/elicitation/sessions/{elicitation_id}/turns`
pub async fn add_turn(
    State(state): State<AppState>,
    Path(elicitation_id): Path<Uuid>,
    Json(payload): Json<AddTurnRequest>,
) -> Result<Response, (StatusCode, Json<Value>)> {
    match state.elicitation.add_turn(elicitation_id, payload).await {
        Ok(turn) => Ok((StatusCode::CREATED, Json(turn)).into_response()),
        Err(ElicitationError::SessionClosed) => Err((
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "ERR_SESSION_CLOSED",
                "message": "Cannot add turns to a closed session"
            })),
        )),
        Err(ElicitationError::CapacityExceeded(msg)) => Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({ "error": "ERR_CAPACITY_EXCEEDED", "message": msg })),
        )),
        Err(ElicitationError::Validation(msg)) => Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({ "error": "ERR_VALIDATION", "message": msg })),
        )),
        Err(ElicitationError::NotFound(msg)) => Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "ERR_NOT_FOUND", "message": msg })),
        )),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": "ERR_INTERNAL", "message": e.to_string() })),
        )),
    }
}

/// PATCH `/api/v1/elicitation/sessions/{elicitation_id}/turns/{number}`
pub async fn patch_turn(
    State(state): State<AppState>,
    Path((elicitation_id, number)): Path<(Uuid, u32)>,
    Json(payload): Json<PatchTurnRequest>,
) -> Result<Response, (StatusCode, Json<Value>)> {
    match state
        .elicitation
        .patch_turn(elicitation_id, number, payload)
        .await
    {
        Ok(turn) => Ok((StatusCode::OK, Json(turn)).into_response()),
        Err(ElicitationError::SessionClosed) => Err((
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "ERR_SESSION_CLOSED",
                "message": "Cannot modify a closed session"
            })),
        )),
        Err(ElicitationError::TurnNotFound(num)) => Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "ERR_TURN_NOT_FOUND",
                "message": format!("Turn t{num} not found")
            })),
        )),
        Err(ElicitationError::Validation(msg)) => Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({ "error": "ERR_VALIDATION", "message": msg })),
        )),
        Err(ElicitationError::NotFound(msg)) => Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "ERR_NOT_FOUND", "message": msg })),
        )),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": "ERR_INTERNAL", "message": e.to_string() })),
        )),
    }
}

/// DELETE `/api/v1/elicitation/sessions/{elicitation_id}/turns/{number}`
pub async fn delete_turn(
    State(state): State<AppState>,
    Path((elicitation_id, number)): Path<(Uuid, u32)>,
) -> Result<Response, (StatusCode, Json<Value>)> {
    match state.elicitation.delete_turn(elicitation_id, number).await {
        Ok(turns) => Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "deleted": number,
                "turns": turns
            })),
        )
            .into_response()),
        Err(ElicitationError::SessionClosed) => Err((
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "ERR_SESSION_CLOSED",
                "message": "Cannot modify a closed session"
            })),
        )),
        Err(ElicitationError::TurnNotFound(num)) => Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "ERR_TURN_NOT_FOUND",
                "message": format!("Turn t{num} not found")
            })),
        )),
        Err(ElicitationError::NotFound(msg)) => Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "ERR_NOT_FOUND", "message": msg })),
        )),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": "ERR_INTERNAL", "message": e.to_string() })),
        )),
    }
}

/// POST `/api/v1/elicitation/sessions/{elicitation_id}/facilitator/proposals`
pub async fn request_facilitator_proposal(
    State(state): State<AppState>,
    Path(elicitation_id): Path<Uuid>,
    body: axum::body::Bytes,
) -> Result<Response, (StatusCode, Json<Value>)> {
    let payload: serde_json::Value =
        serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);

    let client_excluded: Vec<String> = payload
        .get("excluded_ideas")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::trim))
                .filter(|s| !s.is_empty())
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default();

    if !client_excluded.is_empty() {
        state
            .elicitation
            .record_explored_items(elicitation_id, client_excluded)
            .await;
    }

    let session = state
        .elicitation
        .get_session(elicitation_id)
        .await
        .map_err(|e| {
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "ERR_NOT_FOUND", "message": e.to_string() })),
            )
        })?;

    if session.state == SessionState::Closed {
        return Err((
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "ERR_SESSION_CLOSED",
                "message": "Cannot request proposals on a closed session"
            })),
        ));
    }

    let cfg = &state.elicitation.config;
    if cfg.facilitator_provider == crate::elicitation::FacilitatorProviderKind::None {
        return Err((
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "ERR_FACILITATOR_DISABLED",
                "message": "Facilitator generation is not enabled"
            })),
        ));
    }

    let mode = payload
        .get("mode")
        .and_then(|m| m.as_str())
        .unwrap_or("socratic")
        .to_string();
    let is_inspiration = mode.eq_ignore_ascii_case("inspiration");
    let topic = payload
        .get("topic")
        .and_then(|t| t.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);

    let proposal_id = Uuid::new_v4();

    // Assemble context bounded by characters (dropping oldest turns first)
    let system_prompt = if is_inspiration {
        crate::elicitation::INSPIRATION_SYSTEM_PROMPT_V2.to_string()
    } else {
        crate::elicitation::FACILITATOR_SYSTEM_PROMPT_V2.to_string()
    };

    let gap_questions: Vec<String> = session
        .imported_gap_questions
        .iter()
        .map(|g| g.suggested_question.clone())
        .collect();

    let mut turns = Vec::new();
    let mut current_budget: usize = 16_000;
    for turn in session.turns.iter().rev() {
        let turn_len = turn.text.len();
        if turn_len > current_budget {
            break;
        }
        current_budget -= turn_len;
        turns.push((turn.role.to_string(), turn.text.clone()));
    }
    turns.reverse();

    let context = FacilitatorContext {
        system_prompt,
        session_title: session.title.clone(),
        is_inspiration,
        focus_topic: topic,
        gap_questions,
        explored_items: session.explored_items.clone(),
        turns,
    };

    let provider =
        OllamaFacilitatorProvider::new(cfg.ollama_url.clone(), cfg.facilitator_model.clone());
    let store = state.elicitation.clone();

    // Spawn background task so gateway 30s TimeoutLayer does not terminate generation
    tokio::spawn(async move {
        match provider.propose(context).await {
            Ok(proposal_text) => {
                let extracted =
                    crate::elicitation::extract_proposal_items(&proposal_text, is_inspiration);
                store.record_explored_items(elicitation_id, extracted).await;

                let flags = if is_inspiration {
                    Vec::new()
                } else {
                    crate::elicitation::lint_neutrality(&proposal_text)
                };
                store
                    .broadcast_event(
                        elicitation_id,
                        ElicitationEvent::FacilitatorProposal {
                            proposal_id,
                            mode,
                            text: proposal_text,
                            flags,
                        },
                    )
                    .await;
            }
            Err(err_msg) => {
                store
                    .broadcast_event(
                        elicitation_id,
                        ElicitationEvent::FacilitatorError { error: err_msg },
                    )
                    .await;
            }
        }
    });

    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({ "proposal_id": proposal_id })),
    )
        .into_response())
}

/// GET `/api/v1/elicitation/sessions/{elicitation_id}/events`
pub async fn stream_session_events(
    State(state): State<AppState>,
    Path(elicitation_id): Path<Uuid>,
) -> Result<
    Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>,
    (StatusCode, Json<Value>),
> {
    let rx = state
        .elicitation
        .subscribe_events(elicitation_id)
        .await
        .map_err(|e| {
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "ERR_NOT_FOUND", "message": e.to_string() })),
            )
        })?;

    let cancel = state.cancel_token.clone();
    let stream = futures_util::stream::unfold((rx, cancel), |(mut rx, cancel)| async move {
        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    return None;
                }
                msg = rx.recv() => {
                    match msg {
                        Ok(event) => {
                            let event_type = match event {
                                ElicitationEvent::TurnCommitted(_) => "turn_committed",
                                ElicitationEvent::TurnInserted { .. } => "turn_inserted",
                                ElicitationEvent::TurnUpdated(_) => "turn_updated",
                                ElicitationEvent::TurnDeleted { .. } => "turn_deleted",
                                ElicitationEvent::FacilitatorProposal { .. } => "facilitator_proposal",
                                ElicitationEvent::FacilitatorError { .. } => "facilitator_error",
                                ElicitationEvent::SessionClosed { .. } => "session_closed",
                            };
                            match serde_json::to_string(&event) {
                                Ok(json) => {
                                    let sse_event = Event::default().event(event_type).data(json);
                                    return Some((Ok::<_, Infallible>(sse_event), (rx, cancel)));
                                }
                                Err(e) => {
                                    tracing::warn!("Failed to serialize ElicitationEvent for SSE: {e}");
                                    continue;
                                }
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                            tracing::warn!("SSE subscriber lagged, skipped {skipped} messages");
                            continue;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            return None;
                        }
                    }
                }
            }
        }
    });

    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))))
}

/// POST `/api/v1/elicitation/sessions/{elicitation_id}/close`
pub async fn close_session(
    State(state): State<AppState>,
    Path(elicitation_id): Path<Uuid>,
) -> Result<Response, (StatusCode, Json<Value>)> {
    match state.elicitation.close_session(elicitation_id).await {
        Ok(()) => Ok((StatusCode::OK, Json(serde_json::json!({ "valid": true }))).into_response()),
        Err(ElicitationError::CloseValidation(errors)) => Ok((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({ "valid": false, "errors": errors })),
        )
            .into_response()),
        Err(ElicitationError::NotFound(msg)) => Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "ERR_NOT_FOUND", "message": msg })),
        )),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": "ERR_INTERNAL", "message": e.to_string() })),
        )),
    }
}

/// POST `/api/v1/elicitation/sessions/{elicitation_id}/reopen`
pub async fn reopen_session(
    State(state): State<AppState>,
    Path(elicitation_id): Path<Uuid>,
) -> Result<Response, (StatusCode, Json<Value>)> {
    match state.elicitation.reopen_session(elicitation_id).await {
        Ok(session) => Ok((StatusCode::OK, Json(session)).into_response()),
        Err(ElicitationError::NotFound(msg)) => Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "ERR_NOT_FOUND", "message": msg })),
        )),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": "ERR_INTERNAL", "message": e.to_string() })),
        )),
    }
}

/// GET `/api/v1/elicitation/sessions/{elicitation_id}/transcript`
pub async fn get_transcript(
    State(state): State<AppState>,
    Path(elicitation_id): Path<Uuid>,
) -> Result<Response, (StatusCode, Json<Value>)> {
    let session = state
        .elicitation
        .get_session(elicitation_id)
        .await
        .map_err(|e| {
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "ERR_NOT_FOUND", "message": e.to_string() })),
            )
        })?;

    if session.state != SessionState::Closed {
        return Err((
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "ERR_SESSION_OPEN",
                "message": "Transcript download is only available after session is closed"
            })),
        ));
    }

    let rendered = render_transcript(&session);
    let filename = format!("{}-c{}.md", session.session_id, session.cycle);
    let disposition = format!("attachment; filename=\"{filename}\"");

    let mut response = rendered.into_response();
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/markdown; charset=utf-8"),
    );
    if let Ok(val) = HeaderValue::from_str(&disposition) {
        response.headers_mut().insert(CONTENT_DISPOSITION, val);
    }

    Ok(response)
}

/// DELETE `/api/v1/elicitation/sessions/{elicitation_id}`
pub async fn delete_session(
    State(state): State<AppState>,
    Path(elicitation_id): Path<Uuid>,
) -> Result<Response, (StatusCode, Json<Value>)> {
    match state.elicitation.delete_session(elicitation_id).await {
        Ok(()) => Ok(StatusCode::NO_CONTENT.into_response()),
        Err(ElicitationError::NotFound(msg)) => Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "ERR_NOT_FOUND", "message": msg })),
        )),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": "ERR_INTERNAL", "message": e.to_string() })),
        )),
    }
}
