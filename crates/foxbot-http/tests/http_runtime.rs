mod support;
use foxbot_core::{simulation::MockChannel, *};
use foxbot_http::*;
use serde_json::json;
use support::*;

#[tokio::test]
async fn business_http_roundtrip_has_no_injected_prompt_and_sends_only_after_acceptance() {
    let server = Server::start().await;
    let service = HttpReplyService::new(
        server.config(Protocol::BusinessV1, ContextMode::ClientManaged),
        Some("synthetic-token"),
    )
    .unwrap();
    let (_dir, mut runtime, binding) = setup(&service);
    incoming(&mut runtime, &binding.key, "one", 100);
    let request = generate(&service, &mut runtime, &binding.key, 100)
        .await
        .unwrap();
    let action = runtime.prepare_send(&request, 102, false).unwrap();
    let mut channel = MockChannel::new(&binding.key);
    assert_eq!(
        runtime.dispatch(&action, 103, &mut channel).unwrap(),
        ActionState::VerifiedOutgoing
    );
    assert!(feedback(&service, &mut runtime, 104).await.unwrap());
    let state = server.state.lock().unwrap();
    assert_eq!(channel.send_calls, 1);
    assert_eq!(state.committed_turns, 1);
    assert_eq!(state.seen.len(), 2);
    assert_eq!(
        state.seen[0].headers["authorization"],
        "Bearer synthetic-token"
    );
    assert!(state.seen[0].body.get("system_prompt").is_none());
    assert!(state.seen[0].body.get("model").is_none());
    assert!(state.seen[0].body.get("context").is_some());
    assert_eq!(state.seen[1].body["disposition"], "observed_outgoing");
    assert!(state.seen[1].body.get("text").is_none());
}

#[tokio::test]
async fn chat_completions_preserves_metadata_without_promoting_message_roles() {
    let server = Server::start().await;
    let service = HttpReplyService::new(
        server.config(Protocol::ChatCompletions, ContextMode::ClientManaged),
        None,
    )
    .unwrap();
    let (_dir, mut runtime, binding) = setup(&service);
    incoming(&mut runtime, &binding.key, "one", 100);
    let id = generate(&service, &mut runtime, &binding.key, 100)
        .await
        .unwrap();
    assert_eq!(runtime.task_state(&id).unwrap(), "READY");
    let state = server.state.lock().unwrap();
    let body = &state.seen[0].body;
    assert_eq!(body["model"], "synthetic-model");
    assert_eq!(body["stream"], false);
    assert_eq!(body["messages"][0]["role"], "system");
    assert_eq!(body["messages"][1]["role"], "user");
    let data: serde_json::Value =
        serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
    assert_eq!(data["input_events"][0]["message"]["direction"], "incoming");
    assert!(data["input_events"][0]["message"]["sender"].is_string());
    assert!(!state.seen[0].headers.contains_key("idempotency-key"));
}

#[tokio::test]
async fn custom_knowledge_service_over_chat_completions_has_no_system_message() {
    let server = Server::start().await;
    let service = HttpReplyService::new(
        server.config(Protocol::ChatCompletions, ContextMode::ClientManaged),
        None,
    )
    .unwrap();
    let (_dir, mut runtime, mut binding) = setup(&service);
    binding.provider = ProviderProfile::Custom;
    runtime.bind(&binding).unwrap();
    incoming(&mut runtime, &binding.key, "one", 100);
    let id = generate(&service, &mut runtime, &binding.key, 100)
        .await
        .unwrap();
    assert_eq!(runtime.task_state(&id).unwrap(), "READY");
    let state = server.state.lock().unwrap();
    let messages = state.seen[0].body["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["role"], "user");
    assert!(state.seen[0].body.get("system_prompt").is_none());
}

#[tokio::test]
async fn service_managed_is_incremental_and_waits_for_final_feedback() {
    let server = Server::start().await;
    let service = HttpReplyService::new(
        server.config(Protocol::BusinessV1, ContextMode::ServiceManaged),
        None,
    )
    .unwrap();
    let (_dir, mut runtime, binding) = setup(&service);
    incoming(&mut runtime, &binding.key, "one", 100);
    let request = generate(&service, &mut runtime, &binding.key, 100)
        .await
        .unwrap();
    let action = runtime.prepare_send(&request, 102, false).unwrap();
    runtime
        .dispatch(&action, 103, &mut MockChannel::new(&binding.key))
        .unwrap();
    incoming(&mut runtime, &binding.key, "two", 104);
    assert!(
        service
            .begin(&mut runtime, &binding.key, 105, None)
            .unwrap()
            .is_none()
    );
    feedback(&service, &mut runtime, 106).await.unwrap();
    generate(&service, &mut runtime, &binding.key, 108)
        .await
        .unwrap();
    let state = server.state.lock().unwrap();
    let requests: Vec<_> = state
        .seen
        .iter()
        .filter(|r| r.path == "/generate")
        .collect();
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|r| r.body.get("context").is_none()));
    assert_eq!(
        requests[0].body["conversation_ref"],
        requests[1].body["conversation_ref"]
    );
    assert_eq!(
        requests[1].body["input_events"].as_array().unwrap().len(),
        1
    );
    assert_ne!(
        requests[0].body["input_events"][0]["event_id"],
        requests[1].body["input_events"][0]["event_id"]
    );
}

