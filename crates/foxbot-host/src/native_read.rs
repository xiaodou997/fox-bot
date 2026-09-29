use crate::{HostError, Result, native_bridge::PrivateMessageSnapshot};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    thread,
    time::Duration,
};

const MAX_COMMAND_BYTES: usize = 4096;
const MAX_REPLY_BYTES: usize = 65_536;

#[derive(Clone, Debug)]
pub struct NativeWorkerConfig {
    pub binary: PathBuf,
    pub warmup_timeout: Duration,
    pub request_timeout: Duration,
    pub queue_capacity: usize,
}
impl NativeWorkerConfig {
    pub fn validate(&self) -> Result<()> {
        let metadata = fs::symlink_metadata(&self.binary).map_err(|_| HostError::Config)?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || !(1..=8).contains(&self.queue_capacity)
            || !(Duration::from_secs(5)..=Duration::from_secs(120)).contains(&self.warmup_timeout)
            || !(Duration::from_secs(1)..=Duration::from_secs(30)).contains(&self.request_timeout)
        {
            return Err(HostError::Config);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
pub struct NativeWorkerStatus {
    pub paused: bool,
    pub warmed: bool,
    pub starts: u64,
    pub successful_snapshots: u64,
    pub failures: u64,
}

#[derive(Serialize)]
struct WorkerRequest<'a> {
    id: &'a str,
    command: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    app: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    focused_window: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Warmup {
    elapsed_milliseconds: u64,
    line_count: u32,
    succeeded: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkerReply {
    id: String,
    status: String,
    #[serde(default)]
    warmup: Option<Warmup>,
    #[serde(default)]
    private_snapshot: Option<PrivateMessageSnapshot>,
    #[serde(default)]
    report: Option<serde_json::Value>,
}

enum ReaderEvent {
    Line(Vec<u8>),
    Invalid,
    Closed,
}

struct WorkerProcess {
    child: Child,
    stdin: ChildStdin,
    replies: Receiver<ReaderEvent>,
    reader: Option<thread::JoinHandle<()>>,
    next_id: u64,
}
impl WorkerProcess {
    fn spawn(binary: &Path) -> Result<Self> {
        let mut child = Command::new(binary)
            .arg("--worker")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| HostError::NativeWorker)?;
        let stdin = child.stdin.take().ok_or(HostError::NativeWorker)?;
        let stdout = child.stdout.take().ok_or(HostError::NativeWorker)?;
        let (tx, replies) = mpsc::channel();
        let reader = thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = Vec::new();
                match reader
                    .by_ref()
                    .take((MAX_REPLY_BYTES + 2) as u64)
                    .read_until(b'\n', &mut line)
                {
                    Ok(0) => {
                        let _ = tx.send(ReaderEvent::Closed);
                        break;
                    }
                    Ok(_) if line.len() > MAX_REPLY_BYTES + 1 || !line.ends_with(b"\n") => {
                        let _ = tx.send(ReaderEvent::Invalid);
                        break;
                    }
                    Ok(_) => {
                        line.pop();
                        if tx.send(ReaderEvent::Line(line)).is_err() {
                            break;
                        }
                    }
                    Err(_) => {
                        let _ = tx.send(ReaderEvent::Closed);
                        break;
                    }
                }
            }
        });
        Ok(Self {
            child,
            stdin,
            replies,
            reader: Some(reader),
            next_id: 1,
        })
    }

