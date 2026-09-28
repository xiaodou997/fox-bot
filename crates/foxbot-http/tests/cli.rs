mod support;
use foxbot_core::*;
use foxbot_http::*;
use std::{
    io::Read,
    process::{Child, Command, Output, Stdio},
    time::Duration,
};
use support::*;

const BIN: &str = env!("CARGO_BIN_EXE_foxbot-http");
struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn invoke(command: &str, config: &std::path::Path, state: &std::path::Path, allow: bool) -> Output {
    let mut cmd = Command::new(BIN);
    cmd.args([command])
        .arg(config)
        .arg(state)
        .env_remove("FOXBOT_HTTP_TOKEN");
    if allow {
        cmd.arg("--allow-network");
    }
    cmd.output().unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_real_http_to_mock_send_is_replay_safe_and_requires_network_opt_in() {
    let server = Server::start().await;
    let dir = private_dir();
    let config = dir.path().join("provider.json");
    let state = dir.path().join("state");
    std::fs::write(
        &config,
        serde_json::to_vec(&server.config(Protocol::BusinessV1, ContextMode::ServiceManaged))
            .unwrap(),
    )
    .unwrap();
    let rejected = invoke("synthetic-run", &config, &state, false);
    assert!(!rejected.status.success());
    assert!(server.state.lock().unwrap().seen.is_empty());
    let first = invoke("synthetic-run", &config, &state, true);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let first: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    let second = invoke("synthetic-run", &config, &state, true);
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    let second: serde_json::Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_eq!(first["provider_jobs_this_run"], 1);
    assert_eq!(first["mock_send_calls_this_run"], 1);
    assert_eq!(second["provider_jobs_this_run"], 0);
    assert_eq!(second["mock_send_calls_this_run"], 0);
    assert_eq!(second["feedback_queue"]["acked"], 1);
    assert_eq!(server.state.lock().unwrap().generations.len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn killed_http_process_recovers_with_cancellation_receipt_and_no_chat_send() {
    let server = Server::start().await;
    server.state.lock().unwrap().generate_delay_ms = 4000;
    let dir = private_dir();
    let config_path = dir.path().join("provider.json");
    let state = dir.path().join("state");
    let mut config = server.config(Protocol::BusinessV1, ContextMode::ServiceManaged);
    // This case tests process death AFTER request arrival, not HTTP timeout. Give
    // process scheduling its own budget; do not let a 1s transport retry race the kill.
    config.attempt_timeout_ms = 10_000;
    config.total_timeout_ms = 10_000;
    config.max_attempts = 1;
    std::fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    let mut child = ChildGuard(
        Command::new(BIN)
            .arg("synthetic-run")
            .arg(&config_path)
            .arg(&state)
            .arg("--allow-network")
            .env_remove("FOXBOT_HTTP_TOKEN")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if !server.state.lock().unwrap().seen.is_empty() {
                break;
            }
            if let Some(status) = child.0.try_wait().unwrap() {
                let mut diagnostic = String::new();
                child
                    .0
                    .stderr
                    .take()
                    .unwrap()
                    .read_to_string(&mut diagnostic)
                    .unwrap();
                panic!("HTTP child exited before request arrival ({status}): {diagnostic}");
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("HTTP child startup did not reach the request-arrival checkpoint");
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    let service = HttpReplyService::new(config, None).unwrap();
    let mut runtime = Runtime::open_simulation(&state).unwrap();
    let clock = RunClock::default();
    feedback(&service, &mut runtime, clock.now_ms())
        .await
        .unwrap();
    assert!(runtime.summary().unwrap().actions.is_empty());
    let s = server.state.lock().unwrap();
    assert_eq!(s.generations.len(), 1);
    assert_eq!(s.committed_turns, 0);
    assert_eq!(
        s.receipts.values().next().unwrap()["disposition"],
        "cancelled"
    );
}
