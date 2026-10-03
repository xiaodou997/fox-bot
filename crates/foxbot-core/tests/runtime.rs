use foxbot_core::{simulation::*, *};
use tempfile::TempDir;

fn private_dir() -> TempDir {
    let directory = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    directory
}

fn setup() -> (TempDir, Runtime, Binding) {
    let dir = private_dir();
    let mut runtime = Runtime::open_simulation(dir.path()).unwrap();
    let mut binding = Binding::paused(fixture_key());
    binding.enabled = true;
    binding.quiet_ms = 0;
    binding.max_wait_ms = 0;
    runtime.bind(&binding).unwrap();
    (dir, runtime, binding)
}

fn incoming(runtime: &mut Runtime, key: &ConversationKey, id: &str, now: u64) {
    runtime
        .ingest(&fixture_observation(key, id, "合成问题", now))
        .unwrap();
}

fn ready(runtime: &mut Runtime, key: &ConversationKey, now: u64) -> String {
    let mut provider = FixedReply::new("合成回答\n第二行");
    runtime
        .generate_once(key, now, None, &mut provider)
        .unwrap()
        .unwrap()
}

fn prepared(runtime: &mut Runtime, key: &ConversationKey) -> String {
    incoming(runtime, key, "new", 1);
    let request = ready(runtime, key, 1);
    runtime.prepare_send(&request, 1, false).unwrap()
}

#[test]
fn new_binding_is_paused() {
    let dir = private_dir();
    let mut runtime = Runtime::open_simulation(dir.path()).unwrap();
    let binding = Binding::paused(fixture_key());
    runtime.bind(&binding).unwrap();
    incoming(&mut runtime, &binding.key, "one", 1);
    assert!(
        runtime
            .begin_reply(&binding.key, 1_000, None)
            .unwrap()
            .is_none()
    );
}

#[test]
fn history_and_restart_never_trigger_backfill() {
    let (dir, mut runtime, binding) = setup();
    let mut old = fixture_observation(&binding.key, "old", "合成历史", 0);
    old.historical = true;
    assert_eq!(runtime.ingest(&old).unwrap(), IngestOutcome::Baseline);
    drop(runtime);
    let mut runtime = Runtime::open_simulation(dir.path()).unwrap();
    old.historical = false;
    assert_eq!(runtime.ingest(&old).unwrap(), IngestOutcome::Duplicate);
    assert!(
        runtime
            .begin_reply(&binding.key, 1_000, None)
            .unwrap()
            .is_none()
    );
    incoming(&mut runtime, &binding.key, "new", 1_001);
    let request = runtime
        .begin_reply(&binding.key, 1_001, None)
        .unwrap()
        .unwrap();
    assert_eq!(request.input_events.len(), 1);
    assert_eq!(request.context.len(), 2);
}

#[test]
fn same_event_across_sources_is_one_logical_message() {
    let (_dir, mut runtime, binding) = setup();
    let mut event = fixture_observation(&binding.key, "one", "合成消息", 1);
    assert_eq!(runtime.ingest(&event).unwrap(), IngestOutcome::Queued);
    assert_eq!(runtime.ingest(&event).unwrap(), IngestOutcome::Duplicate);
    for source in [Source::Notification, Source::Ocr, Source::UiTree] {
        event.source = source;
        assert_eq!(runtime.ingest(&event).unwrap(), IngestOutcome::Duplicate);
    }
    assert_eq!(runtime.summary().unwrap().messages, 1);
}

#[test]
fn repeated_text_is_not_a_global_identity() {
    let (_dir, mut runtime, binding) = setup();
    for id in ["one", "two"] {
        runtime
            .ingest(&fixture_observation(&binding.key, id, "好的", 1))
            .unwrap();
    }
    let request = runtime.begin_reply(&binding.key, 1, None).unwrap().unwrap();
    assert_eq!(request.input_events.len(), 2);
    assert_ne!(
        request.input_events[0].event_id,
        request.input_events[1].event_id
    );
}

#[test]
fn uncertain_identity_is_not_fabricated_from_text() {
    let (_dir, mut runtime, binding) = setup();
    let mut event = fixture_observation(&binding.key, "one", "相同文字", 1);
    event.message.canonical_id = None;
    assert_eq!(runtime.ingest(&event).unwrap(), IngestOutcome::Ambiguous);
    assert!(
        runtime
            .begin_reply(&binding.key, 1, None)
            .unwrap()
            .is_none()
    );
}

#[test]
fn conflicting_observation_invalidates_inflight_reply() {
    let (_dir, mut runtime, binding) = setup();
    let mut event = fixture_observation(&binding.key, "one", "型号 A1", 1);
    runtime.ingest(&event).unwrap();
    let request = runtime.begin_reply(&binding.key, 1, None).unwrap().unwrap();
    event.source = Source::Ocr;
    event.message.text = Some("型号 AI".into());
    assert_eq!(runtime.ingest(&event).unwrap(), IngestOutcome::Ambiguous);
    let response = ReplyResponse::for_request(
        &request,
        ReplyOutcome::Reply {
            text: "合成回复".into(),
        },
    );
    assert!(matches!(
        runtime.accept_reply(&request.request_id, &response, 1),
        Err(Error::Stale)
    ));
    assert!(
        runtime
            .begin_reply(&binding.key, 1, None)
            .unwrap()
            .is_none()
    );
}

#[test]
fn updated_notification_is_not_blindly_a_new_message() {
    let (_dir, mut runtime, binding) = setup();
    let mut event = fixture_observation(&binding.key, "notification-key", "初始摘要", 1);
    event.source = Source::Notification;
    runtime.ingest(&event).unwrap();
    event.message.text = Some("更新的摘要".into());
    assert_eq!(runtime.ingest(&event).unwrap(), IngestOutcome::Ambiguous);
    assert_eq!(runtime.summary().unwrap().messages, 1);
}