#[tokio::test]
async fn business_no_reply_and_handoff_are_not_outbound_messages() {
    for (outcome, expected) in [
        (json!({"result":"no_reply"}), "NO_REPLY"),
        (json!({"result":"handoff","reason":"合成交接"}), "HANDOFF"),
    ] {
        let server = Server::start().await;
        server.state.lock().unwrap().outcome = outcome;
        let service = HttpReplyService::new(
            server.config(Protocol::BusinessV1, ContextMode::ServiceManaged),
            None,
        )
        .unwrap();
        let (_dir, mut runtime, binding) = setup(&service);
        incoming(&mut runtime, &binding.key, "one", 100);
        let id = generate(&service, &mut runtime, &binding.key, 100)
            .await
            .unwrap();
        assert_eq!(runtime.task_state(&id).unwrap(), expected);
        assert!(runtime.summary().unwrap().actions.is_empty());
        feedback(&service, &mut runtime, 102).await.unwrap();
        assert_eq!(runtime.service_queue_summary().unwrap().acked, 1);
    }
}

#[tokio::test]
async fn dropped_response_retries_identical_bytes_and_one_idempotent_generation() {
    let server = Server::start().await;
    server.state.lock().unwrap().drop_generate_response = 1;
    let service = HttpReplyService::new(
        server.config(Protocol::BusinessV1, ContextMode::ServiceManaged),
        None,
    )
    .unwrap();
    let (_dir, mut runtime, binding) = setup(&service);
    incoming(&mut runtime, &binding.key, "one", 100);
    generate(&service, &mut runtime, &binding.key, 100)
        .await
        .unwrap();
    let state = server.state.lock().unwrap();
    assert_eq!(state.seen.len(), 2);
    assert_eq!(state.generations.len(), 1);
    assert_eq!(state.seen[0].raw, state.seen[1].raw);
    assert_eq!(
        state.seen[0].headers["idempotency-key"],
        state.seen[1].headers["idempotency-key"]
    );
}

#[tokio::test]
async fn non_idempotent_interface_never_replays_after_a_lost_response() {
    let server = Server::start().await;
    server.state.lock().unwrap().drop_generate_response = 1;
    let mut config = server.config(Protocol::BusinessV1, ContextMode::ClientManaged);
    config.idempotency_supported = false;
    config.staging_contract = false;
    config.receipt_endpoint = None;
    config.max_attempts = 1;
    let service = HttpReplyService::new(config, None).unwrap();
    let (_dir, mut runtime, binding) = setup(&service);
    incoming(&mut runtime, &binding.key, "one", 100);
    assert!(matches!(
        generate(&service, &mut runtime, &binding.key, 100).await,
        Err(HttpError::Transport)
    ));
    assert_eq!(server.state.lock().unwrap().seen.len(), 1);
    assert!(runtime.summary().unwrap().actions.is_empty());
}

#[tokio::test]
async fn retries_have_a_hard_attempt_limit() {
    let server = Server::start().await;
    server.state.lock().unwrap().generate_status = 503;
    let mut config = server.config(Protocol::BusinessV1, ContextMode::ServiceManaged);
    config.max_attempts = 3;
    let service = HttpReplyService::new(config, None).unwrap();
    let (_dir, mut runtime, binding) = setup(&service);
    incoming(&mut runtime, &binding.key, "one", 100);
    assert!(matches!(
        generate(&service, &mut runtime, &binding.key, 100).await,
        Err(HttpError::Status { code: 503, .. })
    ));
    assert_eq!(server.state.lock().unwrap().seen.len(), 3);
    assert_eq!(runtime.service_queue_summary().unwrap().pending, 1);
}

#[tokio::test]
async fn retry_after_is_not_shortened_to_fit_the_local_deadline() {
    let server = Server::start().await;
    {
        let mut state = server.state.lock().unwrap();
        state.generate_status = 429;
        state.retry_after = Some("60".into());
    }
    let mut config = server.config(Protocol::BusinessV1, ContextMode::ServiceManaged);
    config.attempt_timeout_ms = 100;
    config.total_timeout_ms = 200;
    let service = HttpReplyService::new(config, None).unwrap();
    let (_dir, mut runtime, binding) = setup(&service);
    incoming(&mut runtime, &binding.key, "one", 100);
    assert!(matches!(
        generate(&service, &mut runtime, &binding.key, 100).await,
        Err(HttpError::Timeout)
    ));
    assert_eq!(server.state.lock().unwrap().seen.len(), 1);
}

