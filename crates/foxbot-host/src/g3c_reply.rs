//! G3c-2: explicitly armed, one real incoming message -> HTTP reply -> native send.
//! No daemon, historical backfill, implicit model retry, plaintext ledger or target navigation.
use crate::{
    HostError, Result,
    credentials::{CredentialRef, CredentialStore, NativeCredentials},
    g2d_real::{read_private_json, write_private_json},
    g3c_real,
    native_bridge::NativeConversationBinding,
    native_send::{
        NativeReadFrame, NativeReadMessage, NativeSendChannel, NativeSendObservation,
        NativeSendStats,
    },
    ownership::{DeviceOwner, private_directory},
};
use foxbot_core::{
    ActionState, ContentKind, Direction, IngestOutcome, Mention, Message, Mode, Observation,
    ProviderProfile, Runtime, Source,
};
use foxbot_http::{CancellationToken, HttpConfig, HttpReplyService, Protocol, RunClock};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplyOnceConfig {
    pub schema_version: u32,
    pub http: HttpConfig,
    pub token: Option<CredentialRef>,
    pub ledger_key: CredentialRef,
    pub provider: ProviderProfile,
    pub wait_ms: u64,
    pub poll_ms: u64,
}
impl ReplyOnceConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let config: Self = read_private_json(path)?;
        config.validate()?;
        Ok(config)
    }
    fn validate(&self) -> Result<()> {
        self.http.validate().map_err(|_| HostError::Config)?;
        self.ledger_key.validate()?;
        if let Some(token) = &self.token {
            token.validate()?;
        }
        if self.schema_version != 1
            || self.http.endpoint.contains("YOUR-PROVIDER.invalid")
            || self.http.model.as_deref() == Some("YOUR_MODEL")
            || !(100..=5000).contains(&self.poll_ms)
            || self.wait_ms < self.poll_ms
            || self.wait_ms > 120_000
            || self.http.max_attempts != 1
            || self.http.max_in_flight != 1
            || self.http.total_timeout_ms > 120_000
            || (self.http.protocol == Protocol::BusinessV1
                && !matches!(self.provider, ProviderProfile::Custom))
            || matches!(&self.provider, ProviderProfile::Generic { system_prompt } if system_prompt.len() > 16_384)
        {
            return Err(HostError::Config);
        }
        Ok(())
    }
    fn digest(&self) -> Result<String> {
        Ok(format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(self).map_err(|_| HostError::Config)?)
        ))
    }
    fn service(&self, store: &impl CredentialStore) -> Result<HttpReplyService> {
        let token = self
            .token
            .as_ref()
            .map(|r| store.load(r, "token"))
            .transpose()?;
        HttpReplyService::new(
            self.http.clone(),
            token.as_ref().map(|s| s.token()).transpose()?,
        )
        .map_err(|_| HostError::Config)
    }
}

#[derive(Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum Progress {
    Armed,
    Started,
    Finished,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RunState {
    schema_version: u32,
    config_digest: String,
    profile_tag: String,
    binding: NativeConversationBinding,
    baseline: NativeSendObservation,
    armed_at_ms: u64,
    progress: Progress,
    outcome: String,
    request_id: Option<String>,
    action_id: Option<String>,
}
fn run_directory(session: &str, run: &str) -> Result<PathBuf> {
    if run.is_empty()
        || run.len() > 40
        || !run
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
    {
        return Err(HostError::Config);
    }
    Ok(crate::g2d_real::session_directory(session)?
        .join("g3c-2")
        .join(run))
}
fn available(observation: &NativeSendObservation) -> bool {
    observation.frontmost
        && observation.conversation_resolved
        && observation.draft_state == "EMPTY_HEURISTIC"
        && observation.draft_text.as_deref() == Some("")
        && observation.messages.iter().all(|m| m.complete)
}