    fn request(
        &mut self,
        command: &str,
        app: Option<&str>,
        focused: Option<bool>,
        timeout: Duration,
    ) -> Result<WorkerReply> {
        if self
            .child
            .try_wait()
            .map_err(|_| HostError::NativeWorker)?
            .is_some()
        {
            return Err(HostError::NativeWorker);
        }
        let id = format!("host_{}", self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        let data = serde_json::to_vec(&WorkerRequest {
            id: &id,
            command,
            app,
            focused_window: focused,
        })
        .map_err(|_| HostError::NativeWorker)?;
        if data.len() > MAX_COMMAND_BYTES {
            return Err(HostError::NativeWorker);
        }
        self.stdin
            .write_all(&data)
            .and_then(|_| self.stdin.write_all(b"\n"))
            .and_then(|_| self.stdin.flush())
            .map_err(|_| HostError::NativeWorker)?;
        let event = self
            .replies
            .recv_timeout(timeout)
            .map_err(|_| HostError::NativeWorker)?;
        let ReaderEvent::Line(line) = event else {
            return Err(HostError::NativeWorker);
        };
        let reply: WorkerReply =
            serde_json::from_slice(&line).map_err(|_| HostError::NativeWorker)?;
        if reply.id != id {
            return Err(HostError::NativeWorker);
        }
        Ok(reply)
    }

    fn terminate(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}
impl Drop for WorkerProcess {
    fn drop(&mut self) {
        self.terminate();
    }
}

struct NativeWorkerManager {
    config: NativeWorkerConfig,
    process: Option<WorkerProcess>,
    status: NativeWorkerStatus,
}
impl NativeWorkerManager {
    fn new(config: NativeWorkerConfig) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            config,
            process: None,
            status: NativeWorkerStatus {
                paused: true,
                ..Default::default()
            },
        })
    }

    fn start_and_warm(&mut self) -> Result<()> {
        self.stop_process();
        let mut process = WorkerProcess::spawn(&self.config.binary)?;
        self.status.starts = self.status.starts.saturating_add(1);
        let reply = process.request("warmup", None, None, self.config.warmup_timeout);
        match reply {
            Ok(reply)
                if reply.status == "WARMED"
                    && reply.warmup.as_ref().is_some_and(|w| {
                        w.succeeded && w.line_count == 0 && w.elapsed_milliseconds <= 300_000
                    }) =>
            {
                self.status.warmed = true;
                self.process = Some(process);
                Ok(())
            }
            _ => {
                process.terminate();
                self.status.failures = self.status.failures.saturating_add(1);
                self.status.warmed = false;
                Err(HostError::NativeWorker)
            }
        }
    }

    fn resume(&mut self) -> Result<NativeWorkerStatus> {
        if self.process.is_none() || !self.status.warmed {
            self.start_and_warm()?;
        }
        self.status.paused = false;
        Ok(self.status.clone())
    }

    fn pause(&mut self) -> NativeWorkerStatus {
        self.status.paused = true;
        self.stop_process();
        self.status.clone()
    }

    fn snapshot(&mut self) -> Result<PrivateMessageSnapshot> {
        if self.status.paused {
            return Err(HostError::Paused);
        }
        if self.process.is_none() {
            self.start_and_warm()?;
        }
        let result = self
            .process
            .as_mut()
            .ok_or(HostError::NativeWorker)?
            .request(
                "capture_snapshot",
                Some("wechat"),
                Some(true),
                self.config.request_timeout,
            );
        match result {
            Ok(reply) if reply.status == "SNAPSHOT" => {
                if reply.report.is_none() {
                    self.status.failures = self.status.failures.saturating_add(1);
                    self.stop_process();
                    return Err(HostError::NativeWorker);
                }
                let snapshot = reply.private_snapshot.ok_or(HostError::NativeWorker)?;
                self.status.successful_snapshots =
                    self.status.successful_snapshots.saturating_add(1);
                Ok(snapshot)
            }
            _ => {
                self.status.failures = self.status.failures.saturating_add(1);
                self.stop_process();
                Err(HostError::NativeWorker)
            }
        }
    }

    fn stop_process(&mut self) {
        if let Some(mut process) = self.process.take() {
            let _ = process.request("shutdown", None, None, Duration::from_secs(2));
            process.terminate();
        }
        self.status.warmed = false;
    }
}

enum NativeCommand {
    Resume(mpsc::Sender<Result<NativeWorkerStatus>>),
    Pause(mpsc::Sender<NativeWorkerStatus>),
    Snapshot(mpsc::Sender<Result<PrivateMessageSnapshot>>),
    Status(mpsc::Sender<NativeWorkerStatus>),
    Stop,
}