#[tokio::test]
async fn authentication_errors_are_not_retried_and_do_not_echo_secrets_or_bodies() {
    let server = Server::start().await;
    {
        let mut state = server.state.lock().unwrap();
        state.generate_status = 401;
        state.response_override = Some(b"synthetic-secret-and-body".to_vec());
    }
    let service = HttpReplyService::new(
        server.config(Protocol::BusinessV1, ContextMode::ServiceManaged),
        Some("synthetic-secret"),
    )
    .unwrap();
    let (_dir, mut runtime, binding) = setup(&service);
    incoming(&mut runtime, &binding.key, "one", 100);
    let error = generate(&service, &mut runtime, &binding.key, 100)
        .await
        .unwrap_err();
    assert!(matches!(error, HttpError::Status { code: 401, .. }));
    assert!(!format!("{error:?} {error}").contains("synthetic-secret"));
    assert!(!format!("{error:?} {error}").contains(&server.url));
    assert_eq!(server.state.lock().unwrap().seen.len(), 1);
}

#[tokio::test]
async fn precancelled_request_never_opens_a_connection() {
    let server = Server::start().await;
    let service = HttpReplyService::new(
        server.config(Protocol::BusinessV1, ContextMode::ServiceManaged),
        None,
    )
    .unwrap();
    let (_dir, mut runtime, binding) = setup(&service);
    incoming(&mut runtime, &binding.key, "one", 100);
    let job = service
        .begin(&mut runtime, &binding.key, 100, None)
        .unwrap()
        .unwrap();
    let cancel = CancellationToken::new();
    cancel.cancel();
    let result = service.run(job, cancel).await;
    assert!(matches!(
        service.finish(&mut runtime, result, 101),
        Err(HttpError::Cancelled)
    ));
    assert!(server.state.lock().unwrap().seen.is_empty());
    assert_eq!(runtime.service_queue_summary().unwrap().pending, 1);
}

#[tokio::test]
async fn cancellation_in_flight_leaves_a_durable_server_tombstone_not_a_chat_send() {
    let server = Server::start().await;
    server.state.lock().unwrap().generate_delay_ms = 300;
    let service = HttpReplyService::new(
        server.config(Protocol::BusinessV1, ContextMode::ServiceManaged),
        None,
    )
    .unwrap();
    let (_dir, mut runtime, binding) = setup(&service);
    incoming(&mut runtime, &binding.key, "one", 100);
    let job = service
        .begin(&mut runtime, &binding.key, 100, None)
        .unwrap()
        .unwrap();
    let id = job.request_id().to_owned();
    let token = CancellationToken::new();
    let worker_service = service.clone();
    let worker_token = token.clone();
    let worker = tokio::spawn(async move { worker_service.run(job, worker_token).await });
    server.wait_requests(1).await;
    token.cancel();
    assert!(matches!(
        service.finish(&mut runtime, worker.await.unwrap(), 101),
        Err(HttpError::Cancelled)
    ));
    assert_eq!(runtime.task_state(&id).unwrap(), "ERROR");
    feedback(&service, &mut runtime, 102).await.unwrap();
    let state = server.state.lock().unwrap();
    assert_eq!(state.receipts[&id]["disposition"], "cancelled");
    assert_eq!(state.committed_turns, 0);
    assert!(runtime.summary().unwrap().actions.is_empty());
}

#[tokio::test]
async fn cancellation_after_http_success_still_prevents_acceptance() {
    let server = Server::start().await;
    let service = HttpReplyService::new(
        server.config(Protocol::BusinessV1, ContextMode::ServiceManaged),
        None,
    )
    .unwrap();
    let (_dir, mut runtime, binding) = setup(&service);
    incoming(&mut runtime, &binding.key, "one", 100);
    let job = service
        .begin(&mut runtime, &binding.key, 100, None)
        .unwrap()
        .unwrap();
    let token = CancellationToken::new();
    let completion = service.run(job, token.clone()).await;
    token.cancel();
    assert!(matches!(
        service.finish(&mut runtime, completion, 101),
        Err(HttpError::Cancelled)
    ));
    assert!(runtime.summary().unwrap().actions.is_empty());
}

#[tokio::test]
async fn runtime_remains_available_for_new_input_while_http_runs_and_old_result_is_stale() {
    let server = Server::start().await;
    server.state.lock().unwrap().generate_delay_ms = 100;
    let service = HttpReplyService::new(
        server.config(Protocol::BusinessV1, ContextMode::ServiceManaged),
        None,
    )
    .unwrap();
    let (_dir, mut runtime, binding) = setup(&service);
    incoming(&mut runtime, &binding.key, "one", 100);
    let job = service
        .begin(&mut runtime, &binding.key, 100, None)
        .unwrap()
        .unwrap();
    let svc = service.clone();
    let worker = tokio::spawn(async move { svc.run(job, CancellationToken::new()).await });
    server.wait_requests(1).await;
    incoming(&mut runtime, &binding.key, "two", 101);
    assert!(matches!(
        service.finish(&mut runtime, worker.await.unwrap(), 102),
        Err(HttpError::Stale)
    ));
    assert!(runtime.summary().unwrap().actions.is_empty());
    assert_eq!(runtime.service_queue_summary().unwrap().pending, 1);
}

