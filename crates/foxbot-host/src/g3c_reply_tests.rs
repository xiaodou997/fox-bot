use super::*;
use crate::native_send::{MAX_SEND_LINES, MessageSignature};
use crate::tests::support::Server;
use foxbot_core::{Binding, simulation::fixture_key};
use foxbot_http::ContextMode;
use std::{fs, os::unix::fs::PermissionsExt};

fn binding() -> NativeConversationBinding {
    let mut b = Binding::paused(fixture_key());
    b.enabled = true;
    b.quiet_ms = 0;
    b.max_wait_ms = 0;
    b.reply_ttl_ms = 60_000;
    b.max_auto_sends = 1;
    NativeConversationBinding {
        binding: b,
        application_session_fingerprint: "c".repeat(64),
        conversation_fingerprint: "a".repeat(64),
        identity_source: "configured_wechat_title_continuity_v1".into(),
    }
}
fn configuration(server: &Server, business: bool) -> ReplyOnceConfig {
    let mut http = server.config(
        if business {
            Protocol::BusinessV1
        } else {
            Protocol::ChatCompletions
        },
        if business {
            ContextMode::ServiceManaged
        } else {
            ContextMode::ClientManaged
        },
    );
    http.max_attempts = 1;
    http.max_in_flight = 1;
    ReplyOnceConfig {
        schema_version: 2,
        http,
        token: None,
        ledger_key: None,
        connection_id: Some("test-connection".into()),
        api_key: Some(String::new()),
        provider: ProviderProfile::Custom,
        wait_ms: 100,
        poll_ms: 100,
    }
}
fn messages(path: &Path, incoming: &[(&str, &str)]) {
    let mut values = vec![
        serde_json::json!({"text":"history-a","direction":"THEM","complete":true}),
        serde_json::json!({"text":"history-b","direction":"ME","complete":true}),
    ];
    values.extend(incoming.iter().map(
        |(text, direction)| serde_json::json!({"text":text,"direction":direction,"complete":true}),
    ));
    fs::write(
        path.join("messages.json"),
        serde_json::to_vec(&values).unwrap(),
    )
    .unwrap();
}
fn worker(path: &Path, mode: &str) -> PathBuf {
    let file = path.join("worker.py");
    let code = format!("#!/usr/bin/env python3\nMODE={mode:?}\n")
        + r#"
import pathlib,json,sys,hashlib
root=pathlib.Path(__file__).parent
allow='--allow-single-send' in sys.argv
draft=''
marker=root/'sent.json'
def data():
    rows=json.loads((root/'messages.json').read_text())
    if marker.exists(): rows.append({'text':json.loads(marker.read_text())['text'],'direction':'ME','complete':True})
    return rows
def observation(rows):
    def signature(m):
        content=''.join(m['text'].split())
        return dict(digest=hashlib.sha256(m['text'].encode()).hexdigest(),continuity_digest=hashlib.sha256(m['text'].encode()).hexdigest(),content_digest=hashlib.sha256(content.encode()).hexdigest(),direction=m['direction'],complete=m['complete'])
    return {'application_session':'c'*64,'conversation':'a'*64,'window_ref':'1','layout_ref':'d'*64,
        'frontmost':True,'conversation_resolved':True,'draft_state':'NONEMPTY' if draft else 'EMPTY_HEURISTIC',
        'draft_text':draft,'send_button':{'x':.94,'y':.94},'evidence_revision':'WECHAT_RECEIPT_V4',
        'messages':[signature(m) for m in rows]}
for line in sys.stdin:
    q=json.loads(line); cmd=q['command']
    r={'schema_version':'foxbot.native-send-worker.v5','id':q['id'],'status':'OBSERVED',
       'write_attempted':False,'send_attempted':False,'verified_outgoing':False}
    if cmd=='warmup': r['status']='WARMED'
    else:
        if cmd=='fill':
            assert allow
            draft=q['text'];r.update(status='FILLED',write_attempted=True)
        elif cmd in ('send','recover_send'):
            assert allow and not marker.exists()
            marker.write_text(json.dumps({'text':q['text']}));draft=''
            r.update(status='UNKNOWN' if MODE=='unknown' else 'VERIFIED_OUTGOING',send_attempted=True,verified_outgoing=MODE!='unknown')
        elif cmd=='reconcile':
            assert not allow
            r.update(status='VERIFIED_OUTGOING',verified_outgoing=True)
        rows=data();r['observation']=observation(rows)
        if cmd in ('read','recover_read'):
            r['messages']=rows
            if MODE=='tampered':r['messages'][-1]['text']='untrusted text not matching digest'
    print(json.dumps(r),flush=True)
"#;
    fs::write(&file, code).unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o700)).unwrap();
    file
}
struct Fixture {
    dir: tempfile::TempDir,
    owner: DeviceOwner,
    binary: PathBuf,
    config: ReplyOnceConfig,
    service: HttpReplyService,
    runtime: Runtime,
    state: RunState,
}
impl Fixture {
    fn new(server: &Server, business: bool, mode: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let owner = DeviceOwner::acquire_at(&dir.path().join("device")).unwrap();
        let binary = worker(dir.path(), mode);
        messages(dir.path(), &[]);
        let config = configuration(server, business);
        config.validate().unwrap();
        let service = HttpReplyService::new(config.http.clone(), None).unwrap();
        let binding = binding();
        let mut channel = NativeSendChannel::start(
            &binary,
            binding.clone(),
            &owner,
            dir.path().join("receipt.json"),
            false,
        )
        .unwrap();
        let frame = channel.read_messages().unwrap();
        drop(channel);
        let mut runtime = Runtime::open_local(dir.path().join("runtime")).unwrap();
        let state = arm_runtime(
            &mut runtime,
            &config,
            &service,
            binding,
            frame,
            RunClock::default().now_ms(),
        )
        .unwrap();
        write_private_json(&dir.path().join("run.json"), &state).unwrap();
        Self {
            dir,
            owner,
            binary,
            config,
            service,
            runtime,
            state,
        }
    }
    async fn execute(&mut self, cancel: CancellationToken) -> serde_json::Value {
        execute_runtime(
            &self.config,
            &self.service,
            &mut self.state,
            &mut self.runtime,
            &self.binary,
            &self.owner,
            self.dir.path(),
            cancel,
        )
        .await
        .unwrap()
    }
    fn reopen(&mut self) {
        let replacement = Runtime::open_simulation(self.dir.path().join("unused-temp")).unwrap();
        let previous = std::mem::replace(&mut self.runtime, replacement);
        drop(previous);
        let path = self.dir.path().join("runtime");
        let mut opened = None;
        for _ in 0..20 {
            match Runtime::open_local(&path) {
                Ok(runtime) => {
                    opened = Some(runtime);
                    break;
                }
                Err(_) => std::thread::sleep(Duration::from_millis(5)),
            }
        }
        self.runtime = opened.expect("runtime owner lock was not released after close");
        self.state = read_private_json(&self.dir.path().join("run.json")).unwrap();
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn new_incoming_calls_http_and_native_once_and_replay_does_neither() {
    let server = Server::start().await;
    let mut f = Fixture::new(&server, false, "ok");
    messages(f.dir.path(), &[("new-question-771", "THEM")]);
    let result = f.execute(CancellationToken::new()).await;
    assert_eq!(result["action_state"], "VERIFIED_OUTGOING");
    assert_eq!(result["model_jobs_this_invocation"], 1);
    assert_eq!(result["native"]["send_requests"], 1);
    assert_eq!(server.state.lock().unwrap().seen.len(), 1);
    let request = server.state.lock().unwrap().seen[0].body.clone();
    assert!(request.to_string().contains("new-question-771"));
    assert!(!request.to_string().contains("Operator-authorized"));
    f.reopen();
    let replay = f.execute(CancellationToken::new()).await;
    assert_eq!(replay["action_state"], "VERIFIED_OUTGOING");
    assert_eq!(replay["model_jobs_this_invocation"], 0);
    assert_eq!(replay["native"]["read_requests"], 0);
    assert_eq!(replay["native"]["send_requests"], 0);
    assert_eq!(server.state.lock().unwrap().seen.len(), 1);
    assert!(
        !fs::read_to_string(f.dir.path().join("run.json"))
            .unwrap()
            .contains("new-question-771")
    );
    let bytes = fs::read(f.dir.path().join("runtime/ledger.sqlite3")).unwrap();
    assert!(bytes.starts_with(b"SQLite format 3\0"));
    assert_eq!(result["encrypted_outbox"], false);
    assert_eq!(result["connection_id"], "test-connection");
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_new_message_never_invokes_model_and_arm_baseline_stays_fixed() {
    let server = Server::start().await;
    let mut f = Fixture::new(&server, false, "ok");
    let baseline = f.state.baseline.clone();
    let result = f.execute(CancellationToken::new()).await;
    assert_eq!(result["status"], "WAITING_FOR_NEW_MESSAGE");
    assert_eq!(result["model_jobs_this_invocation"], 0);
    assert_eq!(f.state.baseline, baseline);
    assert!(server.state.lock().unwrap().seen.is_empty());
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn own_message_and_multiple_new_messages_are_not_sent_to_ai() {
    for entries in [vec![("own", "ME")], vec![("q1", "THEM"), ("q2", "THEM")]] {
        let server = Server::start().await;
        let mut f = Fixture::new(&server, false, "ok");
        messages(f.dir.path(), &entries);
        let result = f.execute(CancellationToken::new()).await;
        assert_eq!(result["model_jobs_this_invocation"], 0);
        assert_eq!(result["native"]["fill_requests"], 0);
        assert!(server.state.lock().unwrap().seen.is_empty());
    }
}
fn chat_completion(text: String) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":text}}]
    }))
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invalid_and_unsupported_replies_never_write_or_repeat_the_model_call() {
    for bytes in [
        b"not-json".to_vec(),
        chat_completion("a".repeat(MAX_SEND_UTF16 + 1)),
        chat_completion(vec!["line"; MAX_SEND_LINES + 1].join("\n")),
        chat_completion("line1\tline2".into()),
        chat_completion("line1\r\nline2".into()),
    ] {
        let server = Server::start().await;
        server.state.lock().unwrap().response_override = Some(bytes);
        let mut f = Fixture::new(&server, false, "ok");
        messages(f.dir.path(), &[("question", "THEM")]);
        let result = f.execute(CancellationToken::new()).await;
        assert_eq!(result["native"]["fill_requests"], 0);
        assert_eq!(result["native"]["send_requests"], 0);
        assert_ne!(result["status"], "VERIFIED_OUTGOING");
        f.reopen();
        f.execute(CancellationToken::new()).await;
        assert_eq!(server.state.lock().unwrap().seen.len(), 1);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bounded_multiline_and_long_replies_complete_without_truncation() {
    for text in [
        "第一段：南京有深厚的历史文化底蕴。\n第二段：这里也有活跃的创新产业。".into(),
        "长".repeat(300),
    ] {
        let server = Server::start().await;
        server.state.lock().unwrap().response_override = Some(chat_completion(text.clone()));
        let mut fixture = Fixture::new(&server, false, "ok");
        messages(fixture.dir.path(), &[("question", "THEM")]);
        let result = fixture.execute(CancellationToken::new()).await;
        assert_eq!(result["action_state"], "VERIFIED_OUTGOING", "{result}");
        assert_eq!(result["native"]["fill_requests"], 1);
        assert_eq!(result["native"]["send_requests"], 1);
        let sent: serde_json::Value =
            serde_json::from_slice(&fs::read(fixture.dir.path().join("sent.json")).unwrap())
                .unwrap();
        assert_eq!(sent["text"], text);
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn new_message_during_generation_blocks_the_old_answer() {
    let server = Server::start().await;
    server.state.lock().unwrap().generate_delay_ms = 400;
    let mut f = Fixture::new(&server, false, "ok");
    messages(f.dir.path(), &[("question", "THEM")]);
    let path = f.dir.path().to_owned();
    let shared = server.state.clone();
    let changer = tokio::spawn(async move {
        for _ in 0..200 {
            if !shared.lock().unwrap().seen.is_empty() {
                messages(&path, &[("question", "THEM"), ("newer-question", "THEM")]);
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("no HTTP request observed");
    });
    let result = f.execute(CancellationToken::new()).await;
    changer.await.unwrap();
    assert_eq!(result["status"], "REPLY_CANCELLED_OR_CONTEXT_CHANGED");
    assert_eq!(result["native"]["fill_requests"], 0);
    assert_eq!(result["native"]["send_requests"], 0);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unknown_send_reconciles_readonly_without_regenerating() {
    let server = Server::start().await;
    let mut f = Fixture::new(&server, false, "unknown");
    messages(f.dir.path(), &[("question", "THEM")]);
    assert_eq!(
        f.execute(CancellationToken::new()).await["action_state"],
        "UNKNOWN"
    );
    f.reopen();
    let result = f.execute(CancellationToken::new()).await;
    assert_eq!(result["action_state"], "VERIFIED_OUTGOING");
    assert_eq!(result["native"]["send_requests"], 0);
    assert_eq!(result["native"]["reconcile_requests"], 1);
    assert_eq!(server.state.lock().unwrap().seen.len(), 1);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interrupted_generation_claim_never_starts_a_second_model_request() {
    let server = Server::start().await;
    let mut f = Fixture::new(&server, false, "ok");
    f.state.progress = Progress::Started;
    let result = f.execute(CancellationToken::new()).await;
    assert_eq!(result["status"], "INTERRUPTED_NO_AUTOMATIC_RETRY");
    assert_eq!(result["native"]["read_requests"], 0);
    assert!(server.state.lock().unwrap().seen.is_empty());
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn custom_business_provider_has_no_injected_prompt_and_uses_separate_feedback() {
    let server = Server::start().await;
    let mut f = Fixture::new(&server, true, "ok");
    messages(f.dir.path(), &[("business question", "THEM")]);
    let result = f.execute(CancellationToken::new()).await;
    assert_eq!(result["action_state"], "VERIFIED_OUTGOING");
    assert_eq!(result["feedback_status"], "ACKNOWLEDGED");
    let state = server.state.lock().unwrap();
    assert_eq!(
        state.seen.iter().filter(|r| r.path == "/generate").count(),
        1
    );
    assert!(
        state.seen[0]
            .body
            .get("system_prompt")
            .is_none_or(|v| v.is_null())
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn config_change_and_cancellation_block_external_side_effects() {
    let server = Server::start().await;
    let mut f = Fixture::new(&server, false, "ok");
    let token = CancellationToken::new();
    token.cancel();
    let result = f.execute(token).await;
    assert_eq!(result["model_jobs_this_invocation"], 0);
    let mut changed = f.config.clone();
    changed.http.model = Some("different".into());
    assert!(validate_state(&f.state, &changed, &f.service).is_err());
    assert!(server.state.lock().unwrap().seen.is_empty());
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inconsistent_private_text_and_digest_is_rejected_before_ingest() {
    let server = Server::start().await;
    let f = Fixture::new(&server, false, "ok");
    let bad = worker(f.dir.path(), "tampered");
    let mut channel = NativeSendChannel::start(
        &bad,
        binding(),
        &f.owner,
        f.dir.path().join("receipt.json"),
        false,
    )
    .unwrap();
    assert!(channel.read_messages().is_err());
    assert_eq!(channel.stats.fill_requests, 0);
    assert!(server.state.lock().unwrap().seen.is_empty());
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn timeout_does_not_retry_non_idempotent_generation() {
    let server = Server::start().await;
    server.state.lock().unwrap().generate_delay_ms = 400;
    let mut f = Fixture::new(&server, false, "ok");
    f.config.http.attempt_timeout_ms = 50;
    f.config.http.total_timeout_ms = 50;
    f.service = HttpReplyService::new(f.config.http.clone(), None).unwrap();
    f.state.config_digest = f.config.digest().unwrap();
    f.state.profile_tag = f.service.profile_tag().into();
    messages(f.dir.path(), &[("question", "THEM")]);
    let result = f.execute(CancellationToken::new()).await;
    assert_eq!(result["status"], "MODEL_FAILED_OR_INCOMPLETE");
    assert_eq!(result["native"]["fill_requests"], 0);
    f.reopen();
    f.execute(CancellationToken::new()).await;
    assert_eq!(server.state.lock().unwrap().seen.len(), 1);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_reply_and_handoff_from_custom_service_never_create_outbound_action() {
    for outcome in [
        serde_json::json!({"result":"no_reply"}),
        serde_json::json!({"result":"handoff","reason":"synthetic handoff"}),
    ] {
        let server = Server::start().await;
        server.state.lock().unwrap().outcome = outcome;
        let mut f = Fixture::new(&server, true, "ok");
        messages(f.dir.path(), &[("question", "THEM")]);
        let result = f.execute(CancellationToken::new()).await;
        assert!(result["action_state"].is_null());
        assert_eq!(result["native"]["fill_requests"], 0);
        assert_eq!(result["native"]["send_requests"], 0);
    }
}

#[test]
fn fresh_identical_text_is_new_but_existing_message_is_not() {
    fn sig(t: &str, d: &str) -> MessageSignature {
        let digest = format!("{:x}", Sha256::digest(t.as_bytes()));
        MessageSignature {
            digest: digest.clone(),
            direction: d.into(),
            complete: true,
            continuity_digest: Some(digest),
            content_digest: None,
        }
    }
    let before = NativeSendObservation {
        application_session: "c".repeat(64),
        conversation: "a".repeat(64),
        window_ref: "1".into(),
        layout_ref: "d".repeat(64),
        frontmost: true,
        conversation_resolved: true,
        draft_state: "EMPTY_HEURISTIC".into(),
        draft_text: Some("".into()),
        messages: vec![sig("same", "THEM"), sig("reply", "ME")],
        send_button: None,
        evidence_revision: Some("WECHAT_RECEIPT_V3".into()),
    };
    assert_eq!(new_incoming(&before, &before).unwrap(), None);
    let mut after = before.clone();
    after.messages.push(sig("same", "THEM"));
    assert_eq!(new_incoming(&before, &after).unwrap(), Some(2));
    after.conversation = "b".repeat(64);
    assert!(new_incoming(&before, &after).is_err());
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn multi_connection_defaults_and_other_edits_do_not_switch_a_pinned_task() {
    use crate::local_config::{Connection, LocalConfig, ReplyPreferences};
    let a = Server::start().await;
    let b = Server::start().await;
    let mut f = Fixture::new(&a, false, "ok");
    let make = |id: &str, server: &Server| Connection {
        id: id.into(),
        name: id.into(),
        protocol: Protocol::ChatCompletions,
        endpoint: format!("{}/chat/completions", server.url),
        api_key: format!("synthetic-key-{id}"),
        model: Some("synthetic-model".into()),
        context_mode: ContextMode::ClientManaged,
        receipt_endpoint: None,
        idempotency_supported: false,
        staging_contract: false,
    };
    let mut local = LocalConfig {
        version: 2,
        default_connection: Some("a".into()),
        connections: vec![make("a", &a), make("b", &b)],
        reply: ReplyPreferences::default(),
    };
    f.config = ReplyOnceConfig::from_local(&local, None).unwrap();
    f.config.wait_ms = 100;
    f.config.poll_ms = 100;
    f.service = f.config.service(&NativeCredentials).unwrap(); // Inline mode never invokes the store.
    f.state.connection_id = Some("a".into());
    f.state.config_digest = f.config.digest().unwrap();
    f.state.profile_tag = f.service.profile_tag().into();
    local.default_connection = Some("b".into());
    local.connections[1].api_key = "edited-unrelated-key".into();
    local.connections[0].name = "renamed-a".into();
    let pinned = ReplyOnceConfig::from_local(&local, f.state.connection_id.as_deref()).unwrap();
    assert!(
        validate_state(
            &f.state,
            &pinned,
            &pinned.service(&NativeCredentials).unwrap()
        )
        .is_ok()
    );
    let config_path = f.dir.path().join("config.json");
    write_private_json(&config_path, &local).unwrap();
    let loaded =
        ReplyOnceConfig::load_selected(&config_path, f.state.connection_id.as_deref()).unwrap();
    assert_eq!(loaded.connection_id.as_deref(), Some("a"));
    messages(f.dir.path(), &[("only-a-should-reply", "THEM")]);
    assert_eq!(
        f.execute(CancellationToken::new()).await["action_state"],
        "VERIFIED_OUTGOING"
    );
    assert_eq!(a.state.lock().unwrap().seen.len(), 1);
    assert_eq!(b.state.lock().unwrap().seen.len(), 0);
    f.reopen();
    f.execute(CancellationToken::new()).await;
    assert_eq!(a.state.lock().unwrap().seen.len(), 1);
    local.connections[0].api_key = "changed-active-key".into();
    let changed = ReplyOnceConfig::from_local(&local, Some("a")).unwrap();
    assert!(
        validate_state(
            &f.state,
            &changed,
            &changed.service(&NativeCredentials).unwrap()
        )
        .is_err()
    );
    local.connections.remove(0);
    assert!(ReplyOnceConfig::from_local(&local, Some("a")).is_err());
}

#[test]
fn normal_local_store_needs_no_credentials_and_never_rewrites_encrypted_history() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("local");
    let runtime = Runtime::open_local(&root).unwrap();
    assert!(!runtime.is_encrypted());
    drop(runtime);
    assert!(
        fs::read(root.join("ledger.sqlite3"))
            .unwrap()
            .starts_with(b"SQLite format 3\0")
    );
    assert!(Runtime::open_local(&root).is_ok());
    let old = dir.path().join("old");
    fs::create_dir(&old).unwrap();
    fs::write(
        old.join("ledger.sqlite3"),
        b"opaque encrypted old database contents",
    )
    .unwrap();
    let before = fs::read(old.join("ledger.sqlite3")).unwrap();
    assert!(Runtime::open_local(&old).is_err());
    assert_eq!(before, fs::read(old.join("ledger.sqlite3")).unwrap());
    assert!(!old.join("owner.lock").exists());
}

#[test]
fn config_rejects_embedded_secrets_and_reply_limits_are_non_destructive() {
    let mut value: serde_json::Value =
        serde_json::from_str(include_str!("../../../examples/g3c2-reply.json")).unwrap();
    value["api_key"] = serde_json::json!("not-allowed");
    assert!(serde_json::from_value::<ReplyOnceConfig>(value).is_err());
    for text in ["", "leading ", "line1\tline2", "x|"] {
        assert!(!supported_reply(text));
    }
    assert!(!supported_reply(&"x".repeat(MAX_SEND_UTF16 + 1)));
    assert!(supported_reply("简短中文测试回复"));
    assert!(supported_reply("第一行\n第二行"));
}
