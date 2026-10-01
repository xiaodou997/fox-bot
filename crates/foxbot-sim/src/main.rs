//! Offline, synthetic-only CLI. No chat application or network is accessed.
use foxbot_core::{simulation::*, *};
use std::{
    env,
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process,
};

fn main() {
    if let Err(error) = run() {
        // Display is intentionally redacted; do not log message payloads or backend errors.
        eprintln!("foxbot-sim: {error}");
        process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut args = env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "help".into());
    if matches!(command.as_str(), "help" | "--help" | "-h") {
        println!(
            "foxbot-sim <demo|inspect|prepare|gate-only|dispatch-prepared|reconcile> [STATE_DIR]\n\
            Synthetic data only; default STATE_DIR=.foxbot-sim.\n\
            Internal process tests: hold-lock, pause-before-send, pause-after-send."
        );
        return Ok(());
    }
    if !matches!(
        command.as_str(),
        "demo"
            | "inspect"
            | "prepare"
            | "gate-only"
            | "dispatch-prepared"
            | "reconcile"
            | "hold-lock"
            | "pause-before-send"
            | "pause-after-send"
    ) {
        return Err(Error::Invalid("command"));
    }
    let directory = PathBuf::from(args.next().unwrap_or_else(|| ".foxbot-sim".into()));
    if args.next().is_some() {
        return Err(Error::Invalid("arguments"));
    }
    let mut runtime = Runtime::open_simulation(&directory)?;
    if command == "hold-lock" {
        checkpoint("LOCKED")?;
        return Ok(());
    }
    if command == "inspect" {
        println!("{}", serde_json::to_string_pretty(&runtime.summary()?)?);
        return Ok(());
    }
    let key = fixture_key();
    let mut channel = JournalChannel::new(&key, &directory, &command)?;
    if command == "reconcile" || command == "dispatch-prepared" {
        for (action, state) in runtime.summary()?.actions {
            if command == "reconcile"
                && matches!(state, ActionState::Unknown | ActionState::Submitted)
            {
                runtime.reconcile(&action, &mut channel)?;
            } else if command == "dispatch-prepared" && state == ActionState::Prepared {
                runtime.dispatch(&action, 3_000, &mut channel)?;
            }
        }
        report(&runtime, 0, &channel)?;
        return Ok(());
    }
    let mut binding = Binding::paused(key.clone());
    binding.enabled = true; // The command explicitly enables this SYNTHETIC account only.
    runtime.bind(&binding)?;
    let mut history = fixture_observation(&key, "history-001", "合成历史：这条不能触发回复", 0);
    history.historical = true;
    runtime.ingest(&history)?;
    let incoming = fixture_observation(&key, "incoming-001", "合成测试：如何安装？", 1_000);
    runtime.ingest(&incoming)?;
    let mut second_source = incoming.clone();
    second_source.source = Source::UiTree;
    second_source.source_event_id = "capture-001".into();
    runtime.ingest(&second_source)?;
    let mut provider = FixedReply::new("模拟回复：请按测试说明安装。此消息不会发送到真实软件。");
    if let Some(request) = runtime.generate_once(&key, 3_000, None, &mut provider)? {
        let action = runtime.prepare_send(&request, 3_000, false)?;
        if command == "gate-only" {
            let before = channel.inner.live.clone();
            let fill_gate = runtime.preview_before_fill_gate(&action, 3_000, &before)?;
            let (payload, state_before) = runtime.action(&action)?;
            let mut after = before.clone();
            after.draft = Draft::Text(payload.text.clone());
            let send_gate = runtime.preview_before_send_gate(&action, 3_000, &before, &after)?;
            let (_, state_after) = runtime.action(&action)?;
            if !fill_gate.allowed
                || !send_gate.allowed
                || state_before != ActionState::Prepared
                || state_after != ActionState::Prepared
                || channel.inner.fill_calls != 0
                || channel.inner.send_calls != 0
            {
                return Err(Error::Blocked("safe send gate smoke"));
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "simulation_only": true,
                    "status": "SAFE_SEND_GATE_PASS",
                    "before_fill": fill_gate,
                    "before_send": send_gate,
                    "action_state": state_after,
                    "fill_calls": channel.inner.fill_calls,
                    "send_calls": channel.inner.send_calls,
                    "synthetic_outgoing_count": channel.inner.sent.len()
                }))?
            );
            return Ok(());
        }
        if command != "prepare" {
            runtime.dispatch(&action, 3_000, &mut channel)?;
        }
    }
    report(&runtime, provider.calls, &channel)
}

fn report(runtime: &Runtime, provider_calls: usize, channel: &JournalChannel) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "simulation_only": true, "provider_calls_this_run": provider_calls,
            "send_calls_this_run": channel.inner.send_calls,
            "synthetic_outgoing_count": channel.inner.sent.len(), "ledger": runtime.summary()?
        }))?
    );
    Ok(())
}

/// The parent process can kill and wait for this child after receiving the marker.
/// On stdin EOF the child returns, so abandoning the parent does not leave a worker.
fn checkpoint(marker: &str) -> Result<()> {
    println!("{marker}");
    io::stdout().flush()?;
    let mut byte = [0];
    match io::stdin().read_exact(&mut byte) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => Ok(()),
        Err(error) => Err(error.into()),
    }
}

struct JournalChannel {
    inner: MockChannel,
    journal: PathBuf,
    checkpoint: String,
}

impl JournalChannel {
    fn new(key: &ConversationKey, directory: &Path, checkpoint: &str) -> Result<Self> {
        let journal = directory.join("synthetic-outgoing.jsonl");
        let mut inner = MockChannel::new(key);
        if journal.exists() {
            let metadata = fs::symlink_metadata(&journal)?;
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err(Error::UnsafeState);
            }
            let text = fs::read_to_string(&journal)?;
            for line in text.lines() {
                inner.sent.push(serde_json::from_str::<String>(line)?);
            }
        }
        Ok(Self {
            inner,
            journal,
            checkpoint: checkpoint.into(),
        })
    }
}

impl MessageChannel for JournalChannel {
    fn inspect(&mut self, target: &ConversationKey) -> Result<LiveTarget> {
        self.inner.inspect(target)
    }
    fn fill(&mut self, action: &OutboundAction, expected: &LiveTarget) -> Result<()> {
        if self.checkpoint == "pause-before-send" {
            checkpoint("EXECUTING_BEFORE_SEND")?;
            return Err(Error::Blocked("test checkpoint released"));
        }
        self.inner.fill(action, expected)
    }
    fn send(&mut self, action: &OutboundAction, expected: &LiveTarget) -> Result<SendEvidence> {
        let evidence = self.inner.send(action, expected)?;
        let mut options = OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&self.journal)?;
        writeln!(file, "{}", serde_json::to_string(&action.action_id)?)?;
        file.sync_all()?; // Durable simulated external effect, independent of the ledger.
        if self.checkpoint == "pause-after-send" {
            checkpoint("EXTERNAL_EFFECT_COMMITTED")?;
            return Ok(SendEvidence::Unknown);
        }
        Ok(evidence)
    }
    fn reconcile(&mut self, action: &OutboundAction) -> Result<SendEvidence> {
        self.inner.reconcile(action)
    }
}