#[tokio::test]
async fn completed_http_response_is_checked_against_fresh_task_time() {
    let server = Server::start().await;
    let service = HttpReplyService::new(
        server.config(Protocol::BusinessV1, ContextMode::ServiceManaged),
        None,
    )
    .unwrap();
    let (_dir, mut runtime, mut binding) = setup(&service);
    binding.reply_ttl_ms = 5;
    runtime.bind(&binding).unwrap();
    incoming(&mut runtime, &binding.key, "one", 100);
    let job = service
        .begin(&mut runtime, &binding.key, 100, None)
        .unwrap()
        .unwrap();
    let completion = service.run(job, CancellationToken::new()).await;
    assert!(matches!(
        service.finish(&mut runtime, completion, 106),
        Err(HttpError::Stale)
    ));
}

#[tokio::test]
async fn both_content_length_and_chunked_body_are_bounded() {
    for chunked in [false, true] {
        let server = Server::start().await;
        {
            let mut state = server.state.lock().unwrap();
            state.response_override = Some(vec![b'x'; 1024]);
            state.chunked = chunked;
        }
        let mut config = server.config(Protocol::BusinessV1, ContextMode::ClientManaged);
        config.max_response_bytes = 128;
        let service = HttpReplyService::new(config, None).unwrap();
        let (_dir, mut runtime, binding) = setup(&service);
        incoming(&mut runtime, &binding.key, "one", 100);
        assert!(matches!(
            generate(&service, &mut runtime, &binding.key, 100).await,
            Err(HttpError::BodyTooLarge)
        ));
        assert_eq!(server.state.lock().unwrap().seen.len(), 1);
    }
}

#[tokio::test]
async fn malformed_json_and_unexpected_sse_are_never_repaired_or_sent() {
    for (content, ctype) in [
        (b"{\"partial\":".to_vec(), "application/json"),
        (
            b"data: {\"content\":\"partial\"}\n\n".to_vec(),
            "text/event-stream",
        ),
    ] {
        let server = Server::start().await;
        {
            let mut state = server.state.lock().unwrap();
            state.response_override = Some(content);
            state.content_type = ctype.into();
        }
        let service = HttpReplyService::new(
            server.config(Protocol::BusinessV1, ContextMode::ClientManaged),
            None,
        )
        .unwrap();
        let (_dir, mut runtime, binding) = setup(&service);
        incoming(&mut runtime, &binding.key, "one", 100);
        assert!(matches!(
            generate(&service, &mut runtime, &binding.key, 100).await,
            Err(HttpError::InvalidResponse)
        ));
        assert!(runtime.summary().unwrap().actions.is_empty());
        assert_eq!(server.state.lock().unwrap().seen.len(), 1);
    }
}

#[tokio::test]
async fn chat_length_tool_calls_refusal_and_missing_stop_do_not_become_replies() {
    for (reason, message) in [
        (
            json!("length"),
            json!({"role":"assistant","content":"partial"}),
        ),
        (
            json!("tool_calls"),
            json!({"role":"assistant","content":"tool","tool_calls":[{}]}),
        ),
        (
            json!("stop"),
            json!({"role":"assistant","content":"text","refusal":"refused"}),
        ),
        (
            json!(null),
            json!({"role":"assistant","content":"missing stop"}),
        ),
    ] {
        let server = Server::start().await;
        server.state.lock().unwrap().response_override = Some(
            serde_json::to_vec(
                &json!({"choices":[{"index":0,"finish_reason":reason,"message":message}]}),
            )
            .unwrap(),
        );
        let service = HttpReplyService::new(
            server.config(Protocol::ChatCompletions, ContextMode::ClientManaged),
            None,
        )
        .unwrap();
        let (_dir, mut runtime, binding) = setup(&service);
        incoming(&mut runtime, &binding.key, "one", 100);
        assert!(matches!(
            generate(&service, &mut runtime, &binding.key, 100).await,
            Err(HttpError::InvalidResponse)
        ));
    }
}

#[tokio::test]
async fn business_misbound_response_cannot_change_routing() {
    let server = Server::start().await;
    server.state.lock().unwrap().response_override = Some(
        serde_json::to_vec(&json!({"schema_version":"0.1",
        "request_id":"wrong","conversation_ref":"wrong","in_reply_to":[],"complete":true,
        "outcome":{"result":"reply","text":"text"},"target":"another-contact"}))
        .unwrap(),
    );
    let service = HttpReplyService::new(
        server.config(Protocol::BusinessV1, ContextMode::ClientManaged),
        None,
    )
    .unwrap();
    let (_dir, mut runtime, binding) = setup(&service);
    incoming(&mut runtime, &binding.key, "one", 100);
    assert!(matches!(
        generate(&service, &mut runtime, &binding.key, 100).await,
        Err(HttpError::InvalidResponse)
    ));
    assert!(runtime.summary().unwrap().actions.is_empty());
}

