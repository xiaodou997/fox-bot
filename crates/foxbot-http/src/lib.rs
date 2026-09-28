//! HTTP reply adapters with immutable request identity and an independent feedback outbox.
//! This G1 library is exercised with synthetic data; native senders and encrypted storage are absent.
#![forbid(unsafe_code)]
mod config;
mod protocol;
mod transport;

pub use config::*;
use foxbot_core::{
    ConversationKey, ProviderProfile, ReceiptClaim, ReplyRequest, ReplyResponse, Runtime,
};
use reqwest::{
    Client,
    header::{AUTHORIZATION, HeaderValue},
};
use sha2::{Digest, Sha256};
use std::{
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::Semaphore;
pub use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum HttpError {
    #[error("invalid HTTP provider configuration")]
    Config,
    #[error("HTTP transport failed; server execution may be unknown")]
    Transport,
    #[error("HTTP deadline exceeded; server execution may be unknown")]
    Timeout,
    #[error("request cancelled locally; server execution is not assumed cancelled")]
    Cancelled,
    #[error("HTTP status {code}")]
    Status {
        code: u16,
        retry_after_ms: Option<u64>,
    },
    #[error("invalid or incomplete reply protocol")]
    InvalidResponse,
    #[error("HTTP payload exceeds configured limit")]
    BodyTooLarge,
    #[error("response or provider configuration is stale")]
    Stale,
    #[error("local runtime rejected the operation")]
    Core,
}
pub type Result<T> = std::result::Result<T, HttpError>;
impl From<foxbot_core::Error> for HttpError {
    fn from(error: foxbot_core::Error) -> Self {
        if matches!(error, foxbot_core::Error::Stale) {
            Self::Stale
        } else {
            Self::Core
        }
    }
}

/// Epoch milliseconds for durable timestamps, advancing monotonically within this process.
/// Across restart the runtime still rejects a clock earlier than task creation.
pub struct RunClock {
    epoch: u64,
    start: Instant,
}
impl Default for RunClock {
    fn default() -> Self {
        Self {
            epoch: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
                .try_into()
                .unwrap_or(u64::MAX),
            start: Instant::now(),
        }
    }
}
impl RunClock {
    pub fn now_ms(&self) -> u64 {
        self.epoch.saturating_add(
            self.start
                .elapsed()
                .as_millis()
                .try_into()
                .unwrap_or(u64::MAX),
        )
    }
}

#[derive(Clone)]
pub struct HttpReplyService {
    config: HttpConfig,
    client: Client,
    authorization: Option<HeaderValue>,
    profile_tag: String,
    slots: Arc<Semaphore>,
}

/// Owned, immutable and deliberately not Clone. A job is run once; controlled retries
/// occur inside that run with identical bytes and the same idempotency key.
pub struct HttpJob {
    request: ReplyRequest,
    wire: Vec<u8>,
    profile_tag: String,
    conversation_ref: String,
}
impl HttpJob {
    pub fn request_id(&self) -> &str {
        &self.request.request_id
    }
}
pub struct HttpCompletion {
    job: HttpJob,
    result: Result<ReplyResponse>,
    cancellation: CancellationToken,
}
pub struct FeedbackCompletion {
    claim: ReceiptClaim,
    profile_tag: String,
    result: Result<()>,
}

impl HttpReplyService {
    /// Token comes from the embedding application's secret store; it is never serialized
    /// into configuration or the ledger, and no lower-level error with URL/body is exposed.
    pub fn new(config: HttpConfig, bearer_token: Option<&str>) -> Result<Self> {
        config.validate()?;
        let authorization = bearer_token
            .map(|token| {
                if token.is_empty() || token.len() > 8192 || token.chars().any(char::is_control) {
                    return Err(HttpError::Config);
                }
                let mut header = HeaderValue::from_str(&format!("Bearer {token}"))
                    .map_err(|_| HttpError::Config)?;
                header.set_sensitive(true);
                Ok(header)
            })
            .transpose()?;
        // Fence semantic routing and credentials, NOT operational timeout/concurrency tuning:
        // increasing a timeout must still allow the original receipt to be acknowledged.
        let mut hash = Sha256::new();
        hash.update(serde_json::to_vec(&serde_json::json!({
            "protocol":config.protocol,"endpoint":config.endpoint,"model":config.model,
            "context_mode":config.context_mode,"receipt_endpoint":config.receipt_endpoint,
            "idempotency_supported":config.idempotency_supported,"staging_contract":config.staging_contract
        })).map_err(|_| HttpError::Config)?);
        hash.update([0]);
        hash.update(bearer_token.unwrap_or_default().as_bytes());
        let profile_tag = format!("{:x}", hash.finalize());
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(Duration::from_millis(config.attempt_timeout_ms.min(10_000)))
            .timeout(Duration::from_millis(config.attempt_timeout_ms))
            .pool_max_idle_per_host(2)
            .build()
            .map_err(|_| HttpError::Config)?;
        let slots = Arc::new(Semaphore::new(config.max_in_flight));
        Ok(Self {
            config,
            client,
            authorization,
            profile_tag,
            slots,
        })
    }

    pub fn profile_tag(&self) -> &str {
        &self.profile_tag
    }

    /// No await and no networking. A persisted exchange exists BEFORE the server sees anything.
    pub fn begin(
        &self,
        runtime: &mut Runtime,
        key: &ConversationKey,
        now_ms: u64,
        user_request: Option<&str>,
    ) -> Result<Option<HttpJob>> {
        if !runtime.service_ready(key)? {
            return Ok(None);
        }
        let Some(request) = runtime.begin_reply(key, now_ms, user_request)? else {
            return Ok(None);
        };
        // Wire protocol and prompt policy are independent: a preconfigured KB service
        // can expose Chat Completions without accepting an additional system prompt.
        // BusinessV1 must not silently discard an explicitly configured generic prompt.
        if self.config.protocol == Protocol::BusinessV1 && request.system_prompt.is_some() {
            runtime.fail_reply(&request.request_id)?;
            return Err(HttpError::Config);
        }
        let scope = format!("{}:{}", self.profile_tag, request.conversation_ref);
        let conversation_ref = format!("{:x}", Sha256::digest(scope.as_bytes()));
        let wire = match protocol::encode(&self.config, &request, &conversation_ref) {
            Ok(wire) => wire,
            Err(error) => {
                runtime.fail_reply(&request.request_id)?;
                return Err(error);
            }
        };
        runtime.track_service_exchange(
            &request.request_id,
            &self.profile_tag,
            &conversation_ref,
            self.config.context_mode == ContextMode::ServiceManaged,
            self.config.receipt_endpoint.is_some(),
        )?;
        Ok(Some(HttpJob {
            request,
            wire,
            profile_tag: self.profile_tag.clone(),
            conversation_ref,
        }))
    }

    /// Await without borrowing Runtime: the host can ingest newer observations, pause,
    /// cancel, or change a binding while this request is in flight. No detached workers.
    pub async fn run(&self, job: HttpJob, cancellation: CancellationToken) -> HttpCompletion {
        let result = if job.profile_tag != self.profile_tag {
            Err(HttpError::Stale)
        } else {
            self.post(
                &self.config.endpoint,
                &job.wire,
                &job.request.request_id,
                self.config.max_attempts,
                self.config.idempotency_supported,
                &cancellation,
            )
            .await
            .and_then(|body| {
                protocol::decode(&self.config, &job.request, &job.conversation_ref, &body)
            })
        };
        HttpCompletion {
            job,
            result,
            cancellation,
        }
    }

    /// A fresh host timestamp, not the pre-request timestamp, enforces expiry on arrival.
    /// Local cancellation wins over a simultaneous successful HTTP response.
    pub fn finish(
        &self,
        runtime: &mut Runtime,
        completion: HttpCompletion,
        now_ms: u64,
    ) -> Result<String> {
        let HttpCompletion {
            job,
            mut result,
            cancellation,
        } = completion;
        if job.profile_tag != self.profile_tag {
            return Err(HttpError::Stale);
        }
        runtime.check_service_exchange(&job.request.request_id, &self.profile_tag)?;
        if cancellation.is_cancelled() {
            result = Err(HttpError::Cancelled);
        }
        let result = match result {
            Ok(response) => runtime
                .accept_reply(&job.request.request_id, &response, now_ms)
                .map_err(Into::into),
            Err(error) => {
                if runtime.task_state(&job.request.request_id)? == "GENERATING" {
                    runtime.fail_reply(&job.request.request_id)?;
                }
                Err(error)
            }
        };
        runtime.sync_service_receipts()?;
        result?;
        Ok(job.request.request_id)
    }

    /// Cancel a task whose future was dropped by its host. Server cancellation is a
    /// separate durable tombstone/receipt, not inferred from dropping a Rust future.
    pub fn cancel(&self, runtime: &mut Runtime, request_id: &str) -> Result<()> {
        runtime.check_service_exchange(request_id, &self.profile_tag)?;
        if runtime.task_state(request_id)? == "GENERATING" {
            runtime.fail_reply(request_id)?;
        }
        runtime.sync_service_receipts()?;
        Ok(())
    }

    pub fn begin_feedback(
        &self,
        runtime: &mut Runtime,
        now_ms: u64,
    ) -> Result<Option<ReceiptClaim>> {
        if self.config.receipt_endpoint.is_none() {
            return Ok(None);
        }
        Ok(runtime.claim_service_receipt(&self.profile_tag, now_ms)?)
    }

    pub async fn run_feedback(
        &self,
        claim: ReceiptClaim,
        cancellation: CancellationToken,
    ) -> FeedbackCompletion {
        let result = async {
            if claim.profile_tag() != self.profile_tag {
                return Err(HttpError::Stale);
            }
            let url = self
                .config
                .receipt_endpoint
                .as_ref()
                .ok_or(HttpError::Config)?;
            let body =
                serde_json::to_vec(&claim.receipt).map_err(|_| HttpError::InvalidResponse)?;
            // One wire attempt per durable claim. Retry scheduling belongs to the ledger.
            let bytes = self
                .post(
                    url,
                    &body,
                    &claim.receipt.receipt_id,
                    1,
                    true,
                    &cancellation,
                )
                .await?;
            let ack: protocol::FeedbackAck =
                serde_json::from_slice(&bytes).map_err(|_| HttpError::InvalidResponse)?;
            if ack.schema_version != "0.1"
                || !ack.accepted
                || ack.receipt_id != claim.receipt.receipt_id
                || ack.revision != claim.receipt.revision
            {
                return Err(HttpError::InvalidResponse);
            }
            Ok(())
        }
        .await;
        FeedbackCompletion {
            claim,
            profile_tag: self.profile_tag.clone(),
            result,
        }
    }

    pub fn finish_feedback(
        &self,
        runtime: &mut Runtime,
        completion: FeedbackCompletion,
        now_ms: u64,
    ) -> Result<()> {
        if completion.profile_tag != self.profile_tag {
            return Err(HttpError::Stale);
        }
        runtime.check_service_exchange(&completion.claim.receipt.request_id, &self.profile_tag)?;
        match completion.result {
            Ok(()) => runtime.acknowledge_service_receipt(&completion.claim)?,
            Err(error) => {
                runtime.defer_service_receipt(
                    &completion.claim,
                    now_ms,
                    error.retryable() || error == HttpError::Cancelled,
                    error.retry_after_ms(),
                )?;
                return Err(error);
            }
        }
        Ok(())
    }

    pub fn required_provider_profile(&self, system_prompt: String) -> ProviderProfile {
        match self.config.protocol {
            Protocol::BusinessV1 => ProviderProfile::Custom,
            Protocol::ChatCompletions => ProviderProfile::Generic { system_prompt },
        }
    }
}