#[test]
fn same_titles_and_source_ids_are_isolated_by_account() {
    let (_dir, mut runtime, mut first) = setup();
    first.title = "同名联系人".into();
    runtime.bind(&first).unwrap();
    let mut second = first.clone();
    second.key.account_binding = "second-account".into();
    runtime.bind(&second).unwrap();
    incoming(&mut runtime, &first.key, "same-source-id", 1);
    incoming(&mut runtime, &second.key, "same-source-id", 1);
    let a = runtime.begin_reply(&first.key, 1, None).unwrap().unwrap();
    let b = runtime.begin_reply(&second.key, 1, None).unwrap().unwrap();
    assert_ne!(a.conversation_ref, b.conversation_ref);
    assert_ne!(a.input_events[0].event_id, b.input_events[0].event_id);
    assert_eq!(a.input_events.len(), 1);
    assert_eq!(b.input_events.len(), 1);
}

#[test]
fn new_message_invalidates_and_requeues_unsent_input() {
    let (_dir, mut runtime, binding) = setup();
    incoming(&mut runtime, &binding.key, "one", 1);
    let old = runtime.begin_reply(&binding.key, 1, None).unwrap().unwrap();
    incoming(&mut runtime, &binding.key, "two", 2);
    let old_response = ReplyResponse::for_request(&old, ReplyOutcome::NoReply);
    assert!(matches!(
        runtime.accept_reply(&old.request_id, &old_response, 2),
        Err(Error::Stale)
    ));
    let new = runtime.begin_reply(&binding.key, 2, None).unwrap().unwrap();
    assert_eq!(new.input_events.len(), 2);
}

#[test]
fn configuration_change_invalidates_without_replaying_old_input() {
    let (_dir, mut runtime, mut binding) = setup();
    let action = prepared(&mut runtime, &binding.key);
    binding.profile_version += 1;
    runtime.bind(&binding).unwrap();
    assert_eq!(runtime.action(&action).unwrap().1, ActionState::Stale);
    assert!(
        runtime
            .begin_reply(&binding.key, 2, None)
            .unwrap()
            .is_none()
    );
}

#[test]
fn maximum_wait_prevents_endless_debounce() {
    let (_dir, mut runtime, mut binding) = setup();
    binding.quiet_ms = 100;
    binding.max_wait_ms = 200;
    runtime.bind(&binding).unwrap();
    for (id, time) in [("one", 0), ("two", 90), ("three", 180)] {
        incoming(&mut runtime, &binding.key, id, time);
    }
    assert!(
        runtime
            .begin_reply(&binding.key, 179, None)
            .unwrap()
            .is_none()
    );
    assert!(
        runtime
            .begin_reply(&binding.key, 199, None)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        runtime
            .begin_reply(&binding.key, 200, None)
            .unwrap()
            .unwrap()
            .input_events
            .len(),
        3
    );
}

#[test]
fn backpressure_does_not_consume_retriable_observation() {
    let (_dir, mut runtime, mut binding) = setup();
    binding.max_pending = 1;
    binding.max_batch = 1;
    runtime.bind(&binding).unwrap();
    incoming(&mut runtime, &binding.key, "one", 1);
    let second = fixture_observation(&binding.key, "two", "合成第二条", 2);
    assert!(matches!(runtime.ingest(&second), Err(Error::Backpressure)));
    let request = runtime.begin_reply(&binding.key, 1, None).unwrap().unwrap();
    runtime
        .accept_reply(
            &request.request_id,
            &ReplyResponse::for_request(&request, ReplyOutcome::NoReply),
            1,
        )
        .unwrap();
    assert_eq!(runtime.ingest(&second).unwrap(), IngestOutcome::Queued);
}

#[test]
fn custom_service_has_no_injected_prompt_generic_is_explicit() {
    let (_dir, mut runtime, mut binding) = setup();
    incoming(&mut runtime, &binding.key, "one", 1);
    let a = runtime.begin_reply(&binding.key, 1, None).unwrap().unwrap();
    assert!(a.system_prompt.is_none());
    binding.provider = ProviderProfile::Generic {
        system_prompt: "用户配置的规则".into(),
    };
    binding.profile_version += 1;
    runtime.bind(&binding).unwrap();
    incoming(&mut runtime, &binding.key, "two", 2);
    let b = runtime
        .begin_reply(&binding.key, 2, Some("用户临时要求"))
        .unwrap()
        .unwrap();
    assert_eq!(b.system_prompt.as_deref(), Some("用户配置的规则"));
    assert_eq!(b.user_request.as_deref(), Some("用户临时要求"));
}

#[test]
fn no_reply_and_handoff_never_create_an_outbound_action() {
    for outcome in [
        ReplyOutcome::NoReply,
        ReplyOutcome::Handoff {
            reason: "合成交接".into(),
        },
    ] {
        let (_dir, mut runtime, binding) = setup();
        incoming(&mut runtime, &binding.key, "one", 1);
        let request = runtime.begin_reply(&binding.key, 1, None).unwrap().unwrap();
        runtime
            .accept_reply(
                &request.request_id,
                &ReplyResponse::for_request(&request, outcome.clone()),
                1,
            )
            .unwrap();
        assert!(runtime.prepare_send(&request.request_id, 1, false).is_err());
        assert!(runtime.summary().unwrap().actions.is_empty());
        if matches!(outcome, ReplyOutcome::Handoff { .. }) {
            incoming(&mut runtime, &binding.key, "two", 2);
            assert!(
                runtime
                    .begin_reply(&binding.key, 2, None)
                    .unwrap()
                    .is_none()
            );
        }
    }
}