/// Return the one new incoming's index. Stable repeated text is not text-hash deduplicated.
fn new_incoming(
    before: &NativeSendObservation,
    after: &NativeSendObservation,
) -> Result<Option<usize>> {
    before.validate()?;
    after.validate()?;
    if !before.same_surface(after) || !available(after) {
        return Err(HostError::Untrusted);
    }
    if before.same_messages(after) {
        return Ok(None);
    }
    let upper = before.messages.len().min(after.messages.len());
    let overlaps: Vec<_> = (2..=upper)
        .filter(|&n| {
            before.messages[before.messages.len() - n..]
                .iter()
                .zip(&after.messages[..n])
                .all(|(a, b)| a.same_context(b))
        })
        .collect();
    if overlaps.len() != 1 {
        return Err(HostError::Untrusted);
    }
    let index = overlaps[0];
    if after.messages.len() != index + 1 || after.messages[index].direction != "THEM" {
        return Err(HostError::Untrusted);
    }
    Ok(Some(index))
}
fn observation(
    binding: &NativeConversationBinding,
    message: &NativeReadMessage,
    id: String,
    historical: bool,
    now: u64,
) -> Observation {
    Observation {
        key: binding.binding.key.clone(),
        identity_epoch: binding.binding.identity_epoch,
        source: Source::Ocr,
        source_event_id: id.clone(),
        historical,
        observed_ms: now,
        message: Message {
            canonical_id: Some(id),
            sender: if message.direction == "THEM" {
                Some(format!("peer:{}", binding.binding.key.conversation))
            } else {
                None
            },
            direction: if message.direction == "THEM" {
                Direction::Incoming
            } else {
                Direction::Own
            },
            mention: Mention::Absent,
            kind: ContentKind::Text,
            text: Some(message.text.clone()),
            complete: message.complete,
            reply_to: None,
        },
    }
}
fn supported_reply(text: &str) -> bool {
    // Matches the G3c-1 native writer. Reject before ANY GUI write; never truncate a model reply.
    !text.is_empty()
        && text.encode_utf16().count() <= 80
        && text.trim() == text
        && !text.chars().any(char::is_control)
        && !text.ends_with('|')
}
fn report(
    state: &RunState,
    runtime: &Runtime,
    stats: &NativeSendStats,
    model_jobs: u32,
) -> Result<serde_json::Value> {
    let action_state = state
        .action_id
        .as_ref()
        .map(|id| runtime.action(id).map(|(_, s)| s))
        .transpose()?;
    Ok(
        serde_json::json!({"schema_version":"foxbot.g3c-reply-once.v1", "status":state.outcome,
        "action_state":action_state, "model_jobs_this_invocation":model_jobs, "native":stats,
        "encrypted_outbox":runtime.is_encrypted(), "trigger_source":"NATIVE_NEW_INCOMING",
        "reply_source":"HTTP_REPLY_PROVIDER", "raw_text_included":false,
        "delivery_confirmed":false, "read_confirmed":false}),
    )
}
fn validate_state(
    state: &RunState,
    config: &ReplyOnceConfig,
    service: &HttpReplyService,
) -> Result<()> {
    if state.schema_version != 1
        || state.config_digest != config.digest()?
        || state.profile_tag != service.profile_tag()
    {
        return Err(HostError::Untrusted);
    }
    state.baseline.validate()?;
    Ok(())
}

pub fn read_check(worker: &Path, session: &str) -> Result<serde_json::Value> {
    let binding = g3c_real::binding(session)?;
    let owner = DeviceOwner::acquire()?;
    let mut channel = NativeSendChannel::start(worker, binding, &owner, PathBuf::new(), false)?;
    let frame = channel.read_messages()?;
    Ok(
        serde_json::json!({"status":"PRIVATE_READ_VALIDATED", "message_count":frame.messages.len(),
        "incoming_count":frame.messages.iter().filter(|m| m.direction == "THEM").count(),
        "draft_state":frame.observation.draft_state, "frontmost":frame.observation.frontmost,
        "read_requests":channel.stats.read_requests, "model_requests":0,
        "write_operations":0, "send_operations":0, "raw_text_included":false}),
    )
}