#[tokio::test]
async fn feedback_failure_survives_restart_and_does_not_repeat_chat_send() {
    let server = Server::start().await;
    let service = HttpReplyService::new(
        server.config(Protocol::BusinessV1, ContextMode::ServiceManaged),
        None,
    )
    .unwrap();
    let (dir, mut runtime, binding) = setup(&service);
    incoming(&mut runtime, &binding.key, "one", 100);
    let request = generate(&service, &mut runtime, &binding.key, 100)
        .await
        .unwrap();
    let action = runtime.prepare_send(&request, 102, false).unwrap();
    let mut channel = MockChannel::new(&binding.key);
    runtime.dispatch(&action, 103, &mut channel).unwrap();
    server.state.lock().unwrap().receipt_status = 503;
    assert!(matches!(
        feedback(&service, &mut runtime, 104).await,
        Err(HttpError::Status { code: 503, .. })
    ));
    drop(runtime);
    let mut runtime = Runtime::open_simulation(dir.path()).unwrap();
    assert!(service.begin_feedback(&mut runtime, 105).unwrap().is_none());
    server.state.lock().unwrap().receipt_status = 200;
    assert!(feedback(&service, &mut runtime, 1000).await.unwrap());
    assert_eq!(
        runtime.action(&action).unwrap().1,
        ActionState::VerifiedOutgoing
    );
    assert!(runtime.dispatch(&action, 1001, &mut channel).is_err());
    assert_eq!(channel.send_calls, 1);
    assert_eq!(runtime.service_queue_summary().unwrap().acked, 1);
}

#[tokio::test]
async fn lost_feedback_ack_is_replayed_idempotently_without_committing_twice() {
    let server = Server::start().await;
    server.state.lock().unwrap().drop_receipt_response = 1;
    let service = HttpReplyService::new(
        server.config(Protocol::BusinessV1, ContextMode::ServiceManaged),
        None,
    )
    .unwrap();
    let (dir, mut runtime, binding) = setup(&service);
    incoming(&mut runtime, &binding.key, "one", 100);
    let request = generate(&service, &mut runtime, &binding.key, 100)
        .await
        .unwrap();
    let action = runtime.prepare_send(&request, 102, false).unwrap();
    let mut channel = MockChannel::new(&binding.key);
    runtime.dispatch(&action, 103, &mut channel).unwrap();
    assert!(matches!(
        feedback(&service, &mut runtime, 104).await,
        Err(HttpError::Transport)
    ));
    assert_eq!(server.state.lock().unwrap().committed_turns, 1);
    drop(runtime);
    let mut runtime = Runtime::open_simulation(dir.path()).unwrap();
    feedback(&service, &mut runtime, 1000).await.unwrap();
    let state = server.state.lock().unwrap();
    assert_eq!(state.committed_turns, 1);
    assert_eq!(channel.send_calls, 1);
    let receipts: Vec<_> = state
        .seen
        .iter()
        .filter(|r| r.path == "/feedback")
        .collect();
    assert_eq!(receipts.len(), 2);
    assert_eq!(receipts[0].raw, receipts[1].raw);
    assert_eq!(
        receipts[0].headers["idempotency-key"],
        receipts[1].headers["idempotency-key"]
    );
}

struct UnknownChannel(MockChannel);
impl MessageChannel for UnknownChannel {
    fn inspect(&mut self, key: &ConversationKey) -> foxbot_core::Result<LiveTarget> {
        self.0.inspect(key)
    }
    fn fill(&mut self, a: &OutboundAction, t: &LiveTarget) -> foxbot_core::Result<()> {
        self.0.fill(a, t)
    }
    fn send(&mut self, a: &OutboundAction, t: &LiveTarget) -> foxbot_core::Result<SendEvidence> {
        self.0.send(a, t)?;
        Ok(SendEvidence::Unknown)
    }
    fn reconcile(&mut self, a: &OutboundAction) -> foxbot_core::Result<SendEvidence> {
        self.0.reconcile(a)
    }
}

#[tokio::test]
async fn unknown_feedback_does_not_commit_history_then_verified_revision_does() {
    let server = Server::start().await;
    let service = HttpReplyService::new(
        server.config(Protocol::BusinessV1, ContextMode::ServiceManaged),
        None,
    )
    .unwrap();
    let (_dir, mut runtime, binding) = setup(&service);
    incoming(&mut runtime, &binding.key, "one", 100);
    let request = generate(&service, &mut runtime, &binding.key, 100)
        .await
        .unwrap();
    let action = runtime.prepare_send(&request, 102, false).unwrap();
    let mut channel = UnknownChannel(MockChannel::new(&binding.key));
    assert_eq!(
        runtime.dispatch(&action, 103, &mut channel).unwrap(),
        ActionState::Unknown
    );
    feedback(&service, &mut runtime, 104).await.unwrap();
    assert_eq!(server.state.lock().unwrap().committed_turns, 0);
    assert!(!runtime.service_ready(&binding.key).unwrap());
    runtime.reconcile(&action, &mut channel).unwrap();
    feedback(&service, &mut runtime, 106).await.unwrap();
    assert_eq!(server.state.lock().unwrap().committed_turns, 1);
    assert_eq!(
        server.state.lock().unwrap().receipts[&request]["revision"],
        2
    );
    assert_eq!(channel.0.send_calls, 1);
    assert!(runtime.service_ready(&binding.key).unwrap());
}

