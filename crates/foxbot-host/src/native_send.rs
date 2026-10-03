//! Bounded private IPC for an unattended, single-action macOS WeChat channel.
#[cfg(all(test, unix))]
#[path = "native_send_tests.rs"]
mod tests;
use crate::{
    HostError, Result, g2d_real, native_bridge::NativeConversationBinding, ownership::DeviceOwner,
};
use foxbot_core::{
    ConversationKey, Draft, LiveTarget, MessageChannel, OutboundAction, SendEvidence,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::Duration,
};

const RECEIPT_REVISION: &str = "WECHAT_RECEIPT_V3";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SendPoint {
    pub x: f64,
    pub y: f64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageSignature {
    pub digest: String,
    pub direction: String,
    pub complete: bool,
    #[serde(default)]
    pub continuity_digest: Option<String>,
}
impl MessageSignature {
    pub(crate) fn same_context(&self, other: &Self) -> bool {
        self.continuity_digest.is_some()
            && self.continuity_digest == other.continuity_digest
            && self.direction == other.direction
            && self.complete == other.complete
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeSendObservation {
    pub application_session: String,
    pub conversation: String,
    pub window_ref: String,
    pub layout_ref: String,
    pub frontmost: bool,
    pub conversation_resolved: bool,
    pub draft_state: String,
    pub draft_text: Option<String>,
    pub messages: Vec<MessageSignature>,
    pub send_button: Option<SendPoint>,
    /// Legacy receipt contexts decode as None, never upgraded implicitly.
    #[serde(default)]
    pub evidence_revision: Option<String>,
}
impl NativeSendObservation {
    pub(crate) fn same_messages(&self, other: &Self) -> bool {
        self.messages.len() == other.messages.len()
            && self
                .messages
                .iter()
                .zip(&other.messages)
                .all(|(a, b)| a.same_context(b))
    }
    pub(crate) fn validate(&self) -> Result<()> {
        fn hash(value: &str) -> bool {
            value.len() == 64
                && value
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
                && value.bytes().any(|b| b != b'0')
        }
        if self.evidence_revision.as_deref() != Some(RECEIPT_REVISION)
            || !hash(&self.application_session)
            || !hash(&self.conversation)
            || !hash(&self.layout_ref)
            || self.window_ref.is_empty()
            || self.window_ref.len() > 128
            || self.messages.len() > 64
            || self.messages.iter().any(|m| {
                !hash(&m.digest)
                    || !m.continuity_digest.as_deref().is_some_and(hash)
                    || !matches!(m.direction.as_str(), "ME" | "THEM" | "UNKNOWN")
            })
            || !matches!(
                self.draft_state.as_str(),
                "EMPTY_HEURISTIC" | "NONEMPTY" | "UNREADABLE"
            )
            || self.draft_text.as_ref().is_some_and(|s| s.len() > 16384)
            || self.send_button.as_ref().is_some_and(|p| {
                !p.x.is_finite()
                    || !p.y.is_finite()
                    || !(0.86..=1.0).contains(&p.x)
                    || !(0.80..=1.0).contains(&p.y)
            })
        {
            return Err(HostError::Untrusted);
        }
        Ok(())
    }
    fn matches_binding(&self, binding: &NativeConversationBinding) -> bool {
        self.conversation_resolved
            && self.application_session == binding.application_session_fingerprint
            && self.conversation == binding.conversation_fingerprint
    }
    pub(crate) fn same_surface(&self, other: &Self) -> bool {
        self.application_session == other.application_session
            && self.conversation == other.conversation
            && self.window_ref == other.window_ref
            && self.layout_ref == other.layout_ref
            && self.conversation_resolved
            && other.conversation_resolved
    }
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeReadMessage {
    pub text: String,
    pub direction: String,
    pub complete: bool,
}

pub(crate) struct NativeReadFrame {
    pub observation: NativeSendObservation,
    pub messages: Vec<NativeReadMessage>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkerReply {
    schema_version: String,
    id: u64,
    status: String,
    observation: Option<NativeSendObservation>,
    messages: Option<Vec<NativeReadMessage>>,
    write_attempted: bool,
    send_attempted: bool,
    verified_outgoing: bool,
}

struct Worker {
    child: Child,
    stdin: ChildStdin,
    replies: Receiver<Vec<u8>>,
    reader: Option<thread::JoinHandle<()>>,
    next_id: u64,
    timeout: Duration,
}
impl Worker {
    fn start(binary: &Path, allow_write: bool, timeout: Duration) -> Result<Self> {
        let metadata = fs::symlink_metadata(binary).map_err(|_| HostError::Config)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(HostError::Config);
        }
        let mut command = Command::new(binary);
        command.arg("--worker");
        if allow_write {
            command.arg("--allow-single-send");
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| HostError::NativeWorker)?;
        let stdin = child.stdin.take().ok_or(HostError::NativeWorker)?;
        let stdout = child.stdout.take().ok_or(HostError::NativeWorker)?;
        let (tx, replies) = mpsc::sync_channel(2);
        let reader = thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = Vec::new();
                match reader.by_ref().take(65_538).read_until(b'\n', &mut line) {
                    Ok(n) if n > 0 && line.len() <= 65_537 && line.ends_with(b"\n") => {
                        line.pop();
                        if tx.try_send(line).is_err() {
                            break;
                        }
                    }
                    _ => break,
                }
            }
        });
        let mut worker = Self {
            child,
            stdin,
            replies,
            reader: Some(reader),
            next_id: 1,
            timeout,
        };
        let warmed = worker.request("warmup", None, None)?;
        if warmed.status != "WARMED" || warmed.observation.is_some() {
            return Err(HostError::NativeWorker);
        }
        Ok(worker)
    }
    fn request(
        &mut self,
        command: &str,
        expected: Option<&NativeSendObservation>,
        action: Option<&OutboundAction>,
    ) -> Result<WorkerReply> {
        let result = self.request_inner(command, expected, action);
        if result.is_err() {
            self.terminate();
        }
        result
    }
    fn request_inner(
        &mut self,
        command: &str,
        expected: Option<&NativeSendObservation>,
        action: Option<&OutboundAction>,
    ) -> Result<WorkerReply> {
        if self
            .child
            .try_wait()
            .map_err(|_| HostError::NativeWorker)?
            .is_some()
        {
            return Err(HostError::NativeWorker);
        }
        let id = self.next_id;
        self.next_id += 1;
        let data = serde_json::to_vec(&serde_json::json!({
            "id": id, "command": command, "expected": expected,
            "text": action.map(|a| a.text.as_str()), "action_id": action.map(|a| a.action_id.as_str())
        })).map_err(|_| HostError::Config)?;
        if data.len() > 16_384 {
            return Err(HostError::Config);
        }
        self.stdin
            .write_all(&data)
            .and_then(|_| self.stdin.write_all(b"\n"))
            .and_then(|_| self.stdin.flush())
            .map_err(|_| HostError::NativeWorker)?;
        let data = self
            .replies
            .recv_timeout(self.timeout)
            .map_err(|_| HostError::NativeWorker)?;
        let reply: WorkerReply =
            serde_json::from_slice(&data).map_err(|_| HostError::NativeWorker)?;
        if reply.schema_version != "foxbot.native-send-worker.v4"
            || reply.id != id
            || (command != "read" && reply.messages.is_some())
            || (command != "fill" && reply.write_attempted)
            || (!matches!(command, "send" | "recover_send") && reply.send_attempted)
            || (reply.verified_outgoing
                && !matches!(command, "send" | "recover_send" | "reconcile"))
        {
            return Err(HostError::NativeWorker);
        }
        if let Some(observation) = &reply.observation {
            observation.validate()?;
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
impl Drop for Worker {
    fn drop(&mut self) {
        self.terminate();
    }
}

fn read_frame(reply: WorkerReply) -> Result<NativeReadFrame> {
    if reply.status != "OBSERVED" {
        return Err(HostError::Untrusted);
    }
    let observation = reply.observation.ok_or(HostError::Untrusted)?;
    let messages = reply.messages.ok_or(HostError::Untrusted)?;
    if !observation.frontmost
        || !observation.conversation_resolved
        || messages.len() != observation.messages.len()
        || messages.iter().zip(&observation.messages).any(|(m, s)| {
            m.text.trim().is_empty()
                || m.text.len() > 16_384
                || !m.complete
                || !s.complete
                || !matches!(m.direction.as_str(), "ME" | "THEM")
                || m.direction != s.direction
                || format!("{:x}", Sha256::digest(m.text.as_bytes())) != s.digest
                || Some(format!(
                    "{:x}",
                    Sha256::digest(crate::native_bridge::continuity_text(&m.text).as_bytes())
                )) != s.continuity_digest
        })
    {
        return Err(HostError::Untrusted);
    }
    Ok(NativeReadFrame {
        observation,
        messages,
    })
}

fn verified_draft_text(observed: Option<&str>, expected: &str) -> bool {
    let Some(observed) = observed else {
        return false;
    };
    if observed == expected {
        return true;
    }
    !expected.is_empty()
        && !expected.contains('\n')
        && expected.chars().all(|value| !value.is_ascii())
        && observed.len() == expected.len() + 1
        && observed.strip_suffix('1') == Some(expected)
}

fn supported_recovery_text(value: &str) -> bool {
    !value.is_empty()
        && value.encode_utf16().count() <= 80
        && value.trim() == value
        && !value.chars().any(char::is_control)
        && !value.ends_with('|')
}

/// Read two stable current frames before an operator-confirmed application-session rebind.
/// This path has no native write capability and intentionally does not assume the old binding.
pub(crate) fn read_unbound_pair(
    binary: &Path,
    owner: &DeviceOwner,
) -> Result<(NativeReadFrame, NativeReadFrame)> {
    owner.verify()?;
    let mut worker = Worker::start(binary, false, Duration::from_secs(60))?;
    let first = read_frame(worker.request("read", None, None)?)?;
    owner.verify()?;
    let second = read_frame(worker.request("read", None, None)?)?;
    owner.verify()?;
    Ok((first, second))
}

#[derive(Default, Serialize)]
pub struct NativeSendStats {
    pub read_requests: u32,
    pub inspect_requests: u32,
    pub fill_requests: u32,
    pub send_requests: u32,
    pub reconcile_requests: u32,
    pub write_attempted: Option<bool>,
    pub send_attempted: Option<bool>,
    pub last_status: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReceiptContext {
    action_id: String,
    text_digest: String,
    before: NativeSendObservation,
}

pub struct NativeSendChannel<'a> {
    worker: Worker,
    owner: &'a DeviceOwner,
    binding: NativeConversationBinding,
    last: Option<NativeSendObservation>,
    baseline: Option<NativeSendObservation>,
    receipt_path: PathBuf,
    recovery_text: Option<String>,
    pub stats: NativeSendStats,
}
impl<'a> NativeSendChannel<'a> {
    pub fn start(
        binary: &Path,
        binding: NativeConversationBinding,
        owner: &'a DeviceOwner,
        receipt_path: PathBuf,
        allow_write: bool,
    ) -> Result<Self> {
        owner.verify()?;
        Ok(Self {
            worker: Worker::start(binary, allow_write, Duration::from_secs(60))?,
            owner,
            binding,
            last: None,
            baseline: None,
            receipt_path,
            recovery_text: None,
            stats: NativeSendStats::default(),
        })
    }

    pub(crate) fn start_filled_recovery(
        binary: &Path,
        binding: NativeConversationBinding,
        owner: &'a DeviceOwner,
        receipt_path: PathBuf,
        text: &str,
    ) -> Result<Self> {
        if !supported_recovery_text(text) {
            return Err(HostError::Config);
        }
        let mut channel = Self::start(binary, binding, owner, receipt_path, true)?;
        channel.recovery_text = Some(text.to_owned());
        Ok(channel)
    }

    fn canonicalize_recovery_draft(
        &self,
        mut observation: NativeSendObservation,
    ) -> Result<NativeSendObservation> {
        if let Some(expected) = &self.recovery_text {
            if observation.draft_state != "NONEMPTY"
                || !verified_draft_text(observation.draft_text.as_deref(), expected)
            {
                return Err(HostError::Untrusted);
            }
            observation.draft_text = Some(expected.clone());
        }
        Ok(observation)
    }
    pub fn observe(&mut self) -> Result<NativeSendObservation> {
        self.owner.verify()?;
        self.stats.inspect_requests += 1;
        let reply = self.worker.request("inspect", None, None)?;
        self.stats.last_status = reply.status.clone();
        if reply.status != "OBSERVED" {
            return Err(HostError::Untrusted);
        }
        let observation =
            self.canonicalize_recovery_draft(reply.observation.ok_or(HostError::Untrusted)?)?;
        if !observation.matches_binding(&self.binding) {
            self.stats.last_status = if observation.application_session
                != self.binding.application_session_fingerprint
            {
                "APPLICATION_SESSION_MISMATCH"
            } else {
                "CONVERSATION_MISMATCH"
            }
            .into();
            return Err(HostError::Untrusted);
        }
        Ok(observation)
    }
    pub(crate) fn read_messages(&mut self) -> Result<NativeReadFrame> {
        self.owner.verify()?;
        self.stats.read_requests += 1;
        let reply = self.worker.request("read", None, None)?;
        self.stats.last_status = reply.status.clone();
        let mut frame = read_frame(reply)?;
        frame.observation = self.canonicalize_recovery_draft(frame.observation)?;
        if !frame.observation.matches_binding(&self.binding) {
            return Err(HostError::Untrusted);
        }
        Ok(frame)
    }

    /// Bind dispatch to the exact context that produced the reply, not the first post-model read.
    pub(crate) fn pin_context(&mut self, observation: NativeSendObservation) -> Result<()> {
        observation.validate()?;
        if !observation.matches_binding(&self.binding) {
            return Err(HostError::Untrusted);
        }
        self.baseline = Some(observation);
        Ok(())
    }

    fn live(&self, observation: &NativeSendObservation) -> LiveTarget {
        let draft = match (observation.draft_state.as_str(), &observation.draft_text) {
            ("EMPTY_HEURISTIC", Some(t)) if t.is_empty() => Draft::Empty,
            ("NONEMPTY", Some(t)) if !t.is_empty() => Draft::Text(t.clone()),
            _ => Draft::Unreadable,
        };
        LiveTarget {
            key: self.binding.binding.key.clone(),
            identity_epoch: self.binding.binding.identity_epoch,
            application_session_ref: observation.application_session.clone(),
            conversation_surface_ref: observation.conversation.clone(),
            window_ref: observation.window_ref.clone(),
            editor_ref: format!("{}:composer-v1", observation.layout_ref),
            layout_revision: 1,
            draft,
            conversation_changed: self
                .baseline
                .as_ref()
                .is_some_and(|b| !b.same_surface(observation) || !b.same_messages(observation)),
            permitted: observation.matches_binding(&self.binding),
            frontmost: observation.frontmost,
        }
    }
    fn expected(
        &self,
        action: &OutboundAction,
        live: &LiveTarget,
    ) -> Result<NativeSendObservation> {
        self.owner.verify()?;
        let observation = self.last.as_ref().ok_or(HostError::Untrusted)?;
        if action.target != self.binding.binding.key || *live != self.live(observation) {
            return Err(HostError::Untrusted);
        }
        Ok(observation.clone())
    }
}
fn matching_outgoing(
    before: &NativeSendObservation,
    after: &NativeSendObservation,
    text: &str,
) -> bool {
    if before.evidence_revision.as_deref() != Some(RECEIPT_REVISION)
        || after.evidence_revision.as_deref() != Some(RECEIPT_REVISION)
        || !before.same_surface(after)
        || !after.frontmost
        || after.draft_state != "EMPTY_HEURISTIC"
        || before
            .messages
            .iter()
            .chain(&after.messages)
            .any(|m| !m.complete)
    {
        return false;
    }
    let digest = format!("{:x}", Sha256::digest(text.as_bytes()));
    let matched = |m: &MessageSignature| m.direction == "ME" && m.digest == digest;
    if before.messages.is_empty() {
        return after.messages.len() == 1 && matched(&after.messages[0]);
    }
    let lower = 2.min(before.messages.len());
    let upper = before.messages.len().min(after.messages.len());
    let overlaps: Vec<_> = (lower..=upper)
        .filter(|&n| {
            before.messages[before.messages.len() - n..]
                .iter()
                .zip(&after.messages[..n])
                .all(|(a, b)| a.same_context(b))
        })
        .collect();
    overlaps.len() == 1
        && after.messages[overlaps[0]..]
            .iter()
            .filter(|m| matched(m))
            .count()
            == 1
}

fn channel_error(_: HostError) -> foxbot_core::Error {
    foxbot_core::Error::Blocked("native channel unavailable")
}

impl MessageChannel for NativeSendChannel<'_> {
    fn inspect(&mut self, key: &ConversationKey) -> foxbot_core::Result<LiveTarget> {
        if *key != self.binding.binding.key {
            return Err(foxbot_core::Error::Blocked("native target mismatch"));
        }
        let observation = self.observe().map_err(channel_error)?;
        if self.baseline.is_none() {
            self.baseline = Some(observation.clone());
        }
        let live = self.live(&observation);
        self.last = Some(observation);
        Ok(live)
    }
    fn fill(&mut self, action: &OutboundAction, expected: &LiveTarget) -> foxbot_core::Result<()> {
        let before = self.expected(action, expected).map_err(channel_error)?;
        self.stats.fill_requests += 1;
        let reply = self
            .worker
            .request("fill", Some(&before), Some(action))
            .map_err(channel_error)?;
        self.stats.write_attempted = Some(reply.write_attempted);
        self.stats.last_status = reply.status;
        if self.stats.last_status != "FILLED" || !reply.write_attempted {
            return Err(foxbot_core::Error::Blocked("native fill not verified"));
        }
        let after = reply
            .observation
            .ok_or(foxbot_core::Error::Blocked("native readback missing"))?;
        if !before.same_surface(&after)
            || self.live(&after).draft != Draft::Text(action.text.clone())
        {
            return Err(foxbot_core::Error::Blocked("native readback mismatch"));
        }
        self.last = Some(after);
        Ok(())
    }
    fn send(
        &mut self,
        action: &OutboundAction,
        expected: &LiveTarget,
    ) -> foxbot_core::Result<SendEvidence> {
        let mut before = self.expected(action, expected).map_err(channel_error)?;
        if self.stats.send_requests != 0 {
            return Err(foxbot_core::Error::Blocked("single send already attempted"));
        }
        // Persist receipt anchors BEFORE requesting the external side effect. No chat text on disk.
        before.draft_text = None;
        g2d_real::write_private_json(
            &self.receipt_path,
            &ReceiptContext {
                action_id: action.action_id.clone(),
                text_digest: format!("{:x}", Sha256::digest(action.text.as_bytes())),
                before: before.clone(),
            },
        )
        .map_err(channel_error)?;
        self.owner.verify().map_err(channel_error)?;
        self.stats.send_requests += 1;
        let command = if self.recovery_text.is_some() {
            "recover_send"
        } else {
            "send"
        };
        let reply = self
            .worker
            .request(command, Some(&before), Some(action))
            .map_err(channel_error)?;
        self.stats.send_attempted = Some(reply.send_attempted);
        self.stats.last_status = reply.status;
        if self.stats.last_status == "VERIFIED_OUTGOING"
            && reply.send_attempted
            && reply.verified_outgoing
            && reply
                .observation
                .as_ref()
                .is_some_and(|after| matching_outgoing(&before, after, &action.text))
        {
            Ok(SendEvidence::ObservedOutgoing {
                action_id: action.action_id.clone(),
            })
        } else {
            Ok(SendEvidence::Unknown)
        }
    }
    fn reconcile(&mut self, action: &OutboundAction) -> foxbot_core::Result<SendEvidence> {
        self.owner.verify().map_err(channel_error)?;
        let context: ReceiptContext =
            g2d_real::read_private_json(&self.receipt_path).map_err(channel_error)?;
        if context.action_id != action.action_id
            || context.text_digest != format!("{:x}", Sha256::digest(action.text.as_bytes()))
            || !context.before.matches_binding(&self.binding)
        {
            return Err(foxbot_core::Error::Blocked("receipt context mismatch"));
        }
        self.stats.reconcile_requests += 1;
        let reply = self
            .worker
            .request("reconcile", Some(&context.before), Some(action))
            .map_err(channel_error)?;
        self.stats.last_status = reply.status;
        if self.stats.last_status == "VERIFIED_OUTGOING"
            && reply.verified_outgoing
            && reply
                .observation
                .as_ref()
                .is_some_and(|after| matching_outgoing(&context.before, after, &action.text))
        {
            Ok(SendEvidence::ObservedOutgoing {
                action_id: action.action_id.clone(),
            })
        } else {
            Ok(SendEvidence::Unknown)
        }
    }
}
