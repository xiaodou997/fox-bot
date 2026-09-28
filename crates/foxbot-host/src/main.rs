//! Development host: stdin commands and synthetic senders; never operates a real chat app.
use foxbot_core::{simulation::*, *};
use foxbot_host::{
    credentials::{CredentialRef, CredentialStore, NativeCredentials, Secret},
    ownership::DeviceOwner,
    *,
};
use foxbot_http::RunClock;
use std::{
    collections::HashMap,
    io::{BufRead, Read, Write},
    path::PathBuf,
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
