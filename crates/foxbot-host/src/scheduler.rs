use crate::{HostConfig, HostError, Result, ownership::DeviceOwner};
use foxbot_core::{ActionState, Binding, MessageChannel, Observation, Runtime};
use foxbot_http::{
    CancellationToken, FeedbackCompletion, HttpCompletion, HttpReplyService, RunClock,
};
use serde::Serialize;
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinSet,
};

#[cfg(test)]
#[path = "scheduler_boundary_tests.rs"]
mod boundary_tests;

#[derive(Clone, Debug, Default, Serialize)]
pub struct HostSnapshot {
    pub paused: bool,
    pub active_jobs: usize,
    pub active_feedback: usize,
    pub provider_jobs_started: u64,
    pub observed_outgoing_this_run: u64,
    pub feedback_acked_this_run: u64,
    pub rejected_inputs: u64,
    pub provider_failures: u64,
    pub messages: u64,
    pub tasks: u64,
    pub ready: u64,
    pub unresolved_sends: u64,
}
struct Input {
    observation: Observation,
    epoch: u64,
    ack: oneshot::Sender<Result<()>>,
}
enum Control {
    Pause,
    Resume,
    Status,
}
struct ControlRequest {
    command: Control,
    ack: oneshot::Sender<Result<HostSnapshot>>,
}

/// Bounded channels; shutdown has a separate cancellation signal so a full data
/// queue cannot hide it. This is an in-process controller, not a public HTTP API.
#[derive(Clone)]
pub struct HostHandle {
    input: mpsc::Sender<Input>,
    control: mpsc::Sender<ControlRequest>,
    stop: CancellationToken,
    epoch: Arc<AtomicU64>,
}
impl HostHandle {
    pub async fn observe(&self, observation: Observation) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        self.input
            .try_send(Input {
                observation,
                epoch: self.epoch.load(Ordering::Acquire),
                ack: tx,
            })
            .map_err(|_| HostError::Closed)?;
        rx.await.map_err(|_| HostError::Closed)?
    }
    async fn control(&self, command: Control) -> Result<HostSnapshot> {
        let (tx, rx) = oneshot::channel();
        self.control
            .try_send(ControlRequest { command, ack: tx })
            .map_err(|_| HostError::Closed)?;
        rx.await.map_err(|_| HostError::Closed)?
    }
    pub async fn pause(&self) -> Result<HostSnapshot> {
        self.control(Control::Pause).await
    }
    pub async fn resume(&self) -> Result<HostSnapshot> {
        self.control(Control::Resume).await
    }
    pub async fn status(&self) -> Result<HostSnapshot> {
        self.control(Control::Status).await
    }
    pub fn stop(&self) {
        self.stop.cancel();
    }
}
struct Active {
    index: usize,
    cancel: CancellationToken,
}

