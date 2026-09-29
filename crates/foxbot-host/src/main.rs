//! Development host: stdin commands and synthetic senders; never operates a real chat app.
use foxbot_core::{simulation::*, *};
use foxbot_host::{
    credentials::{CredentialRef, CredentialStore, NativeCredentials, Secret},
    native_bridge::{
        GroundTruthAcceptance, NativeConversationBinding, NativeObservationBridge,
        PrivateBridgeMessage, PrivateDirection, PrivateMessageSnapshot,
    },
    native_read::{NativeReadHost, NativeWorkerConfig},
    ownership::DeviceOwner,
    *,
};
use foxbot_http::RunClock;
use std::{
    collections::HashMap,
    io::{BufRead, Read, Write},
    path::PathBuf,
    time::Duration,
};
use tokio::sync::mpsc;

#[derive(serde::Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
enum Command {
    Resume,
    Pause,
    Status,
    Stop,
    Message {
        session: usize,
        id: String,
        text: String,
        #[serde(default)]
        historical: bool,
    },
}
struct SyntheticChannels {
    map: HashMap<String, MockChannel>,
}
impl SyntheticChannels {
    fn new(bindings: &[Binding]) -> foxbot_host::Result<Self> {
        let mut map = HashMap::new();
        for b in bindings {
            let mut c = MockChannel::new(&b.key);
            c.live.identity_epoch = b.identity_epoch;
            map.insert(
                serde_json::to_string(&b.key).map_err(|_| HostError::Config)?,
                c,
            );
        }
        Ok(Self { map })
    }
    fn get(&mut self, key: &ConversationKey) -> foxbot_core::Result<&mut MockChannel> {
        self.map
            .get_mut(&serde_json::to_string(key)?)
            .ok_or(foxbot_core::Error::NotFound)
    }
}
impl MessageChannel for SyntheticChannels {
    fn inspect(&mut self, k: &ConversationKey) -> foxbot_core::Result<LiveTarget> {
        self.get(k)?.inspect(k)
    }
    fn fill(&mut self, a: &OutboundAction, e: &LiveTarget) -> foxbot_core::Result<()> {
        self.get(&a.target)?.fill(a, e)
    }
    fn send(&mut self, a: &OutboundAction, e: &LiveTarget) -> foxbot_core::Result<SendEvidence> {
        self.get(&a.target)?.send(a, e)
    }
    fn reconcile(&mut self, a: &OutboundAction) -> foxbot_core::Result<SendEvidence> {
        self.get(&a.target)?.reconcile(a)
    }
}
fn emit(event: &str, status: Option<&HostSnapshot>) -> foxbot_host::Result<()> {
    println!(
        "{}",
        serde_json::json!({"event":event,"synthetic_only":true,"native_chat_operations":0,"status":status})
    );
    std::io::stdout().flush().map_err(|_| HostError::Closed)
}
#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("foxbot-host: {e}");
        std::process::exit(1);
    }
}
async fn run() -> foxbot_host::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.is_empty() || matches!(args[0].as_str(), "help" | "--help") {
        println!(
            "foxbot-host run CONFIG STATE --allow-network [--allow-plaintext-synthetic]\n\
                  Starts PAUSED. JSON stdin: resume, message, status, pause, stop. EOF/Ctrl-C stops.\n\
                  foxbot-host init-key NAME --confirm-keychain-write\n\
                  foxbot-host set-token NAME --confirm-keychain-write  (secret on stdin, never argv)\n\
                  foxbot-host keychain-smoke --allow-keychain-test\n\
                  foxbot-host native-read-probe WORKER --allow-native-read\n\
                  foxbot-host bridge-sim-probe STATE --allow-plaintext-synthetic\n\
                  Test only: foxbot-host lock-probe --hold"
        );
        return Ok(());
    }
    if args[0] == "lock-probe" && args.len() == 2 && args[1] == "--hold" {
        let _owner = DeviceOwner::acquire()?;
        println!("LOCKED");
        std::io::stdout().flush().map_err(|_| HostError::Closed)?;
        let mut one = [0];
        let _ = std::io::stdin().read_exact(&mut one);
        return Ok(());
    }
    if args[0] == "keychain-smoke" && args.len() == 2 && args[1] == "--allow-keychain-test" {
        credentials::native_smoke()?;
        println!("{{\"keychain_roundtrip\":true,\"ephemeral_item_deleted\":true}}");
        return Ok(());
    }
    if args[0] == "native-read-probe" {
        if args.len() != 3 || args[2] != "--allow-native-read" {
            return Err(HostError::Config);
        }
        #[cfg(not(target_os = "macos"))]
        return Err(HostError::Unsupported);
        #[cfg(target_os = "macos")]
        {
            let _owner = DeviceOwner::acquire()?;
            let worker = NativeReadHost::start(NativeWorkerConfig {
                binary: PathBuf::from(&args[1]),
                warmup_timeout: Duration::from_secs(60),
                request_timeout: Duration::from_secs(15),
                queue_capacity: 2,
            })?;
            let handle = worker.handle();
            let warmed = handle.resume()?;
            let first = handle.snapshot()?;
            let snapshot = handle.snapshot()?;
            if first.conversation_fingerprint != snapshot.conversation_fingerprint
                || first.application_session_fingerprint != snapshot.application_session_fingerprint
            {
                handle.pause()?;
                return Err(HostError::Untrusted);
            }
            let status = handle.status()?;
            handle.pause()?;
            println!(
                "{}",
                serde_json::json!({
                    "schema_version":"foxbot.native-read-probe.v1",
                    "status":"SNAPSHOT_RECEIVED",
                    "read_only":true,
                    "raw_text_included":false,
                    "image_saved":false,
                    "write_or_send_operations":0,
                    "application_session_state":"STABLE_TWO_READS",
                    "conversation_fingerprint_state":"STABLE_TWO_READS",
                    "strategy":snapshot.strategy,
                    "message_count":snapshot.messages.len(),
                    "partial_reasons":snapshot.partial_reasons,
                    "worker":{
                        "warmed":warmed.warmed,
                        "starts":status.starts,
                        "successful_snapshots":status.successful_snapshots,
                        "failures":status.failures
                    },
                    "observation_bridge":"PROVISIONAL_REQUIRES_CONFIGURED_IDENTITY_AND_ACCEPTED_GROUND_TRUTH"
                })
            );
            return Ok(());
        }
    }
    if args[0] == "bridge-sim-probe" {
        if args.len() != 3 || args[2] != "--allow-plaintext-synthetic" {
            return Err(HostError::Config);
        }
        let state = PathBuf::from(&args[1]);
        if state.exists() {
            return Err(HostError::Config);
        }
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&state).map_err(|_| HostError::Storage)?;

        let mut binding = Binding::paused(fixture_key());
        binding.enabled = true;
        binding.quiet_ms = 0;
        binding.max_wait_ms = 0;
        let first = PrivateMessageSnapshot {
            schema_version: "foxbot.private-message-snapshot.v1".into(),
            strategy: "WECHAT_HEURISTIC_V0".into(),
            application_session_fingerprint: "c".repeat(64),
            conversation_fingerprint: "a".repeat(64),
            partial_reasons: vec!["HEURISTIC_REGION".into()],
            messages: vec![
                PrivateBridgeMessage {
                    text: "synthetic-A".into(),
                    direction: PrivateDirection::Them,
                    sender_fingerprint: Some("b".repeat(64)),
                    complete: true,
                },
                PrivateBridgeMessage {
                    text: "synthetic-B".into(),
                    direction: PrivateDirection::Me,
                    sender_fingerprint: None,
                    complete: true,
                },
                PrivateBridgeMessage {
                    text: "synthetic-C".into(),
                    direction: PrivateDirection::Them,
                    sender_fingerprint: Some("b".repeat(64)),
                    complete: true,
                },
            ],
        };
        let configured = NativeConversationBinding::from_current_snapshot(&first, binding.clone())?;
        let acceptance = GroundTruthAcceptance {
            schema_version: "foxbot.g2c-ground-truth-result.v1".into(),
            strategy: "WECHAT_HEURISTIC_V0".into(),
            revision: "g2d-synthetic-v1".into(),
            accepted: true,
            cases: 6,
            labeled_messages: 24,
            covered_tags: vec![
                "private".into(),
                "group".into(),
                "duplicate_text".into(),
                "numeric".into(),
                "multiline".into(),
                "reference".into(),
            ],
            direction_errors: 0,
            sender_errors: 0,
            message_count_errors: 0,
            text_errors: 0,
        };
        let mut bridge = NativeObservationBridge::new(vec![configured], acceptance)?;
        let mut runtime = Runtime::open_simulation(&state)?;
        runtime.bind(&binding)?;
        runtime.set_host_paused(false)?;
        let baseline = bridge.ingest_into_runtime(&mut runtime, &first, 1)?;

        let mut second = first.clone();
        second.messages.remove(0);
        second.messages.push(PrivateBridgeMessage {
            text: "synthetic-D".into(),
            direction: PrivateDirection::Them,
            sender_fingerprint: Some("b".repeat(64)),
            complete: true,
        });
        let new = bridge.ingest_into_runtime(&mut runtime, &second, 2)?;
        let repeat = bridge.ingest_into_runtime(&mut runtime, &second, 3)?;
        let counts = runtime.host_counts()?;
        drop(runtime);
        std::fs::remove_dir_all(&state).map_err(|_| HostError::Storage)?;
        println!(
            "{}",
            serde_json::json!({
                "schema_version":"foxbot.g2d-bridge-smoke.v1",
                "status":"PASS",
                "baseline":baseline,
                "new":new,
                "repeat":repeat,
                "runtime":{
                    "messages":counts.0,
                    "tasks":counts.1,
                    "ready":counts.2,
                    "unresolved_sends":counts.3
                },
                "external_model_requests":0,
                "native_chat_operations":0,
                "write_or_send_operations":0
            })
        );
        return Ok(());
    }
    if matches!(args[0].as_str(), "init-key" | "set-token") {
        if args.len() != 3 || args[2] != "--confirm-keychain-write" {
            return Err(HostError::Config);
        }
        let _owner = DeviceOwner::acquire()?;
        let reference = CredentialRef {
            id: args[1].clone(),
        };
        reference.validate()?;
        let (purpose, secret) = if args[0] == "init-key" {
            ("ledger", credentials::generate_ledger_key()?)
        } else {
            let mut data = zeroize::Zeroizing::new(Vec::new());
            std::io::stdin()
                .take(8193)
                .read_to_end(&mut data)
                .map_err(|_| HostError::CredentialInvalid)?;
            let secret = Secret::new(data.to_vec())?;
            secret.token()?;
            ("token", secret)
        };
        NativeCredentials.create(&reference, purpose, &secret)?;
        println!("{{\"credential_created\":true}}");
        return Ok(());
    }
    if args[0] != "run"
        || args.len() < 4
        || args.len() > 5
        || !args[3..].contains(&"--allow-network".to_owned())
        || args[3..]
            .iter()
            .any(|v| v != "--allow-network" && v != "--allow-plaintext-synthetic")
    {
        return Err(HostError::Config);
    }
    let path = PathBuf::from(&args[1]);
    let meta = std::fs::symlink_metadata(&path).map_err(|_| HostError::Config)?;
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > 131072 {
        return Err(HostError::Config);
    }
    let config: HostConfig =
        serde_json::from_slice(&std::fs::read(path).map_err(|_| HostError::Config)?)
            .map_err(|_| HostError::Config)?;
    config.validate()?;
    let owner = DeviceOwner::acquire()?;
    let allow_plain = args[3..].contains(&"--allow-plaintext-synthetic".to_owned());
    let (runtime, service) =
        config.open(&PathBuf::from(&args[2]), &NativeCredentials, allow_plain)?;
    let channels = SyntheticChannels::new(&config.bindings)?;
    let (host, handle) = Host::new(runtime, service, config.clone(), channels, owner)?;
    let mut worker = tokio::spawn(host.run());
    if emit("started_paused", None).is_err() {
        handle.stop();
        let _ = worker.await;
        return Err(HostError::Closed);
    }
    let (tx, mut rx) = mpsc::channel::<Option<Command>>(8);
    // Only this input reader may remain blocked on stdin. It has no runtime, lock,
    // credential or network capability and terminates with the CLI process on stop.
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut reader = stdin.lock();
        loop {
            let mut line = Vec::new();
            let read = (&mut reader).take(32769).read_until(b'\n', &mut line);
            match read {
                Ok(0) | Err(_) => break,
                _ => {}
            }
            if line.len() > 32768 {
                let _ = tx.blocking_send(None);
                break;
            }
            let command = serde_json::from_slice(&line).ok();
            let stop = matches!(command, Some(Command::Stop));
            if tx.blocking_send(command).is_err() || stop {
                break;
            }
        }
    });
    let clock = RunClock::default();
    loop {
        tokio::select! {
            _=tokio::signal::ctrl_c()=>break,
            result=&mut worker=>{
                let status=result.map_err(|_|HostError::Storage)??;return emit("stopped",Some(&status));
            },
            command=rx.recv()=>{
                let Some(command)=command else {break;};
                let result=match command {
                    Some(Command::Stop)=>break,
                    Some(Command::Resume)=>handle.resume().await,
                    Some(Command::Pause)=>handle.pause().await,
                    Some(Command::Status)=>handle.status().await,
                    Some(Command::Message{session,id,text,historical})=>{
                        if let Some(b)=config.bindings.get(session) {
                            let mut o=fixture_observation(&b.key,&id,&text,clock.now_ms());o.identity_epoch=b.identity_epoch;o.historical=historical;
                            match handle.observe(o).await {Ok(())=>handle.status().await,Err(e)=>Err(e)}
                        } else {Err(HostError::Config)}
                    },
                    None=>Err(HostError::Config),
                };
                let output=match result {Ok(s)=>emit("status",Some(&s)),Err(_)=>emit("command_rejected",None)};
                if output.is_err() {break;}
            }
        }
    }
    rx.close();
    handle.stop();
    let status = worker.await.map_err(|_| HostError::Storage)??;
    emit("stopped", Some(&status))
}
