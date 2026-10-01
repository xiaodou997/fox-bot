use super::*;
use crate::native_send::MessageSignature;
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
        schema_version: 1,
        http,
        token: None,
        ledger_key: CredentialRef {
            id: "synthetic-key".into(),
        },
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
    return {'application_session':'c'*64,'conversation':'a'*64,'window_ref':'1','layout_ref':'d'*64,
        'frontmost':True,'conversation_resolved':True,'draft_state':'NONEMPTY' if draft else 'EMPTY_HEURISTIC',
        'draft_text':draft,'send_button':{'x':.94,'y':.94},'evidence_revision':'WECHAT_RECEIPT_V3',
        'messages':[dict(digest=hashlib.sha256(m['text'].encode()).hexdigest(),continuity_digest=hashlib.sha256(m['text'].encode()).hexdigest(),direction=m['direction'],complete=m['complete']) for m in rows]}
for line in sys.stdin:
    q=json.loads(line); cmd=q['command']
    r={'schema_version':'foxbot.native-send-worker.v4','id':q['id'],'status':'OBSERVED',
       'write_attempted':False,'send_attempted':False,'verified_outgoing':False}
    if cmd=='warmup': r['status']='WARMED'
    else:
        if cmd=='fill':
            assert allow
            draft=q['text'];r.update(status='FILLED',write_attempted=True)
        elif cmd=='send':
            assert allow and not marker.exists()
            marker.write_text(json.dumps({'text':q['text']}));draft=''
            r.update(status='UNKNOWN' if MODE=='unknown' else 'VERIFIED_OUTGOING',send_attempted=True,verified_outgoing=MODE!='unknown')
        elif cmd=='reconcile':
            assert not allow
            r.update(status='VERIFIED_OUTGOING',verified_outgoing=True)
        rows=data();r['observation']=observation(rows)
        if cmd=='read':
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
        let mut runtime = Runtime::open_encrypted(dir.path().join("runtime"), &[7; 32]).unwrap();
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
        self.runtime = Runtime::open_simulation(self.dir.path().join("unused-temp")).unwrap();
        self.runtime = Runtime::open_encrypted(self.dir.path().join("runtime"), &[7; 32]).unwrap();
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
    let marker = b"new-question-771";
    assert!(!bytes.windows(marker.len()).any(|v| v == marker));
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
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invalid_and_unsupported_replies_never_write_or_repeat_the_model_call() {
    let response = |text: String| {
        serde_json::to_vec(&serde_json::json!({"choices":[{"finish_reason":"stop","message":{"role":"assistant","content":text}}]})).unwrap()
    };
    for bytes in [
        b"not-json".to_vec(),
        response("a".repeat(81)),
        response("line1\nline2".into()),
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
#[test]
fn config_rejects_embedded_secrets_and_reply_limits_are_non_destructive() {
    let mut value: serde_json::Value =
        serde_json::from_str(include_str!("../../../examples/g3c2-reply.json")).unwrap();
    value["api_key"] = serde_json::json!("not-allowed");
    assert!(serde_json::from_value::<ReplyOnceConfig>(value).is_err());
    for t in ["", "leading ", "line1\nline2", "x|"] {
        assert!(!supported_reply(t));
    }
    assert!(supported_reply("简短中文测试回复"));
}
