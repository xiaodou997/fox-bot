use super::*;
use foxbot_core::{simulation::*, *};
use std::{os::unix::fs::PermissionsExt, time::Instant};

fn binding() -> NativeConversationBinding {
    let mut binding = Binding::paused(fixture_key());
    binding.enabled = true;
    binding.mode = Mode::AutoReply;
    binding.quiet_ms = 0;
    binding.max_wait_ms = 0;
    NativeConversationBinding {
        application_session_fingerprint: "c".repeat(64),
        conversation_fingerprint: "a".repeat(64),
        identity_source: "configured_wechat_title_continuity_v1".into(),
        binding,
    }
}
fn prepared(runtime: &mut Runtime, binding: &NativeConversationBinding) -> String {
    runtime.bind(&binding.binding).unwrap();
    runtime
        .ingest(&fixture_observation(
            &binding.binding.key,
            "one",
            "synthetic input",
            1,
        ))
        .unwrap();
    let request = runtime
        .begin_reply(&binding.binding.key, 1, None)
        .unwrap()
        .unwrap();
    runtime
        .accept_reply(
            &request.request_id,
            &ReplyResponse::for_request(
                &request,
                ReplyOutcome::Reply {
                    text: "FoxBot G3c1 test".into(),
                },
            ),
            1,
        )
        .unwrap();
    runtime.prepare_send(&request.request_id, 1, false).unwrap()
}

// Separate process with the real worker protocol; no desktop, network, or Keychain access.
fn worker(dir: &Path, mode: &str) -> PathBuf {
    let path = dir.join("worker.py");
    let code = format!("#!/usr/bin/env python3\nMODE = {mode:?}\n")
        + r#"
import sys, json, hashlib, pathlib, time
root = pathlib.Path(__file__).parent
marker = root / 'sent.json'
allow = '--allow-single-send' in sys.argv
draft = 'FoxBot G3c1 test' if MODE == 'prefilled' else ''
def sig(text, direction):
    return {'digest': hashlib.sha256(text.encode()).hexdigest(), 'continuity_digest': hashlib.sha256(text.encode()).hexdigest(), 'direction': direction, 'complete': True}
def observe():
    messages = [sig('anchor-a', 'THEM'), sig('anchor-b', 'ME')]
    if marker.exists():
        messages.append(sig(json.loads(marker.read_text())['text'], 'ME'))
    return {'application_session': 'c'*64, 'conversation': 'a'*64, 'window_ref': '1', 'layout_ref': 'd'*64,
            'frontmost': True, 'conversation_resolved': True, 'draft_state': 'NONEMPTY' if draft else 'EMPTY_HEURISTIC',
            'draft_text': draft, 'messages': messages, 'send_button': {'x': .94, 'y': .94}, 'evidence_revision': 'WECHAT_RECEIPT_V3'}
for line in sys.stdin:
    req = json.loads(line)
    cmd = req['command']
    result = {'schema_version': 'foxbot.native-send-worker.v4', 'id': req['id'], 'status': 'OBSERVED',
              'write_attempted': False, 'send_attempted': False, 'verified_outgoing': False}
    if MODE == 'bad-id':
        result['id'] += 1
    if cmd == 'warmup':
        result['status'] = 'WARMED'
    else:
        if MODE == 'timeout':
            time.sleep(10)
        if cmd == 'fill':
            assert allow
            draft = 'wrong text' if MODE == 'bad-fill' else req['text']
            result.update(status='FILLED', write_attempted=True)
        elif cmd in ('send', 'recover_send'):
            assert allow
            assert not marker.exists(), 'second physical send'
            marker.write_text(json.dumps({'text': req['text'], 'sends': 1}))
            draft = ''
            result.update(status='UNKNOWN' if MODE == 'unknown' else 'VERIFIED_OUTGOING',
                          send_attempted=True, verified_outgoing=MODE != 'unknown')
        elif cmd == 'reconcile':
            assert not allow, 'reconciliation must not carry write capability'
            result.update(status='VERIFIED_OUTGOING', verified_outgoing=True)
        result['observation'] = observe()
        if cmd == 'read':
            result['messages'] = [
                {'text': 'anchor-a', 'direction': 'THEM', 'complete': True},
                {'text': 'anchor-b', 'direction': 'ME', 'complete': True},
            ]
            if marker.exists():
                result['messages'].append({'text': json.loads(marker.read_text())['text'], 'direction': 'ME', 'complete': True})
        if MODE == 'false-receipt' and cmd == 'send':
            result['observation']['messages'] = req['expected']['messages']
        if MODE == 'read-side-effect':
            result['send_attempted'] = True
    print(json.dumps(result), flush=True)
"#;
    fs::write(&path, code).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    path
}