pub fn check(config_path: &Path, session: &str) -> Result<serde_json::Value> {
    let config = ReplyOnceConfig::load(config_path)?;
    let _binding = g3c_real::binding(session)?;
    let _service = config.service(&NativeCredentials)?;
    NativeCredentials
        .load(&config.ledger_key, "ledger")?
        .ledger_key()?;
    Ok(
        serde_json::json!({"status":"CONFIG_AND_CREDENTIALS_AVAILABLE", "model_requests":0,
        "native_chat_operations":0, "secret_included":false,
        "note":"No network probe; endpoint reachability is not verified"}),
    )
}

pub fn arm(
    config_path: &Path,
    worker: &Path,
    session: &str,
    run: &str,
) -> Result<serde_json::Value> {
    let config = ReplyOnceConfig::load(config_path)?;
    let owner = DeviceOwner::acquire()?;
    let mut binding = g3c_real::binding(session)?;
    binding.binding.provider = config.provider.clone();
    binding.binding.mode = Mode::AutoReply;
    binding.binding.quiet_ms = 0;
    binding.binding.max_wait_ms = 0;
    binding.binding.max_auto_sends = 1;
    binding.binding.reply_ttl_ms = config.http.total_timeout_ms.saturating_add(60_000);
    binding.binding.validate()?;
    let service = config.service(&NativeCredentials)?;
    let secret = NativeCredentials.load(&config.ledger_key, "ledger")?;
    let directory = run_directory(session, run)?;
    if directory.join("run.json").exists() {
        let state: RunState = read_private_json(&directory.join("run.json"))?;
        validate_state(&state, &config, &service)?;
        return Ok(
            serde_json::json!({"status":"ALREADY_ARMED_OR_USED", "baseline_replaced":false,
            "native_chat_operations":0, "model_requests":0}),
        );
    }
    if directory.join("runtime").exists() {
        return Err(HostError::Config);
    }
    let mut channel = NativeSendChannel::start(
        worker,
        binding.clone(),
        &owner,
        directory.join("receipt.json"),
        false,
    )?;
    let first = channel.read_messages()?;
    let second = channel.read_messages()?;
    if !available(&second.observation)
        || first.messages.len() < 2
        || !first.observation.same_surface(&second.observation)
        || !first.observation.same_messages(&second.observation)
    {
        return Err(HostError::Untrusted);
    }
    private_directory(&directory)?;
    let mut runtime = Runtime::open_encrypted(directory.join("runtime"), secret.ledger_key()?)?;
    let state = arm_runtime(
        &mut runtime,
        &config,
        &service,
        binding,
        second,
        RunClock::default().now_ms(),
    )?;
    write_private_json(&directory.join("run.json"), &state)?;
    let value = report(&state, &runtime, &channel.stats, 0)?;
    write_private_json(&directory.join("arm-report.json"), &value)?;
    Ok(value)
}
fn arm_runtime(
    runtime: &mut Runtime,
    config: &ReplyOnceConfig,
    service: &HttpReplyService,
    binding: NativeConversationBinding,
    frame: NativeReadFrame,
    now: u64,
) -> Result<RunState> {
    runtime.bind(&binding.binding)?;
    runtime.set_host_paused(false)?;
    for (i, m) in frame.messages.iter().enumerate() {
        if runtime.ingest(&observation(
            &binding,
            m,
            format!("g3c2-history-{i}"),
            true,
            now,
        ))? != IngestOutcome::Baseline
        {
            return Err(HostError::Untrusted);
        }
    }
    let mut baseline = frame.observation;
    baseline.draft_text = Some(String::new());
    Ok(RunState {
        schema_version: 1,
        config_digest: config.digest()?,
        profile_tag: service.profile_tag().into(),
        binding,
        baseline,
        armed_at_ms: now,
        progress: Progress::Armed,
        outcome: "ARMED_WAITING_FOR_NEW_MESSAGE".into(),
        request_id: None,
        action_id: None,
    })
}