#[test]
fn malformed_partial_misbound_and_oversize_replies_fail_closed() {
    for case in 0..6 {
        let (_dir, mut runtime, binding) = setup();
        incoming(&mut runtime, &binding.key, "one", 1);
        let request = runtime.begin_reply(&binding.key, 1, None).unwrap().unwrap();
        let mut response = ReplyResponse::for_request(
            &request,
            ReplyOutcome::Reply {
                text: "合成回复".into(),
            },
        );
        match case {
            0 => response.complete = false,
            1 => response.request_id = "wrong-request".into(),
            2 => response.in_reply_to = vec![],
            3 => response.outcome = ReplyOutcome::Reply { text: " ".into() },
            4 => {
                response.outcome = ReplyOutcome::Reply {
                    text: "中".repeat(4097),
                }
            }
            _ => {
                response.outcome = ReplyOutcome::Reply {
                    text: "bad\u{001b}control".into(),
                }
            }
        }
        assert!(
            runtime
                .accept_reply(&request.request_id, &response, 1)
                .is_err()
        );
        assert_eq!(runtime.task_state(&request.request_id).unwrap(), "ERROR");
        assert!(runtime.summary().unwrap().actions.is_empty());
    }
}

#[test]
fn json_cannot_add_routing_instructions() {
    let (_dir, mut runtime, binding) = setup();
    incoming(&mut runtime, &binding.key, "one", 1);
    let request = runtime.begin_reply(&binding.key, 1, None).unwrap().unwrap();
    let response = ReplyResponse::for_request(&request, ReplyOutcome::NoReply);
    let mut json = serde_json::to_value(response).unwrap();
    json["target"] = "different-contact".into();
    assert!(
        runtime
            .accept_json(&request.request_id, &json.to_string(), 1)
            .is_err()
    );
    assert_eq!(runtime.task_state(&request.request_id).unwrap(), "ERROR");
}

#[test]
fn interrupted_json_is_not_repaired_and_sent() {
    let (_dir, mut runtime, binding) = setup();
    incoming(&mut runtime, &binding.key, "one", 1);
    let request = runtime.begin_reply(&binding.key, 1, None).unwrap().unwrap();
    assert!(
        runtime
            .accept_json(&request.request_id, "{\"request_id\":", 1)
            .is_err()
    );
    assert!(runtime.summary().unwrap().actions.is_empty());
}

#[test]
fn expired_or_clock_rollback_results_are_stale() {
    for now in [0, 60_002] {
        let (_dir, mut runtime, binding) = setup();
        incoming(&mut runtime, &binding.key, "one", 1);
        let request = runtime.begin_reply(&binding.key, 1, None).unwrap().unwrap();
        assert!(matches!(
            runtime.accept_reply(
                &request.request_id,
                &ReplyResponse::for_request(&request, ReplyOutcome::NoReply),
                now
            ),
            Err(Error::Stale)
        ));
    }
}

#[test]
fn provider_failure_does_not_retry_or_invent_fallback() {
    struct Failing(usize);
    impl ReplyProvider for Failing {
        fn generate(&mut self, _: &ReplyRequest) -> Result<ReplyResponse> {
            self.0 += 1;
            Err(Error::Blocked("synthetic provider failure"))
        }
    }
    let (_dir, mut runtime, binding) = setup();
    incoming(&mut runtime, &binding.key, "one", 1);
    let mut provider = Failing(0);
    assert!(
        runtime
            .generate_once(&binding.key, 1, None, &mut provider)
            .is_err()
    );
    assert!(
        runtime
            .generate_once(&binding.key, 2, None, &mut provider)
            .unwrap()
            .is_none()
    );
    assert_eq!(provider.0, 1);
    assert!(runtime.summary().unwrap().actions.is_empty());
}

#[test]
fn suggestion_requires_approval_and_assisted_requires_request() {
    for mode in [Mode::AutoSuggest, Mode::Assisted] {
        let (_dir, mut runtime, mut binding) = setup();
        binding.mode = mode;
        runtime.bind(&binding).unwrap();
        incoming(&mut runtime, &binding.key, "one", 1);
        if mode == Mode::Assisted {
            assert!(
                runtime
                    .begin_reply(&binding.key, 1, None)
                    .unwrap()
                    .is_none()
            );
        }
        let request = runtime
            .begin_reply(&binding.key, 1, Some("合成人工请求"))
            .unwrap()
            .unwrap();
        runtime
            .accept_reply(
                &request.request_id,
                &ReplyResponse::for_request(
                    &request,
                    ReplyOutcome::Reply {
                        text: "合成回复".into(),
                    },
                ),
                1,
            )
            .unwrap();
        assert!(matches!(
            runtime.prepare_send(&request.request_id, 1, false),
            Err(Error::Blocked(_))
        ));
        let action = runtime.prepare_send(&request.request_id, 1, true).unwrap();
        assert_eq!(
            runtime
                .dispatch(&action, 1, &mut MockChannel::new(&binding.key))
                .unwrap(),
            ActionState::VerifiedOutgoing
        );
    }
}

#[test]
fn group_only_verified_mentions_trigger_and_sender_is_preserved() {
    let (_dir, mut runtime, mut binding) = setup();
    binding.kind = ConversationKind::Group;
    runtime.bind(&binding).unwrap();
    let mut event = fixture_observation(&binding.key, "plain-at", "@名字 合成文本", 1);
    assert_eq!(runtime.ingest(&event).unwrap(), IngestOutcome::Ignored);
    event.source_event_id = "real-mention".into();
    event.message.canonical_id = Some("real-mention".into());
    event.message.mention = Mention::Verified;
    event.message.sender = Some("group-sender-2".into());
    runtime.ingest(&event).unwrap();
    let request = runtime.begin_reply(&binding.key, 1, None).unwrap().unwrap();
    assert_eq!(request.input_events.len(), 1);
    assert_eq!(
        request.input_events[0].message.sender.as_deref(),
        Some("group-sender-2")
    );
}