#[test]
fn real_worker_protocol_dispatches_once_and_persists_private_receipt_anchors() {
    let dir = tempfile::tempdir().unwrap();
    let owner = DeviceOwner::acquire_at(&dir.path().join("device")).unwrap();
    let entry = binding();
    let mut runtime = Runtime::open_simulation(dir.path().join("runtime")).unwrap();
    let action = prepared(&mut runtime, &entry);
    let binary = worker(dir.path(), "ok");
    let receipt = dir.path().join("receipt.json");
    let mut channel =
        NativeSendChannel::start(&binary, entry, &owner, receipt.clone(), true).unwrap();
    assert_eq!(
        runtime.dispatch(&action, 1, &mut channel).unwrap(),
        ActionState::VerifiedOutgoing
    );
    assert_eq!(channel.stats.fill_requests, 1);
    assert_eq!(channel.stats.send_requests, 1);
    assert_eq!(channel.stats.send_attempted, Some(true));
    assert!(runtime.dispatch(&action, 2, &mut channel).is_err());
    assert_eq!(channel.stats.send_requests, 1);
    let data = fs::read_to_string(receipt).unwrap();
    assert!(!data.contains("FoxBot G3c1 test"));
    assert!(!data.contains("anchor-a"));
}

#[test]
fn unknown_after_send_restarts_and_reconciles_without_a_second_send() {
    let dir = tempfile::tempdir().unwrap();
    let owner = DeviceOwner::acquire_at(&dir.path().join("device")).unwrap();
    let entry = binding();
    let binary = worker(dir.path(), "unknown");
    let receipt = dir.path().join("receipt.json");
    let state = dir.path().join("runtime");
    let mut runtime = Runtime::open_simulation(&state).unwrap();
    let action = prepared(&mut runtime, &entry);
    let mut channel =
        NativeSendChannel::start(&binary, entry.clone(), &owner, receipt.clone(), true).unwrap();
    assert_eq!(
        runtime.dispatch(&action, 1, &mut channel).unwrap(),
        ActionState::Unknown
    );
    drop(channel);
    drop(runtime);
    let mut runtime = Runtime::open_simulation(&state).unwrap();
    let mut read_only = NativeSendChannel::start(&binary, entry, &owner, receipt, false).unwrap();
    assert_eq!(
        runtime.reconcile(&action, &mut read_only).unwrap(),
        ActionState::VerifiedOutgoing
    );
    assert_eq!(read_only.stats.send_requests, 0);
    assert_eq!(read_only.stats.fill_requests, 0);
    assert_eq!(read_only.stats.reconcile_requests, 1);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(
            &fs::read_to_string(dir.path().join("sent.json")).unwrap()
        )
        .unwrap()["sends"],
        1
    );
}

#[test]
fn mismatched_fill_never_calls_send() {
    let dir = tempfile::tempdir().unwrap();
    let owner = DeviceOwner::acquire_at(&dir.path().join("device")).unwrap();
    let entry = binding();
    let mut runtime = Runtime::open_simulation(dir.path().join("runtime")).unwrap();
    let action = prepared(&mut runtime, &entry);
    let mut channel = NativeSendChannel::start(
        &worker(dir.path(), "bad-fill"),
        entry,
        &owner,
        dir.path().join("receipt"),
        true,
    )
    .unwrap();
    assert_eq!(
        runtime.dispatch(&action, 1, &mut channel).unwrap(),
        ActionState::Unknown
    );
    assert_eq!(channel.stats.send_requests, 0);
    assert!(!dir.path().join("sent.json").exists());
}