pub async fn execute(
    config_path: &Path,
    worker: &Path,
    session: &str,
    run: &str,
    cancellation: CancellationToken,
) -> Result<serde_json::Value> {
    let config = ReplyOnceConfig::load(config_path)?;
    let owner = DeviceOwner::acquire()?;
    let directory = run_directory(session, run)?;
    let service = config.service(&NativeCredentials)?;
    let secret = NativeCredentials.load(&config.ledger_key, "ledger")?;
    let mut state: RunState = read_private_json(&directory.join("run.json"))?;
    validate_state(&state, &config, &service)?;
    let mut runtime = Runtime::open_encrypted(directory.join("runtime"), secret.ledger_key()?)?;
    execute_runtime(
        &config,
        &service,
        &mut state,
        &mut runtime,
        worker,
        &owner,
        &directory,
        cancellation,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn execute_runtime(
    config: &ReplyOnceConfig,
    service: &HttpReplyService,
    state: &mut RunState,
    runtime: &mut Runtime,
    worker: &Path,
    owner: &DeviceOwner,
    directory: &Path,
    cancellation: CancellationToken,
) -> Result<serde_json::Value> {
    config.validate()?;
    validate_state(state, config, service)?;
    let clock = RunClock::default();
    let mut stats = NativeSendStats::default();
    let mut model_jobs = 0;
    // Recovery is read-only. A durable generation claim never becomes a new HTTP request.
    if let Some(action) = &state.action_id {
        let (_, old) = runtime.action(action)?;
        let final_state = if matches!(old, ActionState::Unknown | ActionState::Submitted)
            && directory.join("receipt.json").exists()
            && !cancellation.is_cancelled()
        {
            let mut channel = NativeSendChannel::start(
                worker,
                state.binding.clone(),
                owner,
                directory.join("receipt.json"),
                false,
            )?;
            let result = runtime.reconcile(action, &mut channel)?;
            stats = channel.stats;
            result
        } else {
            old
        };
        if final_state == ActionState::VerifiedOutgoing {
            state.outcome = "VERIFIED_OUTGOING".into();
        }
        state.progress = Progress::Finished;
    } else if state.progress == Progress::Started {
        state.outcome = "INTERRUPTED_NO_AUTOMATIC_RETRY".into();
        state.progress = Progress::Finished;
    } else if state.progress == Progress::Armed {
        if cancellation.is_cancelled() {
            state.outcome = "CANCELLED".into();
        } else if clock.now_ms() < state.armed_at_ms
            || clock.now_ms() - state.armed_at_ms > 1_800_000
        {
            state.outcome = "ARM_EXPIRED".into();
            state.progress = Progress::Finished;
        } else {
            let mut channel = NativeSendChannel::start(
                worker,
                state.binding.clone(),
                owner,
                directory.join("receipt.json"),
                true,
            )?;
            let started = Instant::now();
            let selected = loop {
                if cancellation.is_cancelled() {
                    state.outcome = "CANCELLED".into();
                    break None;
                }
                let frame = channel.read_messages()?;
                let index = match new_incoming(&state.baseline, &frame.observation) {
                    Ok(index) => index,
                    Err(_) => {
                        state.outcome = "NEW_MESSAGE_OR_TARGET_AMBIGUOUS".into();
                        state.progress = Progress::Finished;
                        break None;
                    }
                };
                if let Some(index) = index {
                    let stable = channel.read_messages()?;
                    if !frame.observation.same_surface(&stable.observation)
                        || !frame.observation.same_messages(&stable.observation)
                        || frame.messages[index].text != stable.messages[index].text
                        || !available(&stable.observation)
                    {
                        state.outcome = "NEW_MESSAGE_UNSTABLE".into();
                        state.progress = Progress::Finished;
                        break None;
                    }
                    break Some((stable, index));
                }
                if started.elapsed() >= Duration::from_millis(config.wait_ms) {
                    state.outcome = "WAITING_FOR_NEW_MESSAGE".into();
                    break None;
                }
                tokio::select! {
                    _ = cancellation.cancelled() => {},
                    _ = tokio::time::sleep(Duration::from_millis(config.poll_ms)) => {},
                }
            };
            if let Some((frame, index)) = selected {
                state.progress = Progress::Started;
                state.outcome = "GENERATION_CLAIMED".into();
                write_private_json(&directory.join("run.json"), state)?;
                // Exactly one real adapter-origin incoming. No fixture text is injected.
                if runtime.ingest(&observation(
                    &state.binding,
                    &frame.messages[index],
                    "g3c2-incoming-1".into(),
                    false,
                    clock.now_ms(),
                ))? != IngestOutcome::Queued
                {
                    return Err(HostError::Untrusted);
                }
                runtime.set_host_paused(false)?;
                let job = service
                    .begin(runtime, &state.binding.binding.key, clock.now_ms(), None)
                    .map_err(|_| HostError::Storage)?
                    .ok_or(HostError::Untrusted)?;
                state.request_id = Some(job.request_id().to_owned());
                write_private_json(&directory.join("run.json"), state)?;
                channel.pin_context(frame.observation.clone())?;
                model_jobs = 1;
                let completion = service.run(job, cancellation.clone()).await;
                // A changed conversation while the model was running invalidates this answer.
                let fresh = channel.observe();
                let unchanged = fresh.as_ref().is_ok_and(|current| {
                    frame.observation.same_surface(current)
                        && frame.observation.same_messages(current)
                        && available(current)
                });
                if cancellation.is_cancelled() || !unchanged {
                    service
                        .cancel(
                            runtime,
                            state.request_id.as_deref().ok_or(HostError::Config)?,
                        )
                        .map_err(|_| HostError::Storage)?;
                    state.outcome = "REPLY_CANCELLED_OR_CONTEXT_CHANGED".into();
                } else {
                    match service.finish(runtime, completion, clock.now_ms()) {
                        Err(_) => state.outcome = "MODEL_FAILED_OR_INCOMPLETE".into(),
                        Ok(request_id) if runtime.task_state(&request_id)? != "READY" => {
                            state.outcome = runtime.task_state(&request_id)?;
                        }
                        Ok(request_id) => {
                            let action_id =
                                runtime.prepare_send(&request_id, clock.now_ms(), false)?;
                            state.action_id = Some(action_id.clone());
                            state.outcome = "PREPARED".into();
                            write_private_json(&directory.join("run.json"), state)?;
                            let (action, _) = runtime.action(&action_id)?;
                            if !supported_reply(&action.text) {
                                runtime.pause(&state.binding.binding.key)?;
                                state.outcome = "UNSUPPORTED_REPLY_NO_WRITE".into();
                            } else {
                                let result =
                                    runtime.dispatch(&action_id, clock.now_ms(), &mut channel)?;
                                state.outcome = serde_json::to_value(result)
                                    .map_err(|_| HostError::Config)?
                                    .as_str()
                                    .ok_or(HostError::Config)?
                                    .into();
                            }
                        }
                    }
                }
                state.progress = Progress::Finished;
            }
            stats = channel.stats;
        }
    }
    write_private_json(&directory.join("run.json"), state)?;
    // Business service receipts use the existing durable outbox, never another chat send.
    runtime.sync_service_receipts()?;
    let mut feedback = "NOT_REQUIRED_OR_NOT_DUE";
    if !cancellation.is_cancelled()
        && let Some(claim) = service
            .begin_feedback(runtime, clock.now_ms())
            .map_err(|_| HostError::Storage)?
    {
        let completion = service.run_feedback(claim, cancellation.clone()).await;
        feedback = if service
            .finish_feedback(runtime, completion, clock.now_ms())
            .is_ok()
        {
            "ACKNOWLEDGED"
        } else {
            "PENDING"
        };
    }
    let mut value = report(state, runtime, &stats, model_jobs)?;
    value["feedback_status"] = serde_json::json!(feedback);
    write_private_json(&directory.join("last-report.json"), &value)?;
    Ok(value)
}

#[cfg(all(test, unix))]
#[path = "g3c_reply_tests.rs"]
mod tests;