#[test]
fn unparsed_content_is_preserved_in_provider_context() {
    let (_dir, mut runtime, binding) = setup();
    let mut image = fixture_observation(&binding.key, "image", "", 0);
    image.historical = true;
    image.message.kind = ContentKind::Image;
    image.message.text = None;
    image.message.complete = false;
    runtime.ingest(&image).unwrap();
    incoming(&mut runtime, &binding.key, "question", 1);
    let request = runtime.begin_reply(&binding.key, 1, None).unwrap().unwrap();
    assert!(!request.context_complete);
    assert_eq!(request.context[0].message.kind, ContentKind::Image);
    assert!(request.context[0].message.text.is_none());
}

#[test]
fn successful_send_is_observed_not_claimed_delivered_and_cannot_repeat() {
    let (_dir, mut runtime, binding) = setup();
    let action = prepared(&mut runtime, &binding.key);
    let mut channel = MockChannel::new(&binding.key);
    assert_eq!(
        runtime.dispatch(&action, 1, &mut channel).unwrap(),
        ActionState::VerifiedOutgoing
    );
    assert_eq!(
        runtime.transition_history(&action).unwrap(),
        ["PREPARED", "EXECUTING", "VERIFIED_OUTGOING"]
    );
    assert!(matches!(
        runtime.dispatch(&action, 2, &mut channel),
        Err(Error::Stale)
    ));
    assert_eq!(channel.send_calls, 1);
}

#[test]
fn unattended_send_gate_needs_only_execution_facts_and_has_no_side_effects() {
    let (_dir, mut runtime, binding) = setup();
    let action_id = prepared(&mut runtime, &binding.key);
    let channel = MockChannel::new(&binding.key);
    let before = channel.live.clone();

    let fill_gate = runtime
        .preview_before_fill_gate(&action_id, 1, &before)
        .unwrap();
    assert_eq!(fill_gate.phase, SendGatePhase::BeforeFill);
    assert!(fill_gate.allowed);
    assert!(fill_gate.blockers.is_empty());
    assert_eq!(runtime.action(&action_id).unwrap().1, ActionState::Prepared);
    assert_eq!(channel.fill_calls, 0);
    assert_eq!(channel.send_calls, 0);

    let (action, _) = runtime.action(&action_id).unwrap();
    let mut after = before.clone();
    after.draft = Draft::Text(action.text.clone());
    let send_gate = runtime
        .preview_before_send_gate(&action_id, 1, &before, &after)
        .unwrap();
    assert_eq!(send_gate.phase, SendGatePhase::BeforeSend);
    assert!(send_gate.allowed);
    assert!(send_gate.blockers.is_empty());
    assert_eq!(runtime.action(&action_id).unwrap().1, ActionState::Prepared);
    assert_eq!(channel.fill_calls, 0);
    assert_eq!(channel.send_calls, 0);
}

#[test]
fn safe_send_gate_reports_preflight_blockers_without_native_calls() {
    for case in 0..11 {
        let (_dir, mut runtime, binding) = setup();
        let action_id = prepared(&mut runtime, &binding.key);
        let channel = MockChannel::new(&binding.key);
        let mut live = channel.live.clone();
        let expected = match case {
            0 => {
                live.application_session_ref.clear();
                SendGateBlocker::ApplicationSessionMissing
            }
            1 => {
                live.conversation_surface_ref.clear();
                SendGateBlocker::ConversationSurfaceMissing
            }
            2 => {
                live.window_ref.clear();
                SendGateBlocker::SurfaceMissing
            }
            3 => {
                live.frontmost = false;
                SendGateBlocker::NotFrontmost
            }
            4 => {
                live.conversation_changed = true;
                SendGateBlocker::ConversationChanged
            }
            5 => {
                live.permitted = false;
                SendGateBlocker::NotPermitted
            }
            6 => {
                live.draft = Draft::Text("unowned leftover draft".into());
                SendGateBlocker::DraftNotEmpty
            }
            7 => {
                live.key.conversation = "other-conversation".into();
                SendGateBlocker::TargetMismatch
            }
            8 => {
                live.identity_epoch += 1;
                SendGateBlocker::IdentityEpochMismatch
            }
            9 => {
                live.draft = Draft::Unreadable;
                SendGateBlocker::DraftNotEmpty
            }
            _ => {
                live.editor_ref.clear();
                SendGateBlocker::SurfaceMissing
            }
        };
        let gate = runtime
            .preview_before_fill_gate(&action_id, 1, &live)
            .unwrap();
        assert!(!gate.allowed, "case {case}");
        assert!(
            gate.blockers.contains(&expected),
            "case {case}: {:?}",
            gate.blockers
        );
        assert_eq!(runtime.action(&action_id).unwrap().1, ActionState::Prepared);
        assert_eq!(channel.fill_calls, 0);
        assert_eq!(channel.send_calls, 0);
    }
}