#[tokio::test]
async fn crash_gap_after_generation_reconstructs_cancellation_feedback() {
    let server = Server::start().await;
    let service = HttpReplyService::new(
        server.config(Protocol::BusinessV1, ContextMode::ServiceManaged),
        None,
    )
    .unwrap();
    let (dir, mut runtime, binding) = setup(&service);
    incoming(&mut runtime, &binding.key, "one", 100);
    let job = service
        .begin(&mut runtime, &binding.key, 100, None)
        .unwrap()
        .unwrap();
    let id = job.request_id().to_owned();
    drop(service.run(job, CancellationToken::new()).await); // HTTP succeeded but was never accepted locally.
    drop(runtime);
    let mut runtime = Runtime::open_simulation(dir.path()).unwrap();
    assert_eq!(runtime.task_state(&id).unwrap(), "ERROR");
    feedback(&service, &mut runtime, 102).await.unwrap();
    assert_eq!(
        server.state.lock().unwrap().receipts[&id]["disposition"],
        "cancelled"
    );
    assert!(runtime.summary().unwrap().actions.is_empty());
    assert_eq!(server.state.lock().unwrap().committed_turns, 0);
}

#[tokio::test]
async fn feedback_claim_recovers_after_ack_was_received_but_not_recorded() {
    let server = Server::start().await;
    server.state.lock().unwrap().outcome = json!({"result":"no_reply"});
    let service = HttpReplyService::new(
        server.config(Protocol::BusinessV1, ContextMode::ServiceManaged),
        None,
    )
    .unwrap();
    let (dir, mut runtime, binding) = setup(&service);
    incoming(&mut runtime, &binding.key, "one", 100);
    generate(&service, &mut runtime, &binding.key, 100)
        .await
        .unwrap();
    let claim = service.begin_feedback(&mut runtime, 102).unwrap().unwrap();
    drop(service.run_feedback(claim, CancellationToken::new()).await);
    assert_eq!(runtime.service_queue_summary().unwrap().in_flight, 1);
    drop(runtime);
    let mut runtime = Runtime::open_simulation(dir.path()).unwrap();
    feedback(&service, &mut runtime, 103).await.unwrap();
    assert_eq!(runtime.service_queue_summary().unwrap().acked, 1);
    assert_eq!(server.state.lock().unwrap().committed_turns, 1);
}

#[tokio::test]
async fn wrong_profile_cannot_take_or_transmit_another_profiles_receipt() {
    let server = Server::start().await;
    server.state.lock().unwrap().outcome = json!({"result":"no_reply"});
    let config = server.config(Protocol::BusinessV1, ContextMode::ServiceManaged);
    let service = HttpReplyService::new(config.clone(), Some("synthetic-first")).unwrap();
    let other = HttpReplyService::new(config, Some("synthetic-second")).unwrap();
    let (_dir, mut runtime, binding) = setup(&service);
    incoming(&mut runtime, &binding.key, "one", 100);
    generate(&service, &mut runtime, &binding.key, 100)
        .await
        .unwrap();
    assert!(other.begin_feedback(&mut runtime, 102).unwrap().is_none());
    let claim = service.begin_feedback(&mut runtime, 102).unwrap().unwrap();
    let completion = other
        .run_feedback(claim.clone(), CancellationToken::new())
        .await;
    assert!(matches!(
        other.finish_feedback(&mut runtime, completion, 103),
        Err(HttpError::Stale)
    ));
    assert_eq!(server.state.lock().unwrap().seen.len(), 1);
    let completion = service.run_feedback(claim, CancellationToken::new()).await;
    service
        .finish_feedback(&mut runtime, completion, 104)
        .unwrap();
}

#[tokio::test]
async fn wrong_ack_is_suspended_instead_of_falsely_advancing_history() {
    let server = Server::start().await;
    {
        let mut state = server.state.lock().unwrap();
        state.outcome = json!({"result":"no_reply"});
        state.ack_wrong = true;
    }
    let service = HttpReplyService::new(
        server.config(Protocol::BusinessV1, ContextMode::ServiceManaged),
        None,
    )
    .unwrap();
    let (_dir, mut runtime, binding) = setup(&service);
    incoming(&mut runtime, &binding.key, "one", 100);
    generate(&service, &mut runtime, &binding.key, 100)
        .await
        .unwrap();
    assert!(matches!(
        feedback(&service, &mut runtime, 102).await,
        Err(HttpError::InvalidResponse)
    ));
    assert_eq!(runtime.service_queue_summary().unwrap().suspended, 1);
    assert!(!runtime.service_ready(&binding.key).unwrap());
}