#[test]
fn claimed_success_without_new_message_evidence_stays_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let owner = DeviceOwner::acquire_at(&dir.path().join("device")).unwrap();
    let entry = binding();
    let mut runtime = Runtime::open_simulation(dir.path().join("runtime")).unwrap();
    let action = prepared(&mut runtime, &entry);
    let mut channel = NativeSendChannel::start(
        &worker(dir.path(), "false-receipt"),
        entry,
        &owner,
        dir.path().join("receipt"),
        true,
    )
    .unwrap();
    assert_eq!(
        runtime.dispatch(&action, 1, &mut channel).unwrap(),
        ActionState::Unknown
    );
    assert_eq!(channel.stats.send_requests, 1);
}

#[test]
fn wrong_request_id_and_side_effects_in_read_reply_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    assert!(
        Worker::start(
            &worker(dir.path(), "bad-id"),
            false,
            Duration::from_secs(20)
        )
        .is_err()
    );
    let mut child = Worker::start(
        &worker(dir.path(), "read-side-effect"),
        false,
        Duration::from_secs(20),
    )
    .unwrap();
    assert!(child.request("inspect", None, None).is_err());
}

#[test]
fn timeout_reaps_worker_and_never_blindly_restarts_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = Worker::start(
        &worker(dir.path(), "timeout"),
        false,
        // Interpreter cold start is not the request timeout under test.
        Duration::from_secs(20),
    )
    .unwrap();
    child.timeout = Duration::from_millis(100);
    let started = Instant::now();
    assert!(child.request("inspect", None, None).is_err());
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(child.child.try_wait().unwrap().is_some());
    assert!(child.request("inspect", None, None).is_err());
}

#[test]
fn legacy_receipt_is_not_rewritten_or_used_to_claim_new_revision_success() {
    let dir = tempfile::tempdir().unwrap();
    let owner = DeviceOwner::acquire_at(&dir.path().join("device")).unwrap();
    let entry = binding();
    let binary = worker(dir.path(), "unknown");
    let receipt = dir.path().join("receipt.json");
    let mut runtime = Runtime::open_simulation(dir.path().join("runtime")).unwrap();
    let action = prepared(&mut runtime, &entry);
    let mut channel =
        NativeSendChannel::start(&binary, entry.clone(), &owner, receipt.clone(), true).unwrap();
    assert_eq!(
        runtime.dispatch(&action, 1, &mut channel).unwrap(),
        ActionState::Unknown
    );
    drop(channel);
    let mut legacy: serde_json::Value =
        serde_json::from_slice(&fs::read(&receipt).unwrap()).unwrap();
    legacy["before"]
        .as_object_mut()
        .unwrap()
        .remove("evidence_revision");
    g2d_real::write_private_json(&receipt, &legacy).unwrap();
    let bytes_before = fs::read(&receipt).unwrap();
    let mut read_only =
        NativeSendChannel::start(&binary, entry, &owner, receipt.clone(), false).unwrap();
    assert_eq!(
        runtime.reconcile(&action, &mut read_only).unwrap(),
        ActionState::Unknown
    );
    assert_eq!(read_only.stats.fill_requests, 0);
    assert_eq!(read_only.stats.send_requests, 0);
    assert_eq!(fs::read(receipt).unwrap(), bytes_before);
}

#[test]
fn unknown_observation_revision_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let mut child =
        Worker::start(&worker(dir.path(), "ok"), false, Duration::from_secs(20)).unwrap();
    let mut observation = child
        .request("inspect", None, None)
        .unwrap()
        .observation
        .unwrap();
    assert!(observation.validate().is_ok());
    observation.evidence_revision = Some("FUTURE_REVISION".into());
    assert!(observation.validate().is_err());
    observation.evidence_revision = None;
    assert!(observation.validate().is_err());
}