#[test]
fn safe_send_gate_rechecks_surface_and_exact_draft_after_fill() {
    for case in 0..7 {
        let (_dir, mut runtime, binding) = setup();
        let action_id = prepared(&mut runtime, &binding.key);
        let channel = MockChannel::new(&binding.key);
        let before = channel.live.clone();
        let (action, _) = runtime.action(&action_id).unwrap();
        let mut after = before.clone();
        after.draft = Draft::Text(action.text.clone());
        let expected = match case {
            0 => {
                after.application_session_ref = "different-session".into();
                SendGateBlocker::SurfaceChanged
            }
            1 => {
                after.conversation_surface_ref = "different-conversation-surface".into();
                SendGateBlocker::SurfaceChanged
            }
            2 => {
                after.window_ref = "different-window".into();
                SendGateBlocker::SurfaceChanged
            }
            3 => {
                after.editor_ref = "different-editor".into();
                SendGateBlocker::SurfaceChanged
            }
            4 => {
                after.layout_revision += 1;
                SendGateBlocker::SurfaceChanged
            }
            5 => {
                after.draft = Draft::Text("unexpected readback text".into());
                SendGateBlocker::DraftMismatch
            }
            _ => {
                after.frontmost = false;
                SendGateBlocker::NotFrontmost
            }
        };
        let gate = runtime
            .preview_before_send_gate(&action_id, 1, &before, &after)
            .unwrap();
        assert!(!gate.allowed, "case {case}");
        assert!(
            gate.blockers.contains(&expected),
            "case {case}: {:?}",
            gate.blockers
        );
        assert_eq!(runtime.action(&action_id).unwrap().1, ActionState::Prepared);
        assert_eq!(channel.fill_calls, 0);
        assert_eq!(channel.send_calls, 0);
    }
}

#[test]
fn safe_send_gate_detects_new_revision_and_host_pause_before_gui_work() {
    {
        let (_dir, mut runtime, binding) = setup();
        let action_id = prepared(&mut runtime, &binding.key);
        let channel = MockChannel::new(&binding.key);
        incoming(&mut runtime, &binding.key, "newer", 2);
        let gate = runtime
            .preview_before_fill_gate(&action_id, 2, &channel.live)
            .unwrap();
        assert!(!gate.allowed);
        assert!(gate.blockers.contains(&SendGateBlocker::ActionState));
        assert!(gate.blockers.contains(&SendGateBlocker::StaleAction));
        assert_eq!(channel.fill_calls, 0);
        assert_eq!(channel.send_calls, 0);
    }
    {
        let (_dir, mut runtime, binding) = setup();
        let action_id = prepared(&mut runtime, &binding.key);
        let channel = MockChannel::new(&binding.key);
        runtime.set_host_paused(true).unwrap();
        let gate = runtime
            .preview_before_fill_gate(&action_id, 2, &channel.live)
            .unwrap();
        assert!(!gate.allowed);
        assert!(gate.blockers.contains(&SendGateBlocker::HostPaused));
        assert_eq!(channel.fill_calls, 0);
        assert_eq!(channel.send_calls, 0);
    }
}

#[test]
fn developer_pause_invalidates_old_action_and_resume_only_allows_fresh_work() {
    let (_dir, mut runtime, binding) = setup();
    let action_id = prepared(&mut runtime, &binding.key);
    let mut channel = MockChannel::new(&binding.key);
    runtime.set_host_paused(true).unwrap();

    let gate = runtime
        .preview_before_fill_gate(&action_id, 1, &channel.live)
        .unwrap();
    assert!(!gate.allowed);
    assert!(gate.blockers.contains(&SendGateBlocker::HostPaused));
    assert_eq!(runtime.action(&action_id).unwrap().1, ActionState::Stale);
    assert!(matches!(
        runtime.dispatch(&action_id, 1, &mut channel),
        Err(Error::Stale)
    ));
    assert_eq!(channel.fill_calls, 0);
    assert_eq!(channel.send_calls, 0);

    runtime.set_host_paused(false).unwrap();
    assert!(matches!(
        runtime.dispatch(&action_id, 2, &mut channel),
        Err(Error::Stale)
    ));
    assert_eq!(channel.fill_calls, 0);
    assert_eq!(channel.send_calls, 0);

    incoming(&mut runtime, &binding.key, "after-dev-resume", 2);
    let request_id = ready(&mut runtime, &binding.key, 2);
    let fresh_action = runtime.prepare_send(&request_id, 2, false).unwrap();
    assert_eq!(
        runtime.dispatch(&fresh_action, 2, &mut channel).unwrap(),
        ActionState::VerifiedOutgoing
    );
    assert_eq!(channel.fill_calls, 1);
    assert_eq!(channel.send_calls, 1);
    assert!(matches!(
        runtime.dispatch(&fresh_action, 3, &mut channel),
        Err(Error::Stale)
    ));
    assert_eq!(channel.send_calls, 1);
}

#[test]
fn unattended_dispatch_preserves_identity_draft_permission_and_focus_guards() {
    for case in 0..7 {
        let (_dir, mut runtime, binding) = setup();
        let action = prepared(&mut runtime, &binding.key);
        let mut channel = MockChannel::new(&binding.key);
        match case {
            0 => channel.live.key.account_binding = "another-account".into(),
            1 => channel.live.key.conversation = "another-conversation".into(),
            2 => channel.live.identity_epoch += 1,
            3 => channel.live.draft = Draft::Text("未归属本任务的残留草稿".into()),
            4 => channel.live.draft = Draft::Unreadable,
            5 => channel.live.frontmost = false,
            _ => channel.live.permitted = false,
        }
        assert_eq!(
            runtime.dispatch(&action, 1, &mut channel).unwrap(),
            ActionState::Blocked
        );
        assert_eq!(channel.fill_calls, 0);
        assert_eq!(channel.send_calls, 0);
    }
}

struct FillErrorChannel(MockChannel);

impl MessageChannel for FillErrorChannel {
    fn inspect(&mut self, target: &ConversationKey) -> Result<LiveTarget> {
        self.0.inspect(target)
    }
    fn fill(&mut self, _: &OutboundAction, _: &LiveTarget) -> Result<()> {
        self.0.fill_calls += 1;
        Err(Error::Blocked("synthetic fill readback error"))
    }
    fn send(&mut self, action: &OutboundAction, expected: &LiveTarget) -> Result<SendEvidence> {
        self.0.send(action, expected)
    }
    fn reconcile(&mut self, action: &OutboundAction) -> Result<SendEvidence> {
        self.0.reconcile(action)
    }
}