/// Runtime and synchronous simulated dispatch have one owner. Network futures never
/// borrow Runtime. Native adapters MUST later provide bounded, interruptible workers;
/// this actor does not claim it can interrupt a blocking native call.
pub struct Host<C: MessageChannel> {
    runtime: Runtime,
    service: HttpReplyService,
    bindings: Vec<Binding>,
    channel: C,
    owner: DeviceOwner,
    config: HostConfig,
    clock: RunClock,
    input: mpsc::Receiver<Input>,
    control: mpsc::Receiver<ControlRequest>,
    stop: CancellationToken,
    jobs: JoinSet<(String, HttpCompletion)>,
    active: HashMap<String, Active>,
    feedback: JoinSet<FeedbackCompletion>,
    feedback_cancel: CancellationToken,
    feedback_claim: Option<foxbot_core::ReceiptClaim>,
    cursor: usize,
    epoch: Arc<AtomicU64>,
    stats: HostSnapshot,
}
impl<C: MessageChannel> Host<C> {
    pub fn new(
        mut runtime: Runtime,
        service: HttpReplyService,
        config: HostConfig,
        channel: C,
        owner: DeviceOwner,
    ) -> Result<(Self, HostHandle)> {
        config.validate()?;
        if !service.matches_configuration(&config.http) {
            return Err(HostError::Config);
        }
        if matches!(config.storage, crate::StorageConfig::Protected { .. })
            != runtime.is_encrypted()
        {
            return Err(HostError::Config);
        }
        owner.verify()?;
        // Every process start requires explicit resume. Old PREPARED/READY results are
        // cancelled; potentially sent UNKNOWN work is preserved for read-only reconciliation.
        runtime.set_host_paused(true)?;
        let mut bindings = Vec::new();
        for template in &config.bindings {
            let mut binding = template.clone();
            use sha2::{Digest, Sha256};
            let mut hash = Sha256::new();
            hash.update(service.profile_tag().as_bytes());
            hash.update(binding.profile_version.to_le_bytes());
            let digest = hash.finalize();
            binding.profile_version =
                (u64::from_le_bytes(digest[..8].try_into().map_err(|_| HostError::Config)?)
                    & i64::MAX as u64)
                    .max(1);
            // A durable handoff/disabled conversation is not re-enabled by restarting.
            if let Some(old) = runtime.binding(&binding.key)? {
                binding.enabled &= old.enabled;
            }
            runtime.bind(&binding)?;
            bindings.push(binding);
        }
        let (itx, input) = mpsc::channel(64);
        let (ctx, control) = mpsc::channel(8);
        let stop = CancellationToken::new();
        let epoch = Arc::new(AtomicU64::new(1));
        let handle = HostHandle {
            input: itx,
            control: ctx,
            stop: stop.clone(),
            epoch: epoch.clone(),
        };
        Ok((
            Self {
                runtime,
                service,
                bindings,
                channel,
                owner,
                config,
                clock: RunClock::default(),
                input,
                control,
                stop,
                jobs: JoinSet::new(),
                active: HashMap::new(),
                feedback: JoinSet::new(),
                feedback_cancel: CancellationToken::new(),
                feedback_claim: None,
                cursor: 0,
                epoch,
                stats: HostSnapshot {
                    paused: true,
                    ..Default::default()
                },
            },
            handle,
        ))
    }
    fn snapshot(&self) -> Result<HostSnapshot> {
        let mut s = self.stats.clone();
        s.active_jobs = self.active.len();
        s.active_feedback = self.feedback.len();
        let (messages, tasks, ready, unresolved) = self.runtime.host_counts()?;
        s.messages = messages;
        s.tasks = tasks;
        s.ready = ready;
        s.unresolved_sends = unresolved;
        Ok(s)
    }
    fn pause_now(&mut self) -> Result<()> {
        self.stats.paused = true;
        self.epoch.fetch_add(1, Ordering::AcqRel);
        for active in self.active.values() {
            active.cancel.cancel();
        }
        self.runtime.set_host_paused(true)?;
        Ok(())
    }
    fn receive(&mut self, mut observation: Observation, epoch: u64) -> Result<()> {
        if !self.bindings.iter().any(|b| b.key == observation.key) {
            return Err(HostError::Config);
        }
        // Observations during suspension only establish a baseline, never a backlog.
        if self.stats.paused || epoch != self.epoch.load(Ordering::Acquire) {
            observation.historical = true;
        }
        self.runtime.ingest(&observation)?;
        self.cancel_stale()?;
        Ok(())
    }
    fn cancel_stale(&mut self) -> Result<()> {
        for (id, active) in &self.active {
            if self.runtime.task_state(id)? != "GENERATING" {
                active.cancel.cancel();
            }
        }
        Ok(())
    }
    fn finish_job(&mut self, id: String, completion: HttpCompletion) -> Result<()> {
        self.active.remove(&id);
        match self
            .service
            .finish(&mut self.runtime, completion, self.clock.now_ms())
        {
            Ok(_) => {}
            Err(foxbot_http::HttpError::Core) => return Err(HostError::Storage),
            Err(_) => self.stats.provider_failures += 1,
        }
        Ok(())
    }
    fn finish_feedback(&mut self, completion: FeedbackCompletion) -> Result<()> {
        match self
            .service
            .finish_feedback(&mut self.runtime, completion, self.clock.now_ms())
        {
            Ok(()) => self.stats.feedback_acked_this_run += 1,
            Err(foxbot_http::HttpError::Core) => return Err(HostError::Storage),
            Err(_) => {} // Retry/suspension was recorded by the independent receipt ledger.
        }
        self.feedback_claim = None;
        Ok(())
    }
    fn consume_input(&mut self, input: Input) {
        let result = self.receive(input.observation, input.epoch);
        if result.is_err() {
            self.stats.rejected_inputs += 1;
        }
        let _ = input.ack.send(result);
    }

