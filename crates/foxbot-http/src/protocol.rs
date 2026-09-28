use crate::*;
use foxbot_core::{InputEvent, ReplyOutcome};
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
struct BusinessRequest<'a> {
    schema_version: &'static str,
    request_id: &'a str,
    idempotency_key: &'a str,
    conversation_ref: &'a str,
    session_revision: u64,
    provider_profile_version: u64,
    context_mode: &'a ContextMode,
    input_events: &'a [InputEvent],
    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<&'a [InputEvent]>,
    context_complete: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_request: Option<&'a str>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BusinessResponse {
    schema_version: String,
    request_id: String,
    conversation_ref: String,
    in_reply_to: Vec<String>,
    complete: bool,
    outcome: ReplyOutcome,
}

#[derive(Deserialize)]
pub(crate) struct FeedbackAck {
    pub schema_version: String,
    pub receipt_id: String,
    pub revision: u32,
    pub accepted: bool,
}

pub(crate) fn encode(config: &HttpConfig, request: &ReplyRequest, scope: &str) -> Result<Vec<u8>> {
    let bytes = match config.protocol {
        Protocol::BusinessV1 => serde_json::to_vec(&BusinessRequest {
            schema_version: "0.1", request_id: &request.request_id, idempotency_key: &request.request_id,
            conversation_ref: scope, session_revision: request.session_revision,
            provider_profile_version: request.provider_profile_version, context_mode: &config.context_mode,
            input_events: &request.input_events,
            context: (config.context_mode == ContextMode::ClientManaged).then_some(request.context.as_slice()),
            context_complete: request.context_complete, user_request: request.user_request.as_deref(),
        }),
        Protocol::ChatCompletions => {
            // Preserve sender/type/completeness metadata as user data. Never promote
            // a message's content or claimed role into a system/developer instruction.
            let data = serde_json::json!({"context": request.context, "input_events": request.input_events,
                "context_complete": request.context_complete, "user_request": request.user_request});
            let mut messages = Vec::new();
            if let Some(prompt) = &request.system_prompt {
                messages.push(serde_json::json!({"role":"system", "content":prompt}));
            }
            messages.push(serde_json::json!({"role":"user", "content":data.to_string()}));
            serde_json::to_vec(&serde_json::json!({"model": config.model,
                "messages":messages, "stream":false, "n":1}))
        }
    }.map_err(|_| HttpError::InvalidResponse)?;
    if bytes.len() > 1_048_576 {
        return Err(HttpError::BodyTooLarge);
    }
    Ok(bytes)
}

pub(crate) fn decode(
    config: &HttpConfig,
    request: &ReplyRequest,
    scope: &str,
    body: &[u8],
) -> Result<ReplyResponse> {
    match config.protocol {
        Protocol::BusinessV1 => {
            let response: BusinessResponse =
                serde_json::from_slice(body).map_err(|_| HttpError::InvalidResponse)?;
            let expected: Vec<_> = request
                .input_events
                .iter()
                .map(|e| e.event_id.as_str())
                .collect();
            if response.schema_version != "0.1"
                || response.request_id != request.request_id
                || response.conversation_ref != scope
                || response.in_reply_to != expected
                || !response.complete
            {
                return Err(HttpError::InvalidResponse);
            }
            Ok(ReplyResponse::for_request(request, response.outcome))
        }
        Protocol::ChatCompletions => {
            let value: serde_json::Value =
                serde_json::from_slice(body).map_err(|_| HttpError::InvalidResponse)?;
            let choices = value
                .get("choices")
                .and_then(|v| v.as_array())
                .ok_or(HttpError::InvalidResponse)?;
            if choices.len() != 1
                || choices[0].get("index").and_then(|v| v.as_u64()) != Some(0)
                || choices[0].get("finish_reason").and_then(|v| v.as_str()) != Some("stop")
            {
                return Err(HttpError::InvalidResponse);
            }
            let message = &choices[0]["message"];
            if message["role"] != "assistant"
                || !message["refusal"].is_null()
                || !message["function_call"].is_null()
                || (!message["tool_calls"].is_null()
                    && message["tool_calls"]
                        .as_array()
                        .is_none_or(|v| !v.is_empty()))
            {
                return Err(HttpError::InvalidResponse);
            }
            let text = message["content"]
                .as_str()
                .ok_or(HttpError::InvalidResponse)?
                .to_owned();
            Ok(ReplyResponse::for_request(
                request,
                ReplyOutcome::Reply { text },
            ))
        }
    }
}