struct PreSendErrorChannel {
    inner: MockChannel,
    fail_once: bool,
}

impl MessageChannel for PreSendErrorChannel {
    fn inspect(&mut self, target: &ConversationKey) -> Result<LiveTarget> {
        self.inner.inspect(target)
    }
    fn fill(&mut self, action: &OutboundAction, expected: &LiveTarget) -> Result<()> {
        self.inner.fill(action, expected)
    }
    fn send(&mut self, action: &OutboundAction, expected: &LiveTarget) -> Result<SendEvidence> {
        if self.fail_once {
            self.fail_once = false;
            return Err(Error::Blocked("synthetic pre-send no-click result"));
        }
        self.inner.send(action, expected)
    }
    fn reconcile(&mut self, action: &OutboundAction) -> Result<SendEvidence> {
        self.inner.reconcile(action)
    }
}

#[test]
fn explicit_filled_recovery_sends_once_without_refilling() {
    let (_dir, mut runtime, binding) = setup();
    let action_id = prepared(&mut runtime, &binding.key);
    let (action, _) = runtime.action(&action_id).unwrap();
    let mut channel = FillErrorChannel(MockChannel::new(&binding.key));
    assert_eq!(
        runtime.dispatch(&action_id, 1, &mut channel).unwrap(),
        ActionState::Unknown
    );
    assert_eq!(channel.0.fill_calls, 1);
    assert_eq!(channel.0.send_calls, 0);

    channel.0.live.draft = Draft::Text(action.text.clone());
    assert_eq!(
        runtime
            .recover_filled_send(&action_id, 2, &mut channel)
            .unwrap(),
        ActionState::VerifiedOutgoing
    );
    assert_eq!(channel.0.fill_calls, 1);
    assert_eq!(channel.0.send_calls, 1);
    assert!(
        runtime
            .recover_filled_send(&action_id, 3, &mut channel)
            .is_err()
    );
}

#[test]
fn explicit_unattempted_recovery_requires_the_distinct_reason_and_sends_once() {
    let (_dir, mut runtime, binding) = setup();
    let action_id = prepared(&mut runtime, &binding.key);
    let mut channel = PreSendErrorChannel {
        inner: MockChannel::new(&binding.key),
        fail_once: true,
    };
    assert_eq!(
        runtime.dispatch(&action_id, 1, &mut channel).unwrap(),
        ActionState::Unknown
    );
    assert_eq!(channel.inner.fill_calls, 1);
    assert_eq!(channel.inner.send_calls, 0);
    assert!(
        runtime
            .recover_filled_send(&action_id, 2, &mut channel)
            .is_err()
    );
    assert_eq!(
        runtime
            .recover_unattempted_send(&action_id, 2, &mut channel)
            .unwrap(),
        ActionState::VerifiedOutgoing
    );
    assert_eq!(channel.inner.fill_calls, 1);
    assert_eq!(channel.inner.send_calls, 1);
    assert!(
        runtime
            .recover_unattempted_send(&action_id, 3, &mut channel)
            .is_err()
    );
}

#[test]
fn filled_recovery_rejects_other_unknown_reasons_wrong_draft_and_expiry() {
    let (_dir, mut runtime, binding) = setup();
    let action_id = prepared(&mut runtime, &binding.key);
    let (action, _) = runtime.action(&action_id).unwrap();
    let mut no_effect = MockChannel::new(&binding.key);
    no_effect.fault = Fault::FillNoEffect;
    assert_eq!(
        runtime.dispatch(&action_id, 1, &mut no_effect).unwrap(),
        ActionState::Unknown
    );
    no_effect.live.draft = Draft::Text(action.text.clone());
    assert!(
        runtime
            .recover_filled_send(&action_id, 2, &mut no_effect)
            .is_err()
    );
    assert_eq!(no_effect.send_calls, 0);

    let (_dir, mut runtime, binding) = setup();
    let action_id = prepared(&mut runtime, &binding.key);
    let (action, _) = runtime.action(&action_id).unwrap();
    let mut fill_error = FillErrorChannel(MockChannel::new(&binding.key));
    runtime.dispatch(&action_id, 1, &mut fill_error).unwrap();
    fill_error.0.live.draft = Draft::Text("different draft".into());
    assert!(
        runtime
            .recover_filled_send(&action_id, 2, &mut fill_error)
            .is_err()
    );
    fill_error.0.live.draft = Draft::Text(action.text);
    assert!(
        runtime
            .recover_filled_send(&action_id, 1_800_002, &mut fill_error)
            .is_err()
    );
    assert_eq!(fill_error.0.send_calls, 0);
}

#[test]
fn unsuccessful_readback_and_layout_race_never_send() {
    for fault in [
        Fault::FillNoEffect,
        Fault::MoveAfterFill,
        Fault::EditAfterFill,
        Fault::MessageAfterFill,
    ] {
        let (_dir, mut runtime, binding) = setup();
        let action = prepared(&mut runtime, &binding.key);
        let mut channel = MockChannel::new(&binding.key);
        channel.fault = fault;
        assert_eq!(
            runtime.dispatch(&action, 1, &mut channel).unwrap(),
            ActionState::Unknown
        );
        assert_eq!(channel.send_calls, 0);
        assert!(runtime.dispatch(&action, 1, &mut channel).is_err());
    }
}

#[test]
fn unknown_survives_restart_and_reconciles_without_resending() {
    let (dir, mut runtime, binding) = setup();
    let action = prepared(&mut runtime, &binding.key);
    let mut channel = MockChannel::new(&binding.key);
    channel.fault = Fault::CommitThenUnknown;
    assert_eq!(
        runtime.dispatch(&action, 1, &mut channel).unwrap(),
        ActionState::Unknown
    );
    drop(runtime);
    let mut runtime = Runtime::open_simulation(dir.path()).unwrap();
    assert_eq!(runtime.action(&action).unwrap().1, ActionState::Unknown);
    assert!(runtime.dispatch(&action, 2, &mut channel).is_err());
    incoming(&mut runtime, &binding.key, "later", 2);
    assert!(
        runtime
            .begin_reply(&binding.key, 2, None)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        runtime.reconcile(&action, &mut channel).unwrap(),
        ActionState::VerifiedOutgoing
    );
    assert_eq!(channel.send_calls, 1);
}