#[test]
fn context_signature_can_ignore_spacing_without_relaxing_exact_outgoing_digest() {
    let dir = tempfile::tempdir().unwrap();
    let mut child =
        Worker::start(&worker(dir.path(), "ok"), false, Duration::from_secs(20)).unwrap();
    let before = child
        .request("inspect", None, None)
        .unwrap()
        .observation
        .unwrap();
    let mut after = before.clone();
    after.messages[0].digest = "e".repeat(64);
    assert!(before.same_messages(&after));
    let text = "回复 42";
    let digest = format!("{:x}", Sha256::digest(text.as_bytes()));
    after.messages.push(MessageSignature {
        digest: digest.clone(),
        direction: "ME".into(),
        complete: true,
        continuity_digest: Some(digest),
    });
    assert!(!before.same_messages(&after));
    assert!(matching_outgoing(&before, &after, text));
    assert!(!matching_outgoing(&before, &after, "回复42"));
    after.messages[0].continuity_digest = Some("f".repeat(64));
    assert!(!matching_outgoing(&before, &after, text));
}

#[test]
fn unbound_pair_validates_current_context_before_a_binding_refresh() {
    let dir = tempfile::tempdir().unwrap();
    let owner = DeviceOwner::acquire_at(&dir.path().join("device")).unwrap();
    let binary = worker(dir.path(), "ok");
    let (first, second) = read_unbound_pair(&binary, &owner).unwrap();
    assert!(first.observation.same_surface(&second.observation));
    assert!(first.observation.same_messages(&second.observation));
    assert_eq!(second.messages.len(), 2);

    let mut wrong = binding();
    wrong.application_session_fingerprint = "e".repeat(64);
    let mut channel =
        NativeSendChannel::start(&binary, wrong, &owner, dir.path().join("receipt"), false)
            .unwrap();
    assert!(channel.read_messages().is_err());
}

#[test]
fn filled_recovery_sends_the_existing_draft_without_a_fill_call() {
    let dir = tempfile::tempdir().unwrap();
    let owner = DeviceOwner::acquire_at(&dir.path().join("device")).unwrap();
    let entry = binding();
    let mut runtime = Runtime::open_simulation(dir.path().join("runtime")).unwrap();
    let action_id = prepared(&mut runtime, &entry);
    let (action, _) = runtime.action(&action_id).unwrap();
    let mut channel = NativeSendChannel::start_filled_recovery(
        &worker(dir.path(), "prefilled"),
        entry,
        &owner,
        dir.path().join("receipt.json"),
        &action.text,
    )
    .unwrap();
    let live = channel.inspect(&action.target).unwrap();
    assert_eq!(live.draft, Draft::Text(action.text.clone()));
    assert!(matches!(
        channel.send(&action, &live).unwrap(),
        SendEvidence::ObservedOutgoing { .. }
    ));
    assert_eq!(channel.stats.fill_requests, 0);
    assert_eq!(channel.stats.send_requests, 1);
    assert_eq!(channel.stats.send_attempted, Some(true));
}

#[test]
fn recovery_draft_caret_one_is_narrow_and_expected_aware() {
    assert!(verified_draft_text(
        Some("南京是一座历史文化名城1"),
        "南京是一座历史文化名城"
    ));
    assert!(!verified_draft_text(
        Some("南京是一座历史文化古城1"),
        "南京是一座历史文化名城"
    ));
    assert!(!verified_draft_text(Some("reply1"), "reply"));
    assert!(verified_draft_text(Some("版本1"), "版本1"));
}

#[test]
fn wrong_binding_and_lost_device_owner_block_before_fill() {
    let dir = tempfile::tempdir().unwrap();
    let device = dir.path().join("device");
    let owner = DeviceOwner::acquire_at(&device).unwrap();
    let mut entry = binding();
    entry.conversation_fingerprint = "b".repeat(64);
    let mut channel = NativeSendChannel::start(
        &worker(dir.path(), "ok"),
        entry,
        &owner,
        dir.path().join("receipt"),
        true,
    )
    .unwrap();
    assert!(channel.observe().is_err());
    fs::remove_file(device.join("device-owner.lock")).unwrap();
    assert!(channel.observe().is_err());
    assert_eq!(channel.stats.fill_requests, 0);
    assert_eq!(channel.stats.send_requests, 0);
}
