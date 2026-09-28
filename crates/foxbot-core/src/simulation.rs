//! Deterministic synthetic fixtures; these adapters never access other software.
use crate::*;

pub fn fixture_key() -> ConversationKey {
    ConversationKey {
        device: "sim-device".into(),
        app_instance: "sim-app".into(),
        account_binding: "sim-account".into(),
        conversation: "sim-conversation".into(),
    }
}

pub fn fixture_observation(
    key: &ConversationKey,
    event: &str,
    text: &str,
    time_ms: u64,
) -> Observation {
    Observation {
        key: key.clone(),
        identity_epoch: 1,
        source: Source::Synthetic,
        source_event_id: event.into(),
        historical: false,
        observed_ms: time_ms,
        message: Message {
            canonical_id: Some(event.into()),
            sender: Some("synthetic-sender".into()),
            direction: Direction::Incoming,
            mention: Mention::Absent,
            kind: ContentKind::Text,
            text: Some(text.into()),
            complete: true,
            reply_to: None,
        },
    }
}

pub struct FixedReply {
    pub outcome: ReplyOutcome,
    pub calls: usize,
    pub requests: Vec<ReplyRequest>,
}

impl FixedReply {
    pub fn new(text: &str) -> Self {
        Self {
            outcome: ReplyOutcome::Reply { text: text.into() },
            calls: 0,
            requests: Vec::new(),
        }
    }
}

impl ReplyProvider for FixedReply {
    fn generate(&mut self, request: &ReplyRequest) -> Result<ReplyResponse> {
        self.calls += 1;
        self.requests.push(request.clone());
        Ok(ReplyResponse::for_request(request, self.outcome.clone()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    None,
    FillNoEffect,
    MoveAfterFill,
    EditAfterFill,
    MessageAfterFill,
    CommitThenUnknown,
    SubmitOnly,
    WrongReceipt,
}

pub struct MockChannel {
    pub live: LiveTarget,
    pub fault: Fault,
    pub sent: Vec<String>,
    pub fill_calls: usize,
    pub send_calls: usize,
}

impl MockChannel {
    pub fn new(key: &ConversationKey) -> Self {
        Self {
            live: LiveTarget {
                key: key.clone(),
                identity_epoch: 1,
                window_ref: "sim-window".into(),
                editor_ref: "sim-editor".into(),
                layout_revision: 1,
                draft: Draft::Empty,
                conversation_changed: false,
                composing: false,
                user_active: false,
                permitted: true,
            },
            fault: Fault::None,
            sent: Vec::new(),
            fill_calls: 0,
            send_calls: 0,
        }
    }
}

impl MessageChannel for MockChannel {
    fn inspect(&mut self, _target: &ConversationKey) -> Result<LiveTarget> {
        Ok(self.live.clone())
    }

    fn fill(&mut self, action: &OutboundAction, expected: &LiveTarget) -> Result<()> {
        if !self.live.same_surface(expected)
            || !self.live.accepts(action)
            || self.live.draft != Draft::Empty
        {
            return Err(Error::Blocked("mock fill guard"));
        }
        self.fill_calls += 1;
        if self.fault != Fault::FillNoEffect {
            self.live.draft = Draft::Text(action.text.clone());
        }
        if self.fault == Fault::MoveAfterFill {
            self.live.layout_revision += 1;
        }
        if self.fault == Fault::MessageAfterFill {
            self.live.conversation_changed = true;
        }
        if self.fault == Fault::EditAfterFill {
            self.live.draft = Draft::Text("synthetic human edit".into());
        }
        Ok(())
    }

    fn send(&mut self, action: &OutboundAction, expected: &LiveTarget) -> Result<SendEvidence> {
        if !self.live.same_surface(expected)
            || !self.live.accepts(action)
            || self.live.draft != Draft::Text(action.text.clone())
        {
            return Err(Error::Blocked("mock send guard"));
        }
        self.send_calls += 1;
        self.sent.push(action.action_id.clone());
        self.live.draft = Draft::Empty;
        Ok(match self.fault {
            Fault::CommitThenUnknown => SendEvidence::Unknown,
            Fault::SubmitOnly => SendEvidence::Submitted,
            Fault::WrongReceipt => SendEvidence::ObservedOutgoing {
                action_id: "unrelated-old-action".into(),
            },
            _ => SendEvidence::ObservedOutgoing {
                action_id: action.action_id.clone(),
            },
        })
    }

    fn reconcile(&mut self, action: &OutboundAction) -> Result<SendEvidence> {
        Ok(if self.sent.contains(&action.action_id) {
            SendEvidence::ObservedOutgoing {
                action_id: action.action_id.clone(),
            }
        } else {
            SendEvidence::Unknown
        })
    }
}