    /// Ingest a bounded prefix BEFORE considering a ready reply. A timer must not
    /// overtake an observation already accepted by this host's input queue. Under
    /// continued load skip dispatch, rather than draining forever and hiding stop.
    fn drain_input_before_dispatch(&mut self) {
        for _ in 0..64 {
            if self.stop.is_cancelled() || !self.control.is_empty() {
                break;
            }
            match self.input.try_recv() {
                Ok(input) => self.consume_input(input),
                Err(_) => break,
            }
        }
    }

    fn control_or_input_pending(&self) -> bool {
        self.stop.is_cancelled() || !self.control.is_empty() || !self.input.is_empty()
    }

    fn tick(&mut self) -> Result<()> {
        if self.stop.is_cancelled() || !self.control.is_empty() {
            return Ok(());
        }
        self.owner.verify()?;
        self.drain_input_before_dispatch();
        self.cancel_stale()?;
        // One independent feedback worker: even while paused, settle already authorized
        // requests with their captured provider. Shutdown cancels this worker too.
        if self.feedback.is_empty()
            && let Some(claim) = self
                .service
                .begin_feedback(&mut self.runtime, self.clock.now_ms())
                .map_err(|_| HostError::Storage)?
        {
            self.feedback_claim = Some(claim.clone());
            let service = self.service.clone();
            self.feedback_cancel = CancellationToken::new();
            let token = self.feedback_cancel.clone();
            self.feedback
                .spawn(async move { service.run_feedback(claim, token).await });
        }
        if self.stats.paused || self.control_or_input_pending() {
            return Ok(());
        }
        let count = self.bindings.len();
        let mut dispatched = false;
        for offset in 0..count {
            if self.control_or_input_pending() {
                break;
            }
            let index = (self.cursor + offset) % count;
            let key = self.bindings[index].key.clone();
            if !dispatched
                && self.bindings[index].mode == foxbot_core::Mode::AutoReply
                && let Some(id) = self.runtime.ready_requests(&key)?.into_iter().next()
            {
                self.owner.verify()?;
                match self.runtime.prepare_send(&id, self.clock.now_ms(), false) {
                    Ok(action) => {
                        let state = self.runtime.dispatch(
                            &action,
                            self.clock.now_ms(),
                            &mut self.channel,
                        )?;
                        if state == ActionState::VerifiedOutgoing {
                            self.stats.observed_outgoing_this_run += 1;
                        }
                        if state == ActionState::Blocked {
                            self.runtime.pause(&key)?;
                        }
                    }
                    Err(foxbot_core::Error::Stale) => {}
                    Err(_) => return Err(HostError::Storage),
                }
                dispatched = true;
            }
            if self.active.len() >= self.config.max_jobs
                || self.active.values().any(|a| a.index == index)
            {
                continue;
            }
            if let Some(job) = self
                .service
                .begin(&mut self.runtime, &key, self.clock.now_ms(), None)
                .map_err(|_| HostError::Storage)?
            {
                let id = job.request_id().to_owned();
                let cancel = CancellationToken::new();
                self.active.insert(
                    id.clone(),
                    Active {
                        index,
                        cancel: cancel.clone(),
                    },
                );
                let service = self.service.clone();
                self.jobs
                    .spawn(async move { (id, service.run(job, cancel).await) });
                self.stats.provider_jobs_started += 1;
            }
        }
        self.cursor = (self.cursor + 1) % count;
        Ok(())
    }
    pub async fn run(mut self) -> Result<HostSnapshot> {
        let mut tick = tokio::time::interval(Duration::from_millis(self.config.tick_ms));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let result: Result<()> = async {
            loop {
                tokio::select! {
                    biased;
                    _=self.stop.cancelled()=>break,
                    request=self.control.recv()=>{
                        let Some(request)=request else {break;};
                        let answer=match request.command {
                            Control::Pause=>self.pause_now(),
                            Control::Resume=>{
                                self.owner.verify()?;
                                self.runtime.set_host_paused(false)?;
                                self.epoch.fetch_add(1,Ordering::AcqRel);
                                self.stats.paused=false; Ok(())
                            },
                            Control::Status=>Ok(()),
                        }.and_then(|()|self.snapshot());
                        let _=request.ack.send(answer);
                    },
                    _=tick.tick()=>self.tick()?,
                    completion=self.jobs.join_next(),if !self.jobs.is_empty()=>{
                        let (id,completion)=completion.ok_or(HostError::Closed)?.map_err(|_|HostError::Storage)?;
                        self.finish_job(id,completion)?;
                    },
                    completion=self.feedback.join_next(),if !self.feedback.is_empty()=>{
                        self.finish_feedback(completion.ok_or(HostError::Closed)?.map_err(|_|HostError::Storage)?)?;
                    },
                    input=self.input.recv()=>{
                        let Some(input)=input else {break;};
                        self.consume_input(input);
                    },
                }
            }
            Ok(())
        }.await;
        // Cancel even on scheduler/ownership failure. No native work is detached.
        let cleanup = self.shutdown().await;
        result?;
        cleanup?;
        self.snapshot()
    }
    async fn shutdown(&mut self) -> Result<()> {
        // Cancellation is issued even when durable storage is failing.
        for active in self.active.values() {
            active.cancel.cancel();
        }
        self.feedback_cancel.cancel();
        let mut outcome = self.pause_now();
        let deadline = tokio::time::Instant::now() + Duration::from_millis(self.config.shutdown_ms);
        while !self.jobs.is_empty() || !self.feedback.is_empty() {
            tokio::select! {
                _=tokio::time::sleep_until(deadline)=>break,
                Some(result)=self.jobs.join_next(),if !self.jobs.is_empty()=>{
                    if let Ok((id,completion))=result && let Err(error)=self.finish_job(id,completion) {outcome=Err(error);}
                },
                Some(result)=self.feedback.join_next(),if !self.feedback.is_empty()=>{
                    if let Ok(completion)=result && let Err(error)=self.finish_feedback(completion) {outcome=Err(error);}
                },
            }
        }
        self.jobs.abort_all();
        self.feedback.abort_all();
        while self.jobs.join_next().await.is_some() {}
        while self.feedback.join_next().await.is_some() {}
        // All workers are joined BEFORE any persistence error can return.
        for id in self.active.keys() {
            if self.service.cancel(&mut self.runtime, id).is_err() {
                outcome = Err(HostError::Storage);
            }
        }
        self.active.clear();
        if let Some(claim) = self.feedback_claim.take()
            && self
                .runtime
                .defer_service_receipt(&claim, self.clock.now_ms(), true, 250)
                .is_err()
        {
            outcome = Err(HostError::Storage);
        }
        if self.runtime.sync_service_receipts().is_err() {
            outcome = Err(HostError::Storage);
        }
        outcome
    }
}
