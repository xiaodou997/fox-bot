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
draft = ''
def sig(text, direction):
    return {'digest': hashlib.sha256(text.encode()).hexdigest(), 'direction': direction, 'complete': True}
def observe():
    messages = [sig('anchor-a', 'THEM'), sig('anchor-b', 'ME')]
    if marker.exists():
        messages.append(sig(json.loads(marker.read_text())['text'], 'ME'))
    return {'application_session': 'c'*64, 'conversation': 'a'*64, 'window_ref': '1', 'layout_ref': 'd'*64,
            'frontmost': True, 'conversation_resolved': True, 'draft_state': 'NONEMPTY' if draft else 'EMPTY_HEURISTIC',
            'draft_text': draft, 'messages': messages, 'send_button': {'x': .94, 'y': .94}}
for line in sys.stdin:
    req = json.loads(line)
    cmd = req['command']
    result = {'schema_version': 'foxbot.native-send-worker.v1', 'id': req['id'], 'status': 'OBSERVED',
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
        elif cmd == 'send':
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