#[tokio::test]
async fn feedback_retry_budget_persists_and_exhaustion_blocks_stateful_progress() {
    let server = Server::start().await;
    {
        let mut state = server.state.lock().unwrap();
        state.outcome = json!({"result":"no_reply"});
        state.receipt_status = 503;
    }
    let service = HttpReplyService::new(
        server.config(Protocol::BusinessV1, ContextMode::ServiceManaged),
        None,
    )
    .unwrap();
    let (dir, mut runtime, binding) = setup(&service);
    incoming(&mut runtime, &binding.key, "one", 100);
    generate(&service, &mut runtime, &binding.key, 100)
        .await
        .unwrap();
    for attempt in 1..=MAX_RECEIPT_ATTEMPTS {
        assert!(
            feedback(&service, &mut runtime, u64::from(attempt) * 100_000)
                .await
                .is_err()
        );
        drop(runtime);
        runtime = Runtime::open_simulation(dir.path()).unwrap();
    }
    assert!(
        service
            .begin_feedback(&mut runtime, 9_000_000)
            .unwrap()
            .is_none()
    );
    assert_eq!(runtime.service_queue_summary().unwrap().suspended, 1);
    assert!(!runtime.service_ready(&binding.key).unwrap());
}

#[tokio::test]
async fn a_waiting_request_can_be_cancelled_without_exceeding_concurrency() {
    let server = Server::start().await;
    server.state.lock().unwrap().generate_delay_ms = 200;
    let mut config = server.config(Protocol::ChatCompletions, ContextMode::ClientManaged);
    config.max_in_flight = 1;
    let service = HttpReplyService::new(config, None).unwrap();
    let (_dir, mut runtime, mut binding) = setup(&service);
    incoming(&mut runtime, &binding.key, "one", 100);
    let first = service
        .begin(&mut runtime, &binding.key, 100, None)
        .unwrap()
        .unwrap();
    binding.key.conversation = "second-conversation".into();
    runtime.bind(&binding).unwrap();
    incoming(&mut runtime, &binding.key, "two", 100);
    let second = service
        .begin(&mut runtime, &binding.key, 100, None)
        .unwrap()
        .unwrap();
    let svc = service.clone();
    let worker = tokio::spawn(async move { svc.run(first, CancellationToken::new()).await });
    server.wait_requests(1).await;
    let token = CancellationToken::new();
    let cancel = token.clone();
    let waiting = service.run(second, token);
    tokio::pin!(waiting);
    tokio::select! { _=&mut waiting => panic!("second request bypassed queue"), _=tokio::time::sleep(std::time::Duration::from_millis(20))=>{} }
    cancel.cancel();
    let completion = waiting.await;
    assert!(matches!(
        service.finish(&mut runtime, completion, 101),
        Err(HttpError::Cancelled)
    ));
    service
        .finish(&mut runtime, worker.await.unwrap(), 102)
        .unwrap();
    assert_eq!(server.state.lock().unwrap().seen.len(), 1);
}

