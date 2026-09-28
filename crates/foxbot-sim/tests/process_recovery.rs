use foxbot_core::{ActionState, Runtime};
use std::{
    io::{BufRead, BufReader},
    path::Path,
    process::{Child, Command, Output, Stdio},
};

const BIN: &str = env!("CARGO_BIN_EXE_foxbot-sim");

fn private_dir() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    directory
}

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        // Confirm process exit BEFORE the test's TempDir may remove its files.
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start_at_checkpoint(command: &str, path: &Path, expected: &str) -> ChildGuard {
    let mut child = ChildGuard(
        Command::new(BIN)
            .arg(command)
            .arg(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let mut output = BufReader::new(child.0.stdout.take().unwrap());
    let (sender, receiver) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut marker = String::new();
        let result = output.read_line(&mut marker).map(|_| marker);
        let _ = sender.send(result);
    });
    let marker = receiver
        .recv_timeout(std::time::Duration::from_secs(15))
        .expect("child did not reach checkpoint before deadline")
        .unwrap();
    reader.join().unwrap();
    assert_eq!(marker.trim(), expected);
    child
}

fn run(command: &str, path: &Path) -> Output {
    let output = Command::new(BIN).arg(command).arg(path).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

#[test]
fn another_process_cannot_own_state_and_kill_releases_lock() {
    let dir = private_dir();
    let mut child = start_at_checkpoint("hold-lock", dir.path(), "LOCKED");
    assert!(matches!(
        Runtime::open_simulation(dir.path()),
        Err(foxbot_core::Error::Busy)
    ));
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    assert!(Runtime::open_simulation(dir.path()).is_ok());
}

#[test]
fn killed_before_external_send_is_unknown_not_automatically_retried() {
    let dir = private_dir();
    let mut child = start_at_checkpoint("pause-before-send", dir.path(), "EXECUTING_BEFORE_SEND");
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    let runtime = Runtime::open_simulation(dir.path()).unwrap();
    assert_eq!(
        runtime.summary().unwrap().actions[0].1,
        ActionState::Unknown
    );
    assert!(!dir.path().join("synthetic-outgoing.jsonl").exists());
    drop(runtime);
    run("dispatch-prepared", dir.path());
    run("reconcile", dir.path());
    let runtime = Runtime::open_simulation(dir.path()).unwrap();
    assert_eq!(
        runtime.summary().unwrap().actions[0].1,
        ActionState::Unknown
    );
    assert!(!dir.path().join("synthetic-outgoing.jsonl").exists());
}

#[test]
fn killed_after_external_send_reconciles_without_a_second_send() {
    let dir = private_dir();
    let mut child =
        start_at_checkpoint("pause-after-send", dir.path(), "EXTERNAL_EFFECT_COMMITTED");
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    let runtime = Runtime::open_simulation(dir.path()).unwrap();
    let (action, state) = runtime.summary().unwrap().actions[0].clone();
    assert_eq!(state, ActionState::Unknown);
    assert_eq!(
        runtime.transition_history(&action).unwrap(),
        ["PREPARED", "EXECUTING", "UNKNOWN"]
    );
    drop(runtime);
    run("dispatch-prepared", dir.path());
    run("reconcile", dir.path());
    let runtime = Runtime::open_simulation(dir.path()).unwrap();
    assert_eq!(
        runtime.action(&action).unwrap().1,
        ActionState::VerifiedOutgoing
    );
    let sent = std::fs::read_to_string(dir.path().join("synthetic-outgoing.jsonl")).unwrap();
    assert_eq!(sent.lines().count(), 1);
}

#[test]
fn prepared_is_durable_and_can_resume_with_fresh_mock_preflight() {
    let dir = private_dir();
    run("prepare", dir.path());
    {
        let runtime = Runtime::open_simulation(dir.path()).unwrap();
        assert_eq!(
            runtime.summary().unwrap().actions[0].1,
            ActionState::Prepared
        );
    }
    run("dispatch-prepared", dir.path());
    let runtime = Runtime::open_simulation(dir.path()).unwrap();
    assert_eq!(
        runtime.summary().unwrap().actions[0].1,
        ActionState::VerifiedOutgoing
    );
}

#[test]
fn demo_replay_has_zero_extra_provider_or_send_calls() {
    let dir = private_dir();
    let first: serde_json::Value = serde_json::from_slice(&run("demo", dir.path()).stdout).unwrap();
    let second: serde_json::Value =
        serde_json::from_slice(&run("demo", dir.path()).stdout).unwrap();
    assert_eq!(first["provider_calls_this_run"], 1);
    assert_eq!(first["send_calls_this_run"], 1);
    assert_eq!(second["provider_calls_this_run"], 0);
    assert_eq!(second["send_calls_this_run"], 0);
    assert_eq!(second["synthetic_outgoing_count"], 1);
}
