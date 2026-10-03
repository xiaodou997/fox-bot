use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};
const BIN: &str = env!("CARGO_BIN_EXE_foxbot-host");
struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn isolated_command(home: &std::path::Path) -> Command {
    let mut command = Command::new(BIN);
    command
        .env("HOME", home)
        .env("XDG_DATA_HOME", home.join("data"));
    command
}
fn spawn(home: &std::path::Path, args: &[&str]) -> (ChildGuard, mpsc::Receiver<String>) {
    let mut child = ChildGuard(
        isolated_command(home)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let stdout = child.0.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if tx.send(line.unwrap()).is_err() {
                break;
            }
        }
    });
    (child, rx)
}
fn wait_exit(child: &mut Child) -> std::process::ExitStatus {
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(s) = child.try_wait().unwrap() {
            return s;
        }
        assert!(Instant::now() < end, "host did not exit");
        std::thread::sleep(Duration::from_millis(5));
    }
}
#[test]
fn separate_processes_share_device_lock_across_ledgers_and_stop_with_stdin_open() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("config.json");
    std::fs::write(&cfg, include_str!("../../../examples/host-synthetic.json")).unwrap();
    let home = dir.path().join("isolated-home");
    std::fs::create_dir(&home).unwrap();
    let (mut owner, lines) = spawn(&home, &["lock-probe", "--hold"]);
    assert_eq!(
        lines.recv_timeout(Duration::from_secs(3)).unwrap(),
        "LOCKED"
    );
    for name in ["ledger-a", "ledger-b"] {
        let path = dir.path().join(name);
        let result = isolated_command(&home)
            .args([
                "run",
                cfg.to_str().unwrap(),
                path.to_str().unwrap(),
                "--allow-network",
                "--allow-plaintext-synthetic",
            ])
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains("busy"));
        assert!(!path.exists());
    }
    owner.0.kill().unwrap();
    owner.0.wait().unwrap();
    let path = dir.path().join("after-release");
    let (mut host, lines) = spawn(
        &home,
        &[
            "run",
            cfg.to_str().unwrap(),
            path.to_str().unwrap(),
            "--allow-network",
            "--allow-plaintext-synthetic",
        ],
    );
    let started: serde_json::Value =
        serde_json::from_str(&lines.recv_timeout(Duration::from_secs(3)).unwrap()).unwrap();
    assert_eq!(started["event"], "started_paused");
    host.0
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"{\"command\":\"status\"}\n")
        .unwrap();
    host.0.stdin.as_mut().unwrap().flush().unwrap();
    let status: serde_json::Value =
        serde_json::from_str(&lines.recv_timeout(Duration::from_secs(3)).unwrap()).unwrap();
    assert_eq!(status["status"]["paused"], true);
    host.0
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"{\"command\":\"stop\"}\n")
        .unwrap();
    host.0.stdin.as_mut().unwrap().flush().unwrap();
    // Keep stdin open: the stop command, not EOF, must terminate the process.
    assert!(wait_exit(&mut host.0).success());
    let stopped: serde_json::Value =
        serde_json::from_str(&lines.recv_timeout(Duration::from_secs(3)).unwrap()).unwrap();
    assert_eq!(stopped["event"], "stopped");
    assert_eq!(stopped["status"]["active_jobs"], 0);
    assert_eq!(stopped["status"]["active_feedback"], 0);
}
