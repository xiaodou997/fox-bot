use crate::{Error, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationKey {
    pub device: String,
    pub app_instance: String,
    pub account_binding: String,
    pub conversation: String,
}

impl ConversationKey {
    pub(crate) fn encoded(&self) -> Result<String> {
        for field in [
            &self.device,
            &self.app_instance,
            &self.account_binding,
            &self.conversation,
        ] {
            valid_id(field)?;
        }
        Ok(serde_json::to_string(self)?)
    }
}

pub(crate) fn valid_id(value: &str) -> Result<()> {
    if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        return Err(Error::Invalid("opaque identifier"));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Assisted,
    AutoSuggest,
    AutoReply,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationKind {
    Private,
    Group,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProviderProfile {
    Custom,
    Generic { system_prompt: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub key: ConversationKey,
    pub title: String,
    pub identity_epoch: u64,
    pub profile_version: u64,
    pub provider: ProviderProfile,
    pub mode: Mode,
    pub kind: ConversationKind,
    pub group_all_messages: bool,
    pub enabled: bool,
    pub quiet_ms: u64,
    pub max_wait_ms: u64,
    pub reply_ttl_ms: u64,
    pub max_pending: usize,
    pub max_batch: usize,
    pub max_reply_chars: usize,
    pub max_auto_sends: u32,
}

impl Binding {
    /// New bindings are paused. Enabling one is an explicit operator action.
    pub fn paused(key: ConversationKey) -> Self {
        Self {
            key,
            title: String::new(),
            identity_epoch: 1,
            profile_version: 1,
            provider: ProviderProfile::Custom,
            mode: Mode::AutoReply,
            kind: ConversationKind::Private,
            group_all_messages: false,
            enabled: false,
            quiet_ms: 500,
            max_wait_ms: 2_000,
            reply_ttl_ms: 60_000,
            max_pending: 64,
            max_batch: 16,
            max_reply_chars: 4096,
            max_auto_sends: 20,
        }
    }
    pub fn validate(&self) -> Result<()> {
        self.key.encoded()?;
        if self.identity_epoch == 0
            || self.identity_epoch > i64::MAX as u64
            || self.profile_version == 0
            || self.profile_version > i64::MAX as u64
            || self.max_pending == 0
            || self.max_pending > 1024
            || self.max_batch == 0
            || self.max_batch > self.max_pending
            || self.max_reply_chars == 0
            || self.max_reply_chars > 16_384
            || self.max_auto_sends == 0
            || self.quiet_ms > self.max_wait_ms
            || self.max_wait_ms > 60_000
            || self.reply_ttl_ms == 0
            || self.reply_ttl_ms > 600_000
            || self.title.len() > 1024
        {
            return Err(Error::Invalid("binding limits"));
        }
        if let ProviderProfile::Generic { system_prompt } = &self.provider
            && system_prompt.len() > 16_384
        {
            return Err(Error::Invalid("system prompt size"));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Incoming,
    Own,
    Unknown,
    System,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mention {
    Verified,
    Absent,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Synthetic,
    UiTree,
    Ocr,
    Notification,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentKind {
    Text,
    Image,
    Voice,
    File,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Message {
    /// Only the trusted adapter may assert equivalence across capture sources.
    /// None is deliberately not replaced with a text hash.
    pub canonical_id: Option<String>,
    pub sender: Option<String>,
    pub direction: Direction,
    pub mention: Mention,
    pub kind: ContentKind,
    pub text: Option<String>,
    pub complete: bool,
    pub reply_to: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub key: ConversationKey,
    pub identity_epoch: u64,
    pub source: Source,
    pub source_event_id: String,
    pub historical: bool,
    pub observed_ms: u64,
    pub message: Message,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IngestOutcome {
    Baseline,
    Queued,
    Ignored,
    Duplicate,
    Ambiguous,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputEvent {
    pub event_id: String,
    pub message: Message,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplyRequest {
    pub request_id: String,
    pub conversation_ref: String,
    pub session_revision: u64,
    pub provider_profile_version: u64,
    pub input_events: Vec<InputEvent>,
    /// Bounded observed context, not a claim to the full conversation history.
    pub context: Vec<InputEvent>,
    pub context_complete: bool,
    /// Custom business services always receive None here.
    pub system_prompt: Option<String>,
    pub user_request: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReplyOutcome {
    Reply { text: String },
    NoReply,
    Handoff { reason: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplyResponse {
    pub request_id: String,
    pub in_reply_to: Vec<String>,
    pub complete: bool,
    pub outcome: ReplyOutcome,
}

impl ReplyResponse {
    pub fn for_request(request: &ReplyRequest, outcome: ReplyOutcome) -> Self {
        Self {
            request_id: request.request_id.clone(),
            in_reply_to: request
                .input_events
                .iter()
                .map(|e| e.event_id.clone())
                .collect(),
            complete: true,
            outcome,
        }
    }
}

pub trait ReplyProvider {
    /// A G1 provider cannot change local routing. No retry is implied on error.
    fn generate(&mut self, request: &ReplyRequest) -> Result<ReplyResponse>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ActionState {
    Prepared,
    Executing,
    Submitted,
    VerifiedOutgoing,
    Unknown,
    Stale,
    Blocked,
}

impl ActionState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Prepared => "PREPARED",
            Self::Executing => "EXECUTING",
            Self::Submitted => "SUBMITTED",
            Self::VerifiedOutgoing => "VERIFIED_OUTGOING",
            Self::Unknown => "UNKNOWN",
            Self::Stale => "STALE",
            Self::Blocked => "BLOCKED",
        }
    }
    pub(crate) fn parse(s: &str) -> Result<Self> {
        match s {
            "PREPARED" => Ok(Self::Prepared),
            "EXECUTING" => Ok(Self::Executing),
            "SUBMITTED" => Ok(Self::Submitted),
            "VERIFIED_OUTGOING" => Ok(Self::VerifiedOutgoing),
            "UNKNOWN" => Ok(Self::Unknown),
            "STALE" => Ok(Self::Stale),
            "BLOCKED" => Ok(Self::Blocked),
            _ => Err(Error::Schema),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OutboundAction {
    pub action_id: String,
    pub request_id: String,
    pub target: ConversationKey,
    pub identity_epoch: u64,
    pub session_revision: u64,
    pub profile_version: u64,
    pub text: String,
    pub approved_by_user: bool,
    pub created_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Draft {
    Empty,
    Text(String),
    Unreadable,
}

/// Fresh execution facts for an unattended, exclusively operated chat surface.
/// Human co-editing and IME state are outside this contract; development handoff
/// uses explicit host pause/resume, not input-activity inference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveTarget {
    pub key: ConversationKey,
    pub identity_epoch: u64,
    /// Opaque platform process/login-session evidence. Never derived from chat text.
    pub application_session_ref: String,
    /// Opaque current-conversation surface evidence, e.g. a bound visual fingerprint.
    pub conversation_surface_ref: String,
    pub window_ref: String,
    pub editor_ref: String,
    pub layout_revision: u64,
    pub draft: Draft,
    /// A fresh adapter observation reports changed message context during execution.
    pub conversation_changed: bool,
    pub permitted: bool,
    pub frontmost: bool,
}

impl LiveTarget {
    pub(crate) fn same_surface(&self, other: &Self) -> bool {
        self.key == other.key
            && self.identity_epoch == other.identity_epoch
            && self.application_session_ref == other.application_session_ref
            && self.conversation_surface_ref == other.conversation_surface_ref
            && self.window_ref == other.window_ref
            && self.editor_ref == other.editor_ref
            && self.layout_revision == other.layout_revision
    }
    pub(crate) fn accepts(&self, action: &OutboundAction) -> bool {
        self.key == action.target
            && self.identity_epoch == action.identity_epoch
            && !self.conversation_changed
            && self.permitted
            && self.frontmost
            && !self.application_session_ref.is_empty()
            && !self.conversation_surface_ref.is_empty()
            && !self.window_ref.is_empty()
            && !self.editor_ref.is_empty()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SendGatePhase {
    BeforeFill,
    BeforeSend,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SendGateBlocker {
    HostPaused,
    ActionState,
    StaleAction,
    AttemptBudget,
    UnresolvedPriorSend,
    TargetMismatch,
    IdentityEpochMismatch,
    ApplicationSessionMissing,
    ConversationSurfaceMissing,
    SurfaceMissing,
    NotFrontmost,
    ConversationChanged,
    NotPermitted,
    DraftNotEmpty,
    DraftMismatch,
    SurfaceChanged,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SendGateReport {
    pub phase: SendGatePhase,
    pub allowed: bool,
    pub blockers: Vec<SendGateBlocker>,
}

#[derive(Clone, Debug)]
pub enum SendEvidence {
    Submitted,
    /// A matching new outgoing message, NOT delivery or read confirmation.
    ObservedOutgoing {
        action_id: String,
    },
    Unknown,
}

pub trait MessageChannel {
    fn inspect(&mut self, target: &ConversationKey) -> Result<LiveTarget>;
    /// Recheck the expected surface immediately before any native write.
    fn fill(&mut self, action: &OutboundAction, expected: &LiveTarget) -> Result<()>;
    /// Recheck surface AND text; a generic Enter implementation is not acceptable.
    fn send(&mut self, action: &OutboundAction, expected: &LiveTarget) -> Result<SendEvidence>;
    /// Read-only reconciliation. Never sends or interprets absence as non-delivery.
    fn reconcile(&mut self, action: &OutboundAction) -> Result<SendEvidence>;
}

#[derive(Clone, Debug, Serialize)]
pub struct Summary {
    pub messages: u64,
    pub tasks: u64,
    pub actions: Vec<(String, ActionState)>,
}