#[derive(Clone)]
pub struct NativeReadHandle {
    tx: SyncSender<NativeCommand>,
    stopped: Arc<AtomicBool>,
    wait_timeout: Duration,
}
impl NativeReadHandle {
    fn send<T>(&self, command: NativeCommand, rx: Receiver<T>) -> Result<T> {
        if self.stopped.load(Ordering::Acquire) {
            return Err(HostError::Closed);
        }
        self.tx.try_send(command).map_err(|error| match error {
            TrySendError::Full(_) => HostError::Backpressure,
            TrySendError::Disconnected(_) => HostError::Closed,
        })?;
        rx.recv_timeout(self.wait_timeout)
            .map_err(|_| HostError::Closed)
    }

    pub fn resume(&self) -> Result<NativeWorkerStatus> {
        let (tx, rx) = mpsc::channel();
        self.send(NativeCommand::Resume(tx), rx)?
    }
    pub fn pause(&self) -> Result<NativeWorkerStatus> {
        let (tx, rx) = mpsc::channel();
        self.send(NativeCommand::Pause(tx), rx)
    }
    pub fn snapshot(&self) -> Result<PrivateMessageSnapshot> {
        let (tx, rx) = mpsc::channel();
        self.send(NativeCommand::Snapshot(tx), rx)?
    }
    pub fn status(&self) -> Result<NativeWorkerStatus> {
        let (tx, rx) = mpsc::channel();
        self.send(NativeCommand::Status(tx), rx)
    }
    pub fn stop(&self) {
        if !self.stopped.swap(true, Ordering::AcqRel) {
            // Stop is lifecycle-critical and cannot be dropped merely because the
            // bounded work queue is momentarily full. Existing work remains bounded.
            let _ = self.tx.send(NativeCommand::Stop);
        }
    }
}

pub struct NativeReadHost {
    join: Option<thread::JoinHandle<()>>,
    handle: NativeReadHandle,
}
impl NativeReadHost {
    pub fn start(config: NativeWorkerConfig) -> Result<Self> {
        config.validate()?;
        let capacity = config.queue_capacity;
        let wait_timeout = config.warmup_timeout + config.request_timeout + Duration::from_secs(3);
        let (tx, rx) = mpsc::sync_channel(capacity);
        let stopped = Arc::new(AtomicBool::new(false));
        let thread_stopped = stopped.clone();
        let join = thread::spawn(move || {
            let Ok(mut manager) = NativeWorkerManager::new(config) else {
                thread_stopped.store(true, Ordering::Release);
                return;
            };
            while let Ok(command) = rx.recv() {
                match command {
                    NativeCommand::Resume(ack) => {
                        let _ = ack.send(manager.resume());
                    }
                    NativeCommand::Pause(ack) => {
                        let _ = ack.send(manager.pause());
                    }
                    NativeCommand::Snapshot(ack) => {
                        let _ = ack.send(manager.snapshot());
                    }
                    NativeCommand::Status(ack) => {
                        let _ = ack.send(manager.status.clone());
                    }
                    NativeCommand::Stop => break,
                }
            }
            manager.pause();
            thread_stopped.store(true, Ordering::Release);
        });
        let handle = NativeReadHandle {
            tx,
            stopped,
            wait_timeout,
        };
        Ok(Self {
            join: Some(join),
            handle,
        })
    }