#[test]
fn absence_or_unrelated_evidence_is_never_proof_of_non_send() {
    let (_dir, mut runtime, binding) = setup();
    let action = prepared(&mut runtime, &binding.key);
    let mut channel = MockChannel::new(&binding.key);
    channel.fault = Fault::WrongReceipt;
    assert_eq!(
        runtime.dispatch(&action, 1, &mut channel).unwrap(),
        ActionState::Unknown
    );
    let mut empty = MockChannel::new(&binding.key);
    assert_eq!(
        runtime.reconcile(&action, &mut empty).unwrap(),
        ActionState::Unknown
    );
    assert_eq!(empty.send_calls, 0);
    assert!(runtime.dispatch(&action, 2, &mut empty).is_err());
}

#[test]
fn submission_is_not_verification_and_recovers_as_unknown() {
    let (dir, mut runtime, binding) = setup();
    let action = prepared(&mut runtime, &binding.key);
    let mut channel = MockChannel::new(&binding.key);
    channel.fault = Fault::SubmitOnly;
    assert_eq!(
        runtime.dispatch(&action, 1, &mut channel).unwrap(),
        ActionState::Submitted
    );
    drop(runtime);
    let runtime = Runtime::open_simulation(dir.path()).unwrap();
    assert_eq!(runtime.action(&action).unwrap().1, ActionState::Unknown);
}

#[test]
fn prepared_action_requires_fresh_inspection_after_restart() {
    let (dir, mut runtime, binding) = setup();
    let action = prepared(&mut runtime, &binding.key);
    drop(runtime);
    let mut runtime = Runtime::open_simulation(dir.path()).unwrap();
    let mut channel = MockChannel::new(&binding.key);
    assert_eq!(
        runtime.dispatch(&action, 1, &mut channel).unwrap(),
        ActionState::VerifiedOutgoing
    );
    assert_eq!(channel.send_calls, 1);
}

#[test]
fn generating_task_is_not_blindly_retried_after_restart() {
    let (dir, mut runtime, binding) = setup();
    incoming(&mut runtime, &binding.key, "one", 1);
    let request = runtime.begin_reply(&binding.key, 1, None).unwrap().unwrap();
    drop(runtime);
    let mut runtime = Runtime::open_simulation(dir.path()).unwrap();
    assert_eq!(runtime.task_state(&request.request_id).unwrap(), "ERROR");
    assert!(
        runtime
            .begin_reply(&binding.key, 2, None)
            .unwrap()
            .is_none()
    );
}

#[test]
fn own_output_and_system_messages_do_not_trigger_replies() {
    let (_dir, mut runtime, binding) = setup();
    for (id, direction) in [("own", Direction::Own), ("system", Direction::System)] {
        let mut event = fixture_observation(&binding.key, id, "合成输出", 1);
        event.message.direction = direction;
        assert_eq!(runtime.ingest(&event).unwrap(), IngestOutcome::Ignored);
    }
    assert!(
        runtime
            .begin_reply(&binding.key, 1, None)
            .unwrap()
            .is_none()
    );
}

#[test]
fn attempt_budget_survives_restart() {
    let (dir, mut runtime, mut binding) = setup();
    binding.max_auto_sends = 1;
    runtime.bind(&binding).unwrap();
    let action = prepared(&mut runtime, &binding.key);
    runtime
        .dispatch(&action, 1, &mut MockChannel::new(&binding.key))
        .unwrap();
    drop(runtime);
    let mut runtime = Runtime::open_simulation(dir.path()).unwrap();
    incoming(&mut runtime, &binding.key, "two", 2);
    let request = ready(&mut runtime, &binding.key, 2);
    let action = runtime.prepare_send(&request, 2, false).unwrap();
    let mut channel = MockChannel::new(&binding.key);
    assert_eq!(
        runtime.dispatch(&action, 2, &mut channel).unwrap(),
        ActionState::Blocked
    );
    assert_eq!(channel.send_calls, 0);
}

#[test]
fn one_owner_per_state_directory_and_release_on_drop() {
    let (dir, runtime, _) = setup();
    assert!(matches!(
        Runtime::open_simulation(dir.path()),
        Err(Error::Busy)
    ));
    drop(runtime);
    assert!(Runtime::open_simulation(dir.path()).is_ok());
}

#[test]
fn manual_takeover_stales_pending_action_without_overwrite() {
    let (_dir, mut runtime, binding) = setup();
    let action = prepared(&mut runtime, &binding.key);
    runtime.pause(&binding.key).unwrap();
    let mut channel = MockChannel::new(&binding.key);
    assert!(runtime.dispatch(&action, 1, &mut channel).is_err());
    assert_eq!(channel.fill_calls, 0);
    assert!(
        runtime
            .begin_reply(&binding.key, 2, None)
            .unwrap()
            .is_none()
    );
}

#[test]
fn observations_from_an_old_account_epoch_are_rejected() {
    let (_dir, mut runtime, mut binding) = setup();
    let old = fixture_observation(&binding.key, "old-account", "合成旧账号观察", 1);
    binding.identity_epoch = 2;
    runtime.bind(&binding).unwrap();
    assert!(matches!(runtime.ingest(&old), Err(Error::Stale)));
    assert_eq!(runtime.summary().unwrap().messages, 0);
    let mut current = old;
    current.identity_epoch = 2;
    assert_eq!(runtime.ingest(&current).unwrap(), IngestOutcome::Queued);
}

