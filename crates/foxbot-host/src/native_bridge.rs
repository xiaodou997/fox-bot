use crate::{HostError, Result};
use foxbot_core::{
    Binding, ContentKind, ConversationKind, Direction, IngestOutcome, Mention, Message,
    Observation, Runtime, Source,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

const SNAPSHOT_SCHEMA: &str = "foxbot.private-message-snapshot.v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PrivateDirection {
    Me,
    Them,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrivateBridgeMessage {
    pub text: String,
    pub direction: PrivateDirection,
    pub sender_fingerprint: Option<String>,
    pub complete: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrivateMessageSnapshot {
    pub schema_version: String,
    pub strategy: String,
    pub application_session_fingerprint: String,
    pub conversation_fingerprint: String,
    pub partial_reasons: Vec<String>,
    pub messages: Vec<PrivateBridgeMessage>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroundTruthAcceptance {
    pub schema_version: String,
    pub strategy: String,
    pub revision: String,
    pub accepted: bool,
    pub cases: u32,
    pub labeled_messages: u32,
    pub covered_tags: Vec<String>,
    pub direction_errors: u32,
    pub sender_errors: u32,
    pub message_count_errors: u32,
    pub text_errors: u32,
    #[serde(default)]
    pub text_edit_distance: u32,
    #[serde(default)]
    pub text_expected_characters: u32,
    #[serde(default)]
    pub text_error_rate_bp: u32,
}
impl GroundTruthAcceptance {
    pub fn validate(&self) -> Result<()> {
        let required = [
            "private",
            "group",
            "duplicate_text",
            "numeric",
            "multiline",
            "reference",
        ];
        let tags: HashSet<_> = self.covered_tags.iter().map(String::as_str).collect();
        let text_valid = if self.text_expected_characters > 0 {
            self.text_edit_distance.saturating_mul(100)
                <= self.text_expected_characters.saturating_mul(2)
                && self.text_error_rate_bp <= 200
        } else {
            // Backward compatibility for pre-CER acceptance receipts.
            self.text_edit_distance == 0
                && self.text_error_rate_bp == 0
                && self.text_errors.saturating_mul(100) <= self.labeled_messages.saturating_mul(2)
        };
        let valid = self.schema_version == "foxbot.g2c-ground-truth-result.v1"
            && !self.strategy.is_empty()
            && self.strategy.len() <= 128
            && !self.revision.is_empty()
            && self.revision.len() <= 128
            && self.cases >= 6
            && self.labeled_messages >= 24
            && required.into_iter().all(|tag| tags.contains(tag))
            && self.direction_errors == 0
            && self.sender_errors == 0
            && self.message_count_errors == 0
            && text_valid;
        if !valid || self.accepted != valid {
            return Err(HostError::Untrusted);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeConversationBinding {
    pub application_session_fingerprint: String,
    pub conversation_fingerprint: String,
    pub identity_source: String,
    pub binding: Binding,
}
impl NativeConversationBinding {
    /// An operator has explicitly bound the currently visible conversation to this
    /// durable Binding. The visual fingerprint is evidence, not identity by itself.
    pub fn from_current_snapshot(
        snapshot: &PrivateMessageSnapshot,
        binding: Binding,
    ) -> Result<Self> {
        binding.validate().map_err(|_| HostError::Config)?;
        if snapshot.schema_version != SNAPSHOT_SCHEMA
            || !is_hex64(&snapshot.application_session_fingerprint)
            || !is_hex64(&snapshot.conversation_fingerprint)
        {
            return Err(HostError::Untrusted);
        }
        Ok(Self {
            application_session_fingerprint: snapshot.application_session_fingerprint.clone(),
            conversation_fingerprint: snapshot.conversation_fingerprint.clone(),
            identity_source: "configured_wechat_title_continuity_v1".into(),
            binding,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeBridgeConfig {
    pub schema_version: u32,
    pub acceptance: GroundTruthAcceptance,
    pub conversations: Vec<NativeConversationBinding>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Signature {
    text: String,
    direction: PrivateDirection,
    sender: Option<String>,
    complete: bool,
}

#[derive(Clone, Debug)]
struct TrackState {
    signatures: Vec<Signature>,
    ids: Vec<String>,
    next: u64,
}

#[derive(Clone, Debug)]
pub enum BridgeOutcome {
    Baseline(Vec<Observation>),
    New(Vec<Observation>),
    NoChange,
    Provisional,
    Ambiguous,
}

#[derive(Clone)]
pub struct NativeObservationBridge {
    conversations: HashMap<String, NativeConversationBinding>,
    acceptance: GroundTruthAcceptance,
    tracks: HashMap<String, TrackState>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct RuntimeBridgeReport {
    pub bridge_state: String,
    pub observations: usize,
    pub baselined: usize,
    pub queued: usize,
    pub ignored: usize,
    pub duplicates: usize,
    pub ambiguous: usize,
}
impl NativeObservationBridge {
    pub fn from_config(config: NativeBridgeConfig) -> Result<Self> {
        if config.schema_version != 1 {
            return Err(HostError::Config);
        }
        Self::new(config.conversations, config.acceptance)
    }

    pub fn new(
        entries: Vec<NativeConversationBinding>,
        acceptance: GroundTruthAcceptance,
    ) -> Result<Self> {
        acceptance.validate()?;
        let mut conversations = HashMap::new();
        for entry in entries {
            entry.binding.validate().map_err(|_| HostError::Config)?;
            if !is_hex64(&entry.application_session_fingerprint)
                || !is_hex64(&entry.conversation_fingerprint)
                || entry.identity_source != "configured_wechat_title_continuity_v1"
                || conversations
                    .insert(entry.conversation_fingerprint.clone(), entry)
                    .is_some()
            {
                return Err(HostError::Config);
            }
        }
        if conversations.is_empty() {
            return Err(HostError::Config);
        }
        Ok(Self {
            conversations,
            acceptance,
            tracks: HashMap::new(),
        })
    }

    pub fn bridge(
        &mut self,
        snapshot: &PrivateMessageSnapshot,
        observed_ms: u64,
    ) -> Result<BridgeOutcome> {
        if snapshot.schema_version != SNAPSHOT_SCHEMA
            || snapshot.strategy != self.acceptance.strategy
            || !is_hex64(&snapshot.application_session_fingerprint)
            || !is_hex64(&snapshot.conversation_fingerprint)
            || snapshot.messages.len() > 64
            || snapshot
                .partial_reasons
                .iter()
                .any(|v| v != "HEURISTIC_REGION")
        {
            return Ok(BridgeOutcome::Provisional);
        }
        let Some(bound) = self
            .conversations
            .get(&snapshot.conversation_fingerprint)
            .cloned()
        else {
            return Ok(BridgeOutcome::Provisional);
        };
        if bound.application_session_fingerprint != snapshot.application_session_fingerprint {
            self.tracks.remove(&snapshot.conversation_fingerprint);
            return Ok(BridgeOutcome::Provisional);
        }
        let binding = bound.binding;
        let signatures: Vec<_> = snapshot
            .messages
            .iter()
            .map(signature)
            .collect::<Result<_>>()?;
        if signatures
            .iter()
            .any(|m| m.direction == PrivateDirection::Unknown || !m.complete)
        {
            return Ok(BridgeOutcome::Provisional);
        }
        if binding.kind == ConversationKind::Group
            && signatures
                .iter()
                .any(|m| m.direction == PrivateDirection::Them && m.sender.is_none())
        {
            return Ok(BridgeOutcome::Provisional);
        }
        let track_key = snapshot.conversation_fingerprint.clone();
        let Some(previous) = self.tracks.get_mut(&track_key) else {
            let (ids, observations) = baseline(&binding, &track_key, &signatures, observed_ms);
            self.tracks.insert(
                track_key,
                TrackState {
                    signatures,
                    ids,
                    next: observations.len() as u64,
                },
            );
            return Ok(BridgeOutcome::Baseline(observations));
        };
        if previous.signatures == signatures {
            return Ok(BridgeOutcome::NoChange);
        }
        let max = previous.signatures.len().min(signatures.len());
        let overlap = (1..=max)
            .rev()
            .find(|n| previous.signatures[previous.signatures.len() - *n..] == signatures[..*n]);
        let Some(overlap) = overlap else {
            return Ok(BridgeOutcome::Ambiguous);
        };
        // A single generic line such as "好的" is not enough continuity to distinguish
        // two same-title conversations. Grow a tiny baseline without emitting work.
        if overlap < 2 {
            if previous.signatures.len() < 2
                && overlap == previous.signatures.len()
                && signatures.len() > previous.signatures.len()
            {
                let (ids, observations) = baseline(&binding, &track_key, &signatures, observed_ms);
                previous.signatures = signatures;
                previous.ids = ids;
                previous.next = observations.len() as u64;
                return Ok(BridgeOutcome::Baseline(observations));
            }
            return Ok(BridgeOutcome::Ambiguous);
        }
        let mut ids = previous.ids[previous.ids.len() - overlap..].to_vec();
        let mut observations = Vec::new();
        for item in &signatures[overlap..] {
            let id = canonical(&track_key, binding.identity_epoch, previous.next);
            previous.next += 1;
            ids.push(id.clone());
            observations.push(observation(&binding, item, id, observed_ms, false));
        }
        previous.signatures = signatures;
        previous.ids = ids;
        if observations.is_empty() {
            Ok(BridgeOutcome::NoChange)
        } else {
            Ok(BridgeOutcome::New(observations))
        }
    }

    /// Bridge into Runtime without advancing the cross-frame cursor until every
    /// generated Observation has been accepted. Runtime may have durably ingested
    /// a prefix before a later error; retrying reuses the same canonical IDs so
    /// that prefix is observed as Duplicate instead of being emitted twice.
    pub fn ingest_into_runtime(
        &mut self,
        runtime: &mut Runtime,
        snapshot: &PrivateMessageSnapshot,
        observed_ms: u64,
    ) -> Result<RuntimeBridgeReport> {
        let mut candidate = self.clone();
        let outcome = candidate.bridge(snapshot, observed_ms)?;
        let mut report = RuntimeBridgeReport::default();
        let observations = match outcome {
            BridgeOutcome::Baseline(values) => {
                report.bridge_state = "BASELINE".into();
                values
            }
            BridgeOutcome::New(values) => {
                report.bridge_state = "NEW".into();
                values
            }
            BridgeOutcome::NoChange => {
                report.bridge_state = "NO_CHANGE".into();
                *self = candidate;
                return Ok(report);
            }
            BridgeOutcome::Provisional => {
                report.bridge_state = "PROVISIONAL".into();
                *self = candidate;
                return Ok(report);
            }
            BridgeOutcome::Ambiguous => {
                report.bridge_state = "AMBIGUOUS".into();
                *self = candidate;
                return Ok(report);
            }
        };
        report.observations = observations.len();
        for observation in &observations {
            let outcome = runtime.ingest(observation).map_err(|error| match error {
                foxbot_core::Error::Backpressure => HostError::Backpressure,
                foxbot_core::Error::Stale => HostError::Untrusted,
                _ => HostError::Storage,
            })?;
            match outcome {
                IngestOutcome::Baseline => report.baselined += 1,
                IngestOutcome::Queued => report.queued += 1,
                IngestOutcome::Ignored => report.ignored += 1,
                IngestOutcome::Duplicate => report.duplicates += 1,
                IngestOutcome::Ambiguous => report.ambiguous += 1,
            }
        }
        *self = candidate;
        Ok(report)
    }

    /// Account/login changes, app replacement or an operator rebind must drop the
    /// old cross-frame tracker. The next accepted snapshot becomes a fresh baseline.
    pub fn invalidate(&mut self, conversation_fingerprint: &str) {
        self.tracks.remove(conversation_fingerprint);
    }
}

fn signature(message: &PrivateBridgeMessage) -> Result<Signature> {
    if message.text.trim().is_empty()
        || message.text.len() > 16_384
        || message
            .sender_fingerprint
            .as_ref()
            .is_some_and(|v| !is_hex64(v))
    {
        return Err(HostError::Untrusted);
    }
    Ok(Signature {
        text: message.text.clone(),
        direction: message.direction.clone(),
        sender: message.sender_fingerprint.clone(),
        complete: message.complete,
    })
}

fn baseline(
    binding: &Binding,
    fingerprint: &str,
    signatures: &[Signature],
    now: u64,
) -> (Vec<String>, Vec<Observation>) {
    let mut ids = Vec::new();
    let mut observations = Vec::new();
    for (index, item) in signatures.iter().enumerate() {
        let id = canonical(fingerprint, binding.identity_epoch, index as u64);
        ids.push(id.clone());
        observations.push(observation(binding, item, id, now, true));
    }
    (ids, observations)
}

fn observation(
    binding: &Binding,
    item: &Signature,
    id: String,
    observed_ms: u64,
    historical: bool,
) -> Observation {
    let direction = match item.direction {
        PrivateDirection::Me => Direction::Own,
        PrivateDirection::Them => Direction::Incoming,
        PrivateDirection::Unknown => Direction::Unknown,
    };
    let sender = match direction {
        Direction::Incoming if binding.kind == ConversationKind::Group => {
            item.sender.as_ref().map(|v| format!("sender:{v}"))
        }
        Direction::Incoming => Some(format!("peer:{}", binding.key.conversation)),
        _ => None,
    };
    Observation {
        key: binding.key.clone(),
        identity_epoch: binding.identity_epoch,
        source: Source::Ocr,
        source_event_id: id.clone(),
        historical,
        observed_ms,
        message: Message {
            canonical_id: Some(id),
            sender,
            direction,
            mention: Mention::Unknown,
            kind: ContentKind::Text,
            text: Some(item.text.clone()),
            complete: item.complete,
            reply_to: None,
        },
    }
}

fn canonical(fingerprint: &str, epoch: u64, sequence: u64) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"foxbot.ocr.message.v1\0");
    hasher.update(fingerprint.as_bytes());
    hasher.update(epoch.to_le_bytes());
    hasher.update(sequence.to_le_bytes());
    format!("ocr:{}", hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|v| format!("{v:02x}")).collect()
}

fn is_hex64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use foxbot_core::{Mode, ProviderProfile, simulation::fixture_key};
    use tempfile::TempDir;

    fn runtime(binding: &Binding) -> (TempDir, Runtime) {
        let directory = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
                .unwrap();
        }
        let mut runtime = Runtime::open_simulation(directory.path()).unwrap();
        runtime.bind(binding).unwrap();
        runtime.set_host_paused(false).unwrap();
        (directory, runtime)
    }

    fn acceptance() -> GroundTruthAcceptance {
        GroundTruthAcceptance {
            schema_version: "foxbot.g2c-ground-truth-result.v1".into(),
            strategy: "WECHAT_HEURISTIC_V0".into(),
            revision: "synthetic-v1".into(),
            accepted: true,
            cases: 6,
            labeled_messages: 24,
            covered_tags: vec![
                "private".into(),
                "group".into(),
                "duplicate_text".into(),
                "numeric".into(),
                "multiline".into(),
                "reference".into(),
            ],
            direction_errors: 0,
            sender_errors: 0,
            message_count_errors: 0,
            text_errors: 0,
            text_edit_distance: 0,
            text_expected_characters: 0,
            text_error_rate_bp: 0,
        }
    }

    fn entry(kind: ConversationKind) -> NativeConversationBinding {
        let mut binding = Binding::paused(fixture_key());
        binding.kind = kind;
        binding.mode = Mode::AutoReply;
        binding.provider = ProviderProfile::Custom;
        binding.enabled = true;
        NativeConversationBinding {
            application_session_fingerprint: "c".repeat(64),
            conversation_fingerprint: "a".repeat(64),
            identity_source: "configured_wechat_title_continuity_v1".into(),
            binding,
        }
    }

    fn msg(text: &str, direction: PrivateDirection) -> PrivateBridgeMessage {
        PrivateBridgeMessage {
            text: text.into(),
            direction,
            sender_fingerprint: Some("b".repeat(64)),
            complete: true,
        }
    }

    fn snapshot(messages: Vec<PrivateBridgeMessage>) -> PrivateMessageSnapshot {
        PrivateMessageSnapshot {
            schema_version: SNAPSHOT_SCHEMA.into(),
            strategy: "WECHAT_HEURISTIC_V0".into(),
            application_session_fingerprint: "c".repeat(64),
            conversation_fingerprint: "a".repeat(64),
            partial_reasons: vec!["HEURISTIC_REGION".into()],
            messages,
        }
    }

    #[test]
    fn acceptance_requires_coverage_and_zero_routing_errors() {
        acceptance().validate().unwrap();
        let mut bad = acceptance();
        bad.covered_tags.retain(|v| v != "numeric");
        assert!(matches!(bad.validate(), Err(HostError::Untrusted)));
        let mut bad = acceptance();
        bad.direction_errors = 1;
        assert!(matches!(bad.validate(), Err(HostError::Untrusted)));
    }

    #[test]
    fn explicit_binding_and_invalidation_force_a_fresh_baseline() {
        let snap = snapshot(vec![msg("A", PrivateDirection::Them)]);
        let configured = NativeConversationBinding::from_current_snapshot(
            &snap,
            entry(ConversationKind::Private).binding,
        )
        .unwrap();
        let fingerprint = configured.conversation_fingerprint.clone();
        assert_eq!(
            configured.identity_source,
            "configured_wechat_title_continuity_v1"
        );
        let mut bridge = NativeObservationBridge::new(vec![configured], acceptance()).unwrap();
        assert!(matches!(
            bridge.bridge(&snap, 1).unwrap(),
            BridgeOutcome::Baseline(_)
        ));
        bridge.invalidate(&fingerprint);
        assert!(matches!(
            bridge.bridge(&snap, 2).unwrap(),
            BridgeOutcome::Baseline(_)
        ));
    }

    #[test]
    fn duplicate_visual_fingerprint_is_rejected_instead_of_guessing() {
        let one = entry(ConversationKind::Private);
        let mut two = entry(ConversationKind::Private);
        two.binding.key.conversation = "other-conversation".into();
        let config = NativeBridgeConfig {
            schema_version: 1,
            acceptance: acceptance(),
            conversations: vec![one, two],
        };
        assert!(matches!(
            NativeObservationBridge::from_config(config),
            Err(HostError::Config)
        ));
    }

    #[test]
    fn first_snapshot_is_historical_then_sliding_overlap_emits_only_new_message() {
        let mut bridge =
            NativeObservationBridge::new(vec![entry(ConversationKind::Private)], acceptance())
                .unwrap();
        let first = snapshot(vec![
            msg("A", PrivateDirection::Them),
            msg("B", PrivateDirection::Me),
            msg("C", PrivateDirection::Them),
        ]);
        let BridgeOutcome::Baseline(base) = bridge.bridge(&first, 10).unwrap() else {
            panic!("expected baseline")
        };
        assert_eq!(base.len(), 3);
        assert!(base.iter().all(|o| o.historical));
        let old_last = base[2].message.canonical_id.clone().unwrap();

        let second = snapshot(vec![
            msg("B", PrivateDirection::Me),
            msg("C", PrivateDirection::Them),
            msg("D", PrivateDirection::Them),
        ]);
        let BridgeOutcome::New(new) = bridge.bridge(&second, 20).unwrap() else {
            panic!("expected new")
        };
        assert_eq!(new.len(), 1);
        assert_eq!(new[0].message.text.as_deref(), Some("D"));
        assert!(!new[0].historical);
        assert_ne!(
            new[0].message.canonical_id.as_deref(),
            Some(old_last.as_str())
        );
        assert_eq!(
            new[0].message.sender.as_deref(),
            Some("peer:sim-conversation")
        );
    }

    #[test]
    fn repeated_identical_text_can_be_a_new_message_without_text_hash_dedup() {
        let mut bridge =
            NativeObservationBridge::new(vec![entry(ConversationKind::Private)], acceptance())
                .unwrap();
        bridge
            .bridge(
                &snapshot(vec![
                    msg("上一条", PrivateDirection::Me),
                    msg("好的", PrivateDirection::Them),
                ]),
                10,
            )
            .unwrap();
        let BridgeOutcome::New(new) = bridge
            .bridge(
                &snapshot(vec![
                    msg("上一条", PrivateDirection::Me),
                    msg("好的", PrivateDirection::Them),
                    msg("好的", PrivateDirection::Them),
                ]),
                20,
            )
            .unwrap()
        else {
            panic!("expected new")
        };
        assert_eq!(new.len(), 1);
        assert_eq!(new[0].message.text.as_deref(), Some("好的"));
    }

    #[test]
    fn one_line_overlap_is_not_enough_to_route_same_title_conversations() {
        let mut bridge =
            NativeObservationBridge::new(vec![entry(ConversationKind::Private)], acceptance())
                .unwrap();
        bridge
            .bridge(
                &snapshot(vec![
                    msg("A", PrivateDirection::Me),
                    msg("好的", PrivateDirection::Them),
                ]),
                10,
            )
            .unwrap();
        assert!(matches!(
            bridge
                .bridge(
                    &snapshot(vec![
                        msg("好的", PrivateDirection::Them),
                        msg("不同会话", PrivateDirection::Them),
                    ]),
                    20,
                )
                .unwrap(),
            BridgeOutcome::Ambiguous
        ));
    }

    #[test]
    fn lost_overlap_or_unmapped_identity_never_becomes_new_work() {
        let mut bridge =
            NativeObservationBridge::new(vec![entry(ConversationKind::Private)], acceptance())
                .unwrap();
        bridge
            .bridge(&snapshot(vec![msg("A", PrivateDirection::Them)]), 10)
            .unwrap();
        assert!(matches!(
            bridge
                .bridge(
                    &snapshot(vec![msg("completely different", PrivateDirection::Them)]),
                    20
                )
                .unwrap(),
            BridgeOutcome::Ambiguous
        ));
        let mut unknown = snapshot(vec![msg("A", PrivateDirection::Them)]);
        unknown.conversation_fingerprint = "c".repeat(64);
        assert!(matches!(
            bridge.bridge(&unknown, 30).unwrap(),
            BridgeOutcome::Provisional
        ));
    }

    #[test]
    fn application_process_session_change_invalidates_old_binding() {
        let mut bridge =
            NativeObservationBridge::new(vec![entry(ConversationKind::Private)], acceptance())
                .unwrap();
        let original = snapshot(vec![
            msg("A", PrivateDirection::Me),
            msg("B", PrivateDirection::Them),
        ]);
        assert!(matches!(
            bridge.bridge(&original, 10).unwrap(),
            BridgeOutcome::Baseline(_)
        ));
        let mut restarted = original;
        restarted.application_session_fingerprint = "d".repeat(64);
        assert!(matches!(
            bridge.bridge(&restarted, 20).unwrap(),
            BridgeOutcome::Provisional
        ));
    }

    #[test]
    fn group_incoming_requires_sender_fingerprint_and_unknown_direction_is_provisional() {
        let mut bridge =
            NativeObservationBridge::new(vec![entry(ConversationKind::Group)], acceptance())
                .unwrap();
        let mut missing = msg("group", PrivateDirection::Them);
        missing.sender_fingerprint = None;
        assert!(matches!(
            bridge.bridge(&snapshot(vec![missing]), 10).unwrap(),
            BridgeOutcome::Provisional
        ));
        assert!(matches!(
            bridge
                .bridge(
                    &snapshot(vec![msg("center", PrivateDirection::Unknown)]),
                    20
                )
                .unwrap(),
            BridgeOutcome::Provisional
        ));
    }

    #[test]
    fn runtime_bridge_baselines_then_queues_only_trusted_new_incoming() {
        let configured = entry(ConversationKind::Private);
        let binding = configured.binding.clone();
        let (_dir, mut runtime) = runtime(&binding);
        let mut bridge = NativeObservationBridge::new(vec![configured], acceptance()).unwrap();

        let first = snapshot(vec![
            msg("A", PrivateDirection::Them),
            msg("B", PrivateDirection::Me),
            msg("C", PrivateDirection::Them),
        ]);
        let report = bridge
            .ingest_into_runtime(&mut runtime, &first, 10)
            .unwrap();
        assert_eq!(report.bridge_state, "BASELINE");
        assert_eq!(report.observations, 3);
        assert_eq!(report.baselined, 3);
        assert_eq!(runtime.host_counts().unwrap().0, 3);

        let second = snapshot(vec![
            msg("B", PrivateDirection::Me),
            msg("C", PrivateDirection::Them),
            msg("D", PrivateDirection::Them),
        ]);
        let report = bridge
            .ingest_into_runtime(&mut runtime, &second, 20)
            .unwrap();
        assert_eq!(report.bridge_state, "NEW");
        assert_eq!(report.observations, 1);
        assert_eq!(report.queued, 1);
        assert_eq!(runtime.host_counts().unwrap().0, 4);

        let report = bridge
            .ingest_into_runtime(&mut runtime, &second, 30)
            .unwrap();
        assert_eq!(report.bridge_state, "NO_CHANGE");
        assert_eq!(report.observations, 0);
        assert_eq!(runtime.host_counts().unwrap().0, 4);
    }

    #[test]
    fn provisional_and_ambiguous_snapshots_do_not_touch_runtime() {
        let configured = entry(ConversationKind::Private);
        let binding = configured.binding.clone();
        let (_dir, mut runtime) = runtime(&binding);
        let mut bridge = NativeObservationBridge::new(vec![configured], acceptance()).unwrap();

        let provisional = snapshot(vec![msg("center", PrivateDirection::Unknown)]);
        let report = bridge
            .ingest_into_runtime(&mut runtime, &provisional, 1)
            .unwrap();
        assert_eq!(report.bridge_state, "PROVISIONAL");
        assert_eq!(runtime.host_counts().unwrap().0, 0);

        let baseline = snapshot(vec![
            msg("A", PrivateDirection::Me),
            msg("B", PrivateDirection::Them),
        ]);
        bridge
            .ingest_into_runtime(&mut runtime, &baseline, 2)
            .unwrap();
        assert_eq!(runtime.host_counts().unwrap().0, 2);

        let unrelated = snapshot(vec![
            msg("X", PrivateDirection::Me),
            msg("Y", PrivateDirection::Them),
        ]);
        let report = bridge
            .ingest_into_runtime(&mut runtime, &unrelated, 3)
            .unwrap();
        assert_eq!(report.bridge_state, "AMBIGUOUS");
        assert_eq!(runtime.host_counts().unwrap().0, 2);
    }

    #[test]
    fn backpressure_retry_reuses_canonical_ids_and_commits_cursor_only_after_success() {
        let mut configured = entry(ConversationKind::Private);
        configured.binding.max_pending = 1;
        configured.binding.max_batch = 1;
        let binding = configured.binding.clone();
        let (_dir, mut runtime) = runtime(&binding);
        let mut bridge = NativeObservationBridge::new(vec![configured], acceptance()).unwrap();

        let first = snapshot(vec![
            msg("A", PrivateDirection::Me),
            msg("B", PrivateDirection::Them),
        ]);
        bridge
            .ingest_into_runtime(&mut runtime, &first, 10)
            .unwrap();

        let burst = snapshot(vec![
            msg("A", PrivateDirection::Me),
            msg("B", PrivateDirection::Them),
            msg("C", PrivateDirection::Them),
            msg("D", PrivateDirection::Them),
        ]);
        assert!(matches!(
            bridge.ingest_into_runtime(&mut runtime, &burst, 20),
            Err(HostError::Backpressure)
        ));
        assert_eq!(runtime.host_counts().unwrap().0, 3);

        runtime.set_host_paused(true).unwrap();
        let report = bridge
            .ingest_into_runtime(&mut runtime, &burst, 30)
            .unwrap();
        assert_eq!(report.bridge_state, "NEW");
        assert_eq!(report.observations, 2);
        assert_eq!(report.duplicates, 1);
        assert_eq!(report.ignored, 1);
        assert_eq!(runtime.host_counts().unwrap().0, 4);

        let report = bridge
            .ingest_into_runtime(&mut runtime, &burst, 40)
            .unwrap();
        assert_eq!(report.bridge_state, "NO_CHANGE");
        assert_eq!(runtime.host_counts().unwrap().0, 4);
    }
}