    pub fn handle(&self) -> NativeReadHandle {
        self.handle.clone()
    }
}
impl Drop for NativeReadHost {
    fn drop(&mut self) {
        self.handle.stop();
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

static REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);
pub fn next_native_request_id() -> u64 {
    REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{os::unix::fs::PermissionsExt, sync::Barrier, time::Instant};

    fn worker_script(directory: &Path, slow: bool, crash_marker: Option<&Path>) -> PathBuf {
        let path = directory.join("fake-worker.py");
        let marker = crash_marker
            .map(|p| p.display().to_string())
            .unwrap_or_default()
            .replace('\\', "\\\\");
        let slow_literal = if slow { "True" } else { "False" };
        let fingerprint = "a".repeat(64);
        let app_session = "c".repeat(64);
        let sender = "b".repeat(64);
        let body = format!(
            r#"#!/usr/bin/env python3
import json, os, sys, time
marker = "{marker}"
for line in sys.stdin:
    req = json.loads(line)
    cid = req["id"]
    cmd = req["command"]
    if cmd == "warmup":
        out = {{"id":cid,"status":"WARMED","warmup":{{"elapsed_milliseconds":1,"line_count":0,"succeeded":True}}}}
    elif cmd == "capture_snapshot":
        if marker and not os.path.exists(marker):
            open(marker,"w").close()
            sys.exit(9)
        if {slow_literal}: time.sleep(0.5)
        out = {{"id":cid,"status":"SNAPSHOT","report":{{"status":"OCR_SUMMARY"}},"private_snapshot":{{
            "schema_version":"foxbot.private-message-snapshot.v1",
            "strategy":"WECHAT_HEURISTIC_V0",
            "application_session_fingerprint":"{app_session}",
            "conversation_fingerprint":"{fingerprint}",
            "partial_reasons":["HEURISTIC_REGION"],
            "messages":[{{"text":"synthetic","direction":"THEM","sender_fingerprint":"{sender}","complete":True}}]
        }}}}
    elif cmd == "shutdown":
        out = {{"id":cid,"status":"SHUTDOWN"}}
        print(json.dumps(out), flush=True)
        break
    else:
        out = {{"id":cid,"status":"INVALID_REQUEST"}}
    print(json.dumps(out), flush=True)
"#
        );
        fs::write(&path, body).unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&path, permissions).unwrap();
        path
    }

    fn config(binary: PathBuf, capacity: usize) -> NativeWorkerConfig {
        NativeWorkerConfig {
            binary,
            warmup_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(2),
            queue_capacity: capacity,
        }
    }

    #[test]
    fn host_starts_paused_resumes_reads_and_pause_kills_worker() {
        let dir = tempfile::tempdir().unwrap();
        let host =
            NativeReadHost::start(config(worker_script(dir.path(), false, None), 2)).unwrap();
        let handle = host.handle();
        assert!(handle.status().unwrap().paused);
        assert!(matches!(handle.snapshot(), Err(HostError::Paused)));
        let status = handle.resume().unwrap();
        assert!(status.warmed);
        let snapshot = handle.snapshot().unwrap();
        assert_eq!(snapshot.messages[0].text, "synthetic");
        let paused = handle.pause().unwrap();
        assert!(paused.paused);
        assert!(!paused.warmed);
        assert!(matches!(handle.snapshot(), Err(HostError::Paused)));
    }

    #[test]
    fn crash_is_fail_closed_and_next_read_restarts_and_warms() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("crashed");
        let host =
            NativeReadHost::start(config(worker_script(dir.path(), false, Some(&marker)), 2))
                .unwrap();
        let handle = host.handle();
        handle.resume().unwrap();
        assert!(matches!(handle.snapshot(), Err(HostError::NativeWorker)));
        let snapshot = handle.snapshot().unwrap();
        assert_eq!(snapshot.messages.len(), 1);
        let status = handle.status().unwrap();
        assert!(status.starts >= 2);
        assert!(status.failures >= 1);
    }

    #[test]
    fn bounded_command_queue_reports_backpressure_instead_of_unbounded_waiting() {
        let dir = tempfile::tempdir().unwrap();
        let host = NativeReadHost::start(config(worker_script(dir.path(), true, None), 1)).unwrap();
        let handle = host.handle();
        handle.resume().unwrap();
        let barrier = Arc::new(Barrier::new(2));
        let first = {
            let h = handle.clone();
            let b = barrier.clone();
            thread::spawn(move || {
                b.wait();
                h.snapshot()
            })
        };
        barrier.wait();
        thread::sleep(Duration::from_millis(50));
        let second = {
            let h = handle.clone();
            thread::spawn(move || h.snapshot())
        };
        thread::sleep(Duration::from_millis(50));
        let started = Instant::now();
        assert!(matches!(handle.snapshot(), Err(HostError::Backpressure)));
        assert!(started.elapsed() < Duration::from_millis(200));
        first.join().unwrap().unwrap();
        second.join().unwrap().unwrap();
    }
}