#[test]
fn new_identity_epoch_cannot_read_old_history_or_collide_with_old_ids() {
    let (_dir, mut runtime, mut binding) = setup();
    let mut old = fixture_observation(&binding.key, "reused-id", "合成旧账号内容", 1);
    old.historical = true;
    runtime.ingest(&old).unwrap();
    binding.identity_epoch = 2;
    runtime.bind(&binding).unwrap();
    let mut fresh = fixture_observation(&binding.key, "reused-id", "合成新账号内容", 2);
    fresh.identity_epoch = 2;
    assert_eq!(runtime.ingest(&fresh).unwrap(), IngestOutcome::Queued);
    let request = runtime.begin_reply(&binding.key, 2, None).unwrap().unwrap();
    assert_eq!(request.context.len(), 1);
    assert_eq!(
        request.context[0].message.text.as_deref(),
        Some("合成新账号内容")
    );
    assert_eq!(runtime.summary().unwrap().messages, 2);
}

#[test]
fn newly_observed_manual_output_invalidates_unsent_reply() {
    let (_dir, mut runtime, binding) = setup();
    let action = prepared(&mut runtime, &binding.key);
    let mut own = fixture_observation(&binding.key, "manual", "合成人工回答", 2);
    own.message.direction = Direction::Own;
    runtime.ingest(&own).unwrap();
    assert_eq!(runtime.action(&action).unwrap().1, ActionState::Stale);
    let mut channel = MockChannel::new(&binding.key);
    assert!(runtime.dispatch(&action, 2, &mut channel).is_err());
    assert_eq!(channel.send_calls, 0);
    assert!(
        runtime
            .begin_reply(&binding.key, 2, None)
            .unwrap()
            .is_none()
    );
}

#[test]
fn expired_ready_task_is_terminal_not_a_permanent_queue_blocker() {
    let (_dir, mut runtime, binding) = setup();
    incoming(&mut runtime, &binding.key, "one", 1);
    let request = ready(&mut runtime, &binding.key, 1);
    assert!(matches!(
        runtime.prepare_send(&request, 60_002, false),
        Err(Error::Stale)
    ));
    assert_eq!(runtime.task_state(&request).unwrap(), "STALE");
    assert!(runtime.summary().unwrap().actions.is_empty());
}

#[test]
fn provider_scope_rotates_when_profile_changes() {
    let (_dir, mut runtime, mut binding) = setup();
    incoming(&mut runtime, &binding.key, "one", 1);
    let first = runtime.begin_reply(&binding.key, 1, None).unwrap().unwrap();
    binding.profile_version += 1;
    runtime.bind(&binding).unwrap();
    incoming(&mut runtime, &binding.key, "two", 2);
    let second = runtime.begin_reply(&binding.key, 2, None).unwrap().unwrap();
    assert_ne!(first.conversation_ref, second.conversation_ref);
}

#[test]
fn unknown_schema_and_foreign_database_are_not_migrated() {
    for pragma in [
        "PRAGMA user_version=999",
        "PRAGMA user_version=1; PRAGMA application_id=42",
    ] {
        let (dir, runtime, _) = setup();
        drop(runtime);
        let db = rusqlite::Connection::open(dir.path().join("ledger.sqlite3")).unwrap();
        db.execute_batch(pragma).unwrap();
        drop(db);
        assert!(matches!(
            Runtime::open_simulation(dir.path()),
            Err(Error::Schema)
        ));
    }
}

#[test]
fn corrupted_outbox_target_is_not_used_as_routing_authority() {
    let (dir, mut runtime, binding) = setup();
    let action_id = prepared(&mut runtime, &binding.key);
    let (mut action, _) = runtime.action(&action_id).unwrap();
    drop(runtime);
    action.target.conversation = "wrong-contact".into();
    let db = rusqlite::Connection::open(dir.path().join("ledger.sqlite3")).unwrap();
    db.execute(
        "UPDATE outbox SET payload=?1 WHERE id=?2",
        rusqlite::params![serde_json::to_string(&action).unwrap(), action_id],
    )
    .unwrap();
    drop(db);
    let mut runtime = Runtime::open_simulation(dir.path()).unwrap();
    let mut channel = MockChannel::new(&action.target);
    assert!(matches!(
        runtime.dispatch(&action_id, 1, &mut channel),
        Err(Error::Schema)
    ));
    assert_eq!(channel.send_calls, 0);
}

#[cfg(unix)]
#[test]
fn permissive_directories_and_symlinked_state_are_rejected() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let directory = private_dir();
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(
        Runtime::open_simulation(directory.path()),
        Err(Error::UnsafeState)
    ));
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let outside = private_dir();
    let target = outside.path().join("must-not-touch");
    std::fs::write(&target, b"unchanged").unwrap();
    symlink(&target, directory.path().join("ledger.sqlite3")).unwrap();
    assert!(matches!(
        Runtime::open_simulation(directory.path()),
        Err(Error::UnsafeState)
    ));
    assert_eq!(std::fs::read(&target).unwrap(), b"unchanged");
}

#[test]
fn batches_do_not_leak_future_input_into_earlier_context() {
    let (_dir, mut runtime, mut binding) = setup();
    binding.max_batch = 1;
    runtime.bind(&binding).unwrap();
    incoming(&mut runtime, &binding.key, "one", 1);
    incoming(&mut runtime, &binding.key, "two", 2);
    let request = runtime.begin_reply(&binding.key, 2, None).unwrap().unwrap();
    assert_eq!(request.input_events.len(), 1);
    assert_eq!(request.context.len(), 1);
    assert_eq!(
        request.context[0].message.canonical_id.as_deref(),
        Some("one")
    );
}
