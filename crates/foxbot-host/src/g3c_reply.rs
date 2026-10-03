//! G3c-2: explicitly armed, one real incoming message -> HTTP reply -> native send.
//! Local multi-connection mode is the default. Legacy encrypted runs remain isolated.
//! No daemon, historical backfill, implicit model retry or target navigation.
use crate::{
    HostError, Result,
    credentials::{CredentialRef, CredentialStore, NativeCredentials},
    g2d_real::{read_private_json, write_private_json},
    g3c_real,
    native_bridge::NativeConversationBinding,
    native_send::{
        MAX_SEND_UTF16, NativeReadFrame, NativeReadMessage, NativeSendChannel,
        NativeSendObservation, NativeSendStats, supported_send_text,
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
    pub ledger_key: Option<CredentialRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_id: Option<String>,
    #[serde(skip)]
    pub api_key: Option<String>,
    pub provider: ProviderProfile,
    pub wait_ms: u64,
    pub poll_ms: u64,
}
impl ReplyOnceConfig {
    pub fn load(path: &Path) -> Result<Self> {
        Self::load_selected(path, None)
    }

    fn load_selected(path: &Path, pinned: Option<&str>) -> Result<Self> {
        let value = crate::local_config::read_value(path)?;
        let config = if value.get("version").and_then(|v| v.as_u64()) == Some(2) {
            let local: crate::local_config::LocalConfig =
                serde_json::from_value(value).map_err(|_| HostError::Config)?;
            Self::from_local(&local, pinned)?
        } else {
            if pinned.is_some() {
                return Err(HostError::Config);
            }
            let legacy: Self = read_private_json(path)?;
            if legacy.schema_version != 1 || legacy.connection_id.is_some() {
                return Err(HostError::Config);
            }
            legacy
        };
        config.validate()?;
        Ok(config)
    }
    pub(crate) fn from_local(
        local: &crate::local_config::LocalConfig,
        pinned: Option<&str>,
    ) -> Result<Self> {
        local.validate()?;
        let c = local.selected(pinned)?;
        Ok(Self {
            schema_version: 2,
            http: c.http(),
            token: None,
            ledger_key: None,
            connection_id: Some(c.id.clone()),
            api_key: Some(c.api_key.clone()),
            provider: c.profile(&local.reply.system_prompt),
            wait_ms: 60_000,
            poll_ms: 1000,
        })
    }
    fn open_runtime(&self, path: &Path) -> Result<Runtime> {
        self.validate()?;
        if self.schema_version == 2 {
            return Ok(Runtime::open_local(path)?);
        }
        let key = self.ledger_key.as_ref().ok_or(HostError::Config)?;
        let secret = NativeCredentials.load(key, "ledger")?;
        Ok(Runtime::open_encrypted(path, secret.ledger_key()?)?)
    }
    fn validate(&self) -> Result<()> {
        self.http.validate().map_err(|_| HostError::Config)?;
        if let Some(key) = &self.ledger_key {
            key.validate()?;
        }
        match self.schema_version {
            1 if self.ledger_key.is_some()
                && self.connection_id.is_none()
                && self.api_key.is_none() => {}
            2 if self.ledger_key.is_none()
                && self.token.is_none()
                && self
                    .connection_id
                    .as_deref()
                    .is_some_and(crate::local_config::identifier)
                && self.api_key.is_some() => {}
            _ => return Err(HostError::Config),
        }
        if let Some(token) = &self.token {
            token.validate()?;
        }
        if self.http.endpoint.contains("YOUR-PROVIDER.invalid")
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
        if self.schema_version == 2 {
            // Only the pinned connection and reply semantics, not defaults or unrelated entries.
            // Credential changes are separately fenced by HttpReplyService::profile_tag.
            return Ok(
                format!("{:x}", Sha256::digest(serde_json::to_vec(&serde_json::json!({
                "connection_id":self.connection_id, "http":self.http, "provider":self.provider
            })).map_err(|_| HostError::Config)?)),
            );
        }
        Ok(format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(self).map_err(|_| HostError::Config)?)
        ))
    }
    fn service(&self, store: &impl CredentialStore) -> Result<HttpReplyService> {
        if self.schema_version == 2 {
            let key = self.api_key.as_deref().filter(|s| !s.is_empty());
            return HttpReplyService::new(self.http.clone(), key).map_err(|_| HostError::Config);
        }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    connection_id: Option<String>,
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
fn run_directory(config_path: &Path, session: &str, run: &str) -> Result<PathBuf> {
    if run.is_empty()
        || run.len() > 40
        || !run
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
    {
        return Err(HostError::Config);
    }
    if crate::local_config::read_value(config_path)?
        .get("version")
        .and_then(|v| v.as_u64())
        == Some(2)
    {
        if !crate::local_config::identifier(session) {
            return Err(HostError::Config);
        }
        let legacy = PathBuf::from("target/g2d-real")
            .join(session)
            .join("g3c-2")
            .join(run);
        if legacy.join("run.json").exists() || legacy.join("runtime").exists() {
            return Err(HostError::Config); // Do not recreate an old run using a different store.
        }
        let root = config_path.parent().ok_or(HostError::Config)?;
        return Ok(root.join("runs").join(session).join(run));
    }
    Ok(crate::g2d_real::session_directory(session)?
        .join("g3c-2")
        .join(run))
}
fn context_available(observation: &NativeSendObservation) -> bool {
    observation.frontmost
        && observation.conversation_resolved
        && observation.messages.iter().all(|m| m.complete)
}
fn available(observation: &NativeSendObservation) -> bool {
    context_available(observation)
        && observation.draft_state == "EMPTY_HEURISTIC"
        && observation.draft_text.as_deref() == Some("")
}

/// Return the one new incoming's index. Stable repeated text is not text-hash deduplicated.
fn new_incoming_context(
    before: &NativeSendObservation,
    after: &NativeSendObservation,
) -> Result<Option<usize>> {
    before.validate()?;
    after.validate()?;
    if !before.same_surface(after) || !context_available(after) {
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
fn new_incoming(
    before: &NativeSendObservation,
    after: &NativeSendObservation,
) -> Result<Option<usize>> {
    if !available(after) {
        return Err(HostError::Untrusted);
    }
    new_incoming_context(before, after)
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
    // Reject before ANY GUI write; never truncate or rewrite a model reply.
    supported_send_text(text)
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
        "connection_id":state.connection_id,
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
        || state.connection_id != config.connection_id
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
    if let Some(key) = &config.ledger_key {
        NativeCredentials.load(key, "ledger")?.ledger_key()?;
    }
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
    let directory = run_directory(config_path, session, run)?;
    let existing: Option<RunState> = if directory.join("run.json").exists() {
        Some(read_private_json(&directory.join("run.json"))?)
    } else {
        None
    };
    let config = ReplyOnceConfig::load_selected(
        config_path,
        existing.as_ref().and_then(|s| s.connection_id.as_deref()),
    )?;
    let owner = DeviceOwner::acquire()?;
    let mut binding = g3c_real::binding(session)?;
    binding.binding.provider = config.provider.clone();
    binding.binding.mode = Mode::AutoReply;
    binding.binding.quiet_ms = 0;
    binding.binding.max_wait_ms = 0;
    binding.binding.max_auto_sends = 1;
    binding.binding.max_reply_chars = MAX_SEND_UTF16;
    binding.binding.reply_ttl_ms = config.http.total_timeout_ms.saturating_add(60_000);
    binding.binding.validate()?;
    let service = config.service(&NativeCredentials)?;

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
    let mut runtime = config.open_runtime(&directory.join("runtime"))?;
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
        connection_id: config.connection_id.clone(),
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
    let owner = DeviceOwner::acquire()?;
    let directory = run_directory(config_path, session, run)?;
    let mut state: RunState = read_private_json(&directory.join("run.json"))?;
    let config = ReplyOnceConfig::load_selected(config_path, state.connection_id.as_deref())?;
    let service = config.service(&NativeCredentials)?;
    validate_state(&state, &config, &service)?;
    let mut runtime = config.open_runtime(&directory.join("runtime"))?;
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

/// Explicitly continue an existing UNKNOWN/fill_uncertain task. No model request and no
/// second fill are permitted; the currently visible draft must exactly match the persisted
/// reply and the same single incoming must still be the only change since arm.
pub fn recover_filled(
    config_path: &Path,
    worker: &Path,
    session: &str,
    run: &str,
) -> Result<serde_json::Value> {
    let owner = DeviceOwner::acquire()?;
    let directory = run_directory(config_path, session, run)?;
    let mut state: RunState = read_private_json(&directory.join("run.json"))?;
    let config = ReplyOnceConfig::load_selected(config_path, state.connection_id.as_deref())?;
    let service = config.service(&NativeCredentials)?;
    validate_state(&state, &config, &service)?;
    let mut runtime = config.open_runtime(&directory.join("runtime"))?;
    if state.progress != Progress::Finished
        || state.outcome != "UNKNOWN"
        || directory.join("receipt.json").exists()
    {
        return Err(HostError::Untrusted);
    }
    let action_id = state.action_id.clone().ok_or(HostError::Untrusted)?;
    let (action, action_state) = runtime.action(&action_id)?;
    if action_state != ActionState::Unknown || !supported_reply(&action.text) {
        return Err(HostError::Untrusted);
    }
    let mut channel = NativeSendChannel::start_filled_recovery(
        worker,
        state.binding.clone(),
        &owner,
        directory.join("receipt.json"),
        &action,
    )?;
    let first = channel.read_messages()?;
    let second = channel.read_messages()?;
    let first_index =
        new_incoming_context(&state.baseline, &first.observation)?.ok_or(HostError::Untrusted)?;
    let second_index =
        new_incoming_context(&state.baseline, &second.observation)?.ok_or(HostError::Untrusted)?;
    if first_index != second_index
        || !first.observation.same_surface(&second.observation)
        || !first.observation.same_messages(&second.observation)
        || first.messages[first_index].text != second.messages[second_index].text
        || second.observation.draft_text.as_deref() != Some(action.text.as_str())
    {
        return Err(HostError::Untrusted);
    }
    let final_state =
        runtime.recover_filled_send(&action_id, RunClock::default().now_ms(), &mut channel)?;
    state.outcome = serde_json::to_value(final_state)
        .map_err(|_| HostError::Config)?
        .as_str()
        .ok_or(HostError::Config)?
        .into();
    state.progress = Progress::Finished;
    write_private_json(&directory.join("run.json"), &state)?;
    let mut value = report(&state, &runtime, &channel.stats, 0)?;
    value["recovery"] = serde_json::json!("EXISTING_FILLED_DRAFT_ONLY");
    value["feedback_status"] = serde_json::json!("NOT_REQUIRED_OR_NOT_DUE");
    write_private_json(&directory.join("last-report.json"), &value)?;
    Ok(value)
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