#[tokio::test]
async fn v1_ledger_upgrades_additively_without_losing_messages() {
    let server = Server::start().await;
    let service = HttpReplyService::new(
        server.config(Protocol::ChatCompletions, ContextMode::ClientManaged),
        None,
    )
    .unwrap();
    let (dir, mut runtime, binding) = setup(&service);
    incoming(&mut runtime, &binding.key, "one", 100);
    drop(runtime);
    let conn = rusqlite::Connection::open(dir.path().join("ledger.sqlite3")).unwrap();
    conn.execute_batch(
        "DROP TABLE service_receipts; DROP TABLE service_exchanges; PRAGMA user_version=1;",
    )
    .unwrap();
    drop(conn);
    let runtime = Runtime::open_simulation(dir.path()).unwrap();
    assert_eq!(runtime.summary().unwrap().messages, 1);
    assert_eq!(runtime.service_queue_summary().unwrap().pending, 0);
    drop(runtime);
    let conn = rusqlite::Connection::open(dir.path().join("ledger.sqlite3")).unwrap();
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[tokio::test]
async fn redirects_never_forward_authorization_or_payload_to_another_endpoint() {
    let server = Server::start().await;
    let other = Server::start().await;
    {
        let mut s = server.state.lock().unwrap();
        s.generate_status = 307;
        s.redirect_to = Some(format!("{}/generate", other.url));
    }
    let service = HttpReplyService::new(
        server.config(Protocol::BusinessV1, ContextMode::ServiceManaged),
        Some("synthetic-no-leak"),
    )
    .unwrap();
    let (_dir, mut runtime, binding) = setup(&service);
    incoming(&mut runtime, &binding.key, "one", 100);
    assert!(matches!(
        generate(&service, &mut runtime, &binding.key, 100).await,
        Err(HttpError::Status { code: 307, .. })
    ));
    assert!(other.state.lock().unwrap().seen.is_empty());
    assert_eq!(server.state.lock().unwrap().seen.len(), 1);
}

#[tokio::test]
async fn per_attempt_timeout_does_not_retry_a_non_idempotent_provider() {
    let server = Server::start().await;
    server.state.lock().unwrap().generate_delay_ms = 500;
    let mut config = server.config(Protocol::ChatCompletions, ContextMode::ClientManaged);
    config.attempt_timeout_ms = 50;
    config.total_timeout_ms = 1000;
    let service = HttpReplyService::new(config, None).unwrap();
    let (_dir, mut runtime, binding) = setup(&service);
    incoming(&mut runtime, &binding.key, "one", 100);
    assert!(matches!(
        generate(&service, &mut runtime, &binding.key, 100).await,
        Err(HttpError::Timeout)
    ));
    assert_eq!(server.state.lock().unwrap().seen.len(), 1);
}

#[tokio::test]
async fn cancellation_tombstone_wins_even_when_generate_arrives_later() {
    let server = Server::start().await;
    let service = HttpReplyService::new(
        server.config(Protocol::BusinessV1, ContextMode::ServiceManaged),
        None,
    )
    .unwrap();
    let (_dir, mut runtime, binding) = setup(&service);
    incoming(&mut runtime, &binding.key, "one", 100);
    let job = service
        .begin(&mut runtime, &binding.key, 100, None)
        .unwrap()
        .unwrap();
    service.cancel(&mut runtime, job.request_id()).unwrap();
    feedback(&service, &mut runtime, 101).await.unwrap();
    let completion = service.run(job, CancellationToken::new()).await;
    assert!(matches!(
        service.finish(&mut runtime, completion, 102),
        Err(HttpError::Stale)
    ));
    assert!(server.state.lock().unwrap().generations.is_empty());
    assert_eq!(server.state.lock().unwrap().committed_turns, 0);
}

#[tokio::test]
async fn late_unknown_feedback_cannot_regress_the_final_outgoing_evidence() {
    let server = Server::start().await;
    let service = HttpReplyService::new(
        server.config(Protocol::BusinessV1, ContextMode::ServiceManaged),
        None,
    )
    .unwrap();
    let (_dir, mut runtime, binding) = setup(&service);
    incoming(&mut runtime, &binding.key, "one", 100);
    let request = generate(&service, &mut runtime, &binding.key, 100)
        .await
        .unwrap();
    let action = runtime.prepare_send(&request, 102, false).unwrap();
    let mut channel = UnknownChannel(MockChannel::new(&binding.key));
    runtime.dispatch(&action, 103, &mut channel).unwrap();
    let earlier = service.begin_feedback(&mut runtime, 104).unwrap().unwrap();
    runtime.reconcile(&action, &mut channel).unwrap();
    feedback(&service, &mut runtime, 105).await.unwrap();
    let late = service
        .run_feedback(earlier, CancellationToken::new())
        .await;
    service.finish_feedback(&mut runtime, late, 106).unwrap();
    let s = server.state.lock().unwrap();
    assert_eq!(s.receipts[&request]["disposition"], "observed_outgoing");
    assert_eq!(s.receipts[&request]["revision"], 2);
    assert_eq!(s.committed_turns, 1);
    assert_eq!(channel.0.send_calls, 1);
}

#[tokio::test]
async fn operational_timeout_tuning_preserves_receipt_identity() {
    let server = Server::start().await;
    server.state.lock().unwrap().outcome = json!({"result":"no_reply"});
    let config = server.config(Protocol::BusinessV1, ContextMode::ServiceManaged);
    let service = HttpReplyService::new(config.clone(), None).unwrap();
    let (_dir, mut runtime, binding) = setup(&service);
    incoming(&mut runtime, &binding.key, "one", 100);
    generate(&service, &mut runtime, &binding.key, 100)
        .await
        .unwrap();
    let mut tuned = config;
    tuned.attempt_timeout_ms = 2000;
    tuned.total_timeout_ms = 4000;
    tuned.max_in_flight = 1;
    let tuned = HttpReplyService::new(tuned, None).unwrap();
    assert_eq!(service.profile_tag(), tuned.profile_tag());
    assert!(feedback(&tuned, &mut runtime, 102).await.unwrap());
    assert!(runtime.service_ready(&binding.key).unwrap());
}

#[tokio::test]
async fn invalid_configurations_fail_before_network_or_secret_serialization() {
    let server = Server::start().await;
    for endpoint in [
        "http://example.com/api",
        "http://localhost/api",
        "https://user:pass@example.com/api",
        "https://example.com/api?key=secret",
        "https://example.com/api#secret",
        "file:///tmp/example",
    ] {
        let mut config = server.config(Protocol::ChatCompletions, ContextMode::ClientManaged);
        config.endpoint = endpoint.into();
        assert!(matches!(
            HttpReplyService::new(config, None),
            Err(HttpError::Config)
        ));
    }
    let mut config = server.config(Protocol::BusinessV1, ContextMode::ServiceManaged);
    config.staging_contract = false;
    assert!(HttpReplyService::new(config, None).is_err());
    let mut config = server.config(Protocol::ChatCompletions, ContextMode::ClientManaged);
    config.max_attempts = 2;
    assert!(HttpReplyService::new(config, None).is_err());
    let mut config = server.config(Protocol::BusinessV1, ContextMode::ServiceManaged);
    config.receipt_endpoint = Some("https://other.invalid/feedback".into());
    assert!(HttpReplyService::new(config, None).is_err());
    assert!(server.state.lock().unwrap().seen.is_empty());
}
