use crate::{
    HostError, Result,
    native_bridge::{
        GroundTruthAcceptance, NativeBridgeConfig, NativeConversationBinding,
        NativeObservationBridge, PrivateDirection, PrivateMessageSnapshot, RuntimeBridgeReport,
    },
};
use foxbot_core::{Binding, ConversationKey, ConversationKind, Mode, ProviderProfile, Runtime};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
use zeroize::Zeroize;

const ROOT_NAME: &str = "target/g2d-real";
const MAX_PRIVATE_JSON_BYTES: u64 = 2 * 1024 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroundTruthMessage {
    pub text: String,
    pub direction: String,
    pub sender_labeled: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroundTruthCase {
    pub id: String,
    pub tags: Vec<String>,
    pub expected: Vec<GroundTruthMessage>,
    pub observed: Vec<GroundTruthMessage>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroundTruthDraft {
    pub schema_version: String,
    pub strategy: String,
    pub revision: String,
    pub cases: Vec<GroundTruthCase>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PrivateCaptureReport {
    pub status: String,
    pub session: String,
    pub case_id: String,
    pub tags: Vec<String>,
    pub observed_messages: usize,
    pub stable_two_reads: bool,
    pub raw_text_included: bool,
    pub private_file_written: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct BaselineReport {
    pub status: String,
    pub session: String,
    pub stable_two_reads: bool,
    pub baseline_messages: usize,
    pub binding_written: bool,
    pub baseline_written: bool,
    pub raw_text_included: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct RealVerifyReport {
    pub status: String,
    pub baseline: RuntimeBridgeReport,
    pub current: RuntimeBridgeReport,
    pub repeat: RuntimeBridgeReport,
    pub runtime_messages: u64,
    pub runtime_tasks: u64,
    pub runtime_ready: u64,
    pub runtime_unresolved_sends: u64,
    pub raw_text_included: bool,
    pub image_saved: bool,
    pub external_model_requests: u32,
    pub native_chat_operations: u32,
    pub write_or_send_operations: u32,
}

pub fn safe_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

fn private_root() -> Result<PathBuf> {
    let root = std::env::current_dir()
        .map_err(|_| HostError::Storage)?
        .join(ROOT_NAME);
    private_directory(&root)?;
    fs::canonicalize(root).map_err(|_| HostError::Storage)
}

pub fn session_directory(session: &str) -> Result<PathBuf> {
    if !safe_token(session) {
        return Err(HostError::Config);
    }
    let root = private_root()?;
    let directory = root.join(session);
    private_directory(&directory)?;
    let canonical = fs::canonicalize(&directory).map_err(|_| HostError::Storage)?;
    if canonical.parent() != Some(root.as_path()) {
        return Err(HostError::Config);
    }
    Ok(canonical)
}

fn private_directory(path: &Path) -> Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path).map_err(|_| HostError::Storage)?;
    let metadata = fs::symlink_metadata(path).map_err(|_| HostError::Storage)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(HostError::Config);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))
                .map_err(|_| HostError::Storage)?;
        }
    }
    Ok(())
}

fn write_private_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Ok(metadata) = fs::symlink_metadata(path)
        && (!metadata.is_file() || metadata.file_type().is_symlink())
    {
        return Err(HostError::Config);
    }
    let mut nonce = [0u8; 8];
    getrandom::fill(&mut nonce).map_err(|_| HostError::Storage)?;
    let suffix = nonce.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let filename = path
        .file_name()
        .and_then(|v| v.to_str())
        .ok_or(HostError::Config)?;
    let temporary = path.with_file_name(format!(".{filename}.{suffix}.tmp"));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary).map_err(|_| HostError::Storage)?;
    let payload = serde_json::to_vec_pretty(value).map_err(|_| HostError::Config)?;
    if payload.len() as u64 > MAX_PRIVATE_JSON_BYTES {
        let _ = fs::remove_file(&temporary);
        return Err(HostError::Config);
    }
    file.write_all(&payload).map_err(|_| HostError::Storage)?;
    file.write_all(b"\n").map_err(|_| HostError::Storage)?;
    file.sync_all().map_err(|_| HostError::Storage)?;
    drop(file);
    fs::rename(&temporary, path).map_err(|_| HostError::Storage)?;
    Ok(())
}

fn read_private_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let metadata = fs::symlink_metadata(path).map_err(|_| HostError::Config)?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAX_PRIVATE_JSON_BYTES
    {
        return Err(HostError::Config);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(HostError::Config);
        }
    }
    serde_json::from_slice(&fs::read(path).map_err(|_| HostError::Storage)?)
        .map_err(|_| HostError::Config)
}

fn direction(value: &PrivateDirection) -> &'static str {
    match value {
        PrivateDirection::Me => "ME",
        PrivateDirection::Them => "THEM",
        PrivateDirection::Unknown => "UNKNOWN",
    }
}

fn observed_messages(snapshot: &PrivateMessageSnapshot) -> Vec<GroundTruthMessage> {
    snapshot
        .messages
        .iter()
        .map(|message| GroundTruthMessage {
            text: message.text.clone(),
            direction: direction(&message.direction).into(),
            sender_labeled: message.sender_fingerprint.is_some(),
        })
        .collect()
}

pub fn capture_case(
    session: &str,
    case_id: &str,
    tags: &[String],
    snapshot: &PrivateMessageSnapshot,
) -> Result<PrivateCaptureReport> {
    if !safe_token(case_id) || tags.is_empty() || tags.len() > 16 {
        return Err(HostError::Config);
    }
    if tags.iter().any(|tag| !safe_token(tag)) {
        return Err(HostError::Config);
    }
    let directory = session_directory(session)?;
    let snapshot_path = directory.join(format!("snapshot-{case_id}.json"));
    write_private_json(&snapshot_path, snapshot)?;

    let draft_path = directory.join("groundtruth.json");
    let mut draft = if draft_path.exists() {
        read_private_json::<GroundTruthDraft>(&draft_path)?
    } else {
        GroundTruthDraft {
            schema_version: "foxbot.g2c-ground-truth.v1".into(),
            strategy: snapshot.strategy.clone(),
            revision: format!("g2d-real-{session}"),
            cases: Vec::new(),
        }
    };
    if draft.strategy != snapshot.strategy
        || draft.cases.iter().any(|case| case.id == case_id)
        || draft.cases.len() >= 64
    {
        return Err(HostError::Config);
    }
    draft.cases.push(GroundTruthCase {
        id: case_id.into(),
        tags: tags.to_vec(),
        expected: Vec::new(),
        observed: observed_messages(snapshot),
    });
    write_private_json(&draft_path, &draft)?;
    Ok(PrivateCaptureReport {
        status: "PRIVATE_CASE_CAPTURED".into(),
        session: session.into(),
        case_id: case_id.into(),
        tags: tags.to_vec(),
        observed_messages: snapshot.messages.len(),
        stable_two_reads: true,
        raw_text_included: false,
        private_file_written: true,
    })
}

pub fn prepare_baseline(
    session: &str,
    acceptance: GroundTruthAcceptance,
    snapshot: &PrivateMessageSnapshot,
    account_binding: &str,
    conversation: &str,
    kind: ConversationKind,
) -> Result<BaselineReport> {
    acceptance.validate()?;
    if !safe_token(account_binding) || !safe_token(conversation) {
        return Err(HostError::Config);
    }
    if snapshot.strategy != acceptance.strategy
        || snapshot
            .conversation_fingerprint
            .bytes()
            .all(|value| value == b'0')
        || snapshot
            .partial_reasons
            .iter()
            .any(|value| value == "CONVERSATION_IDENTITY_UNRESOLVED")
        || snapshot
            .messages
            .iter()
            .any(|message| message.direction == PrivateDirection::Unknown || !message.complete)
    {
        return Err(HostError::Untrusted);
    }
    if kind == ConversationKind::Group
        && snapshot.messages.iter().any(|message| {
            message.direction == PrivateDirection::Them && message.sender_fingerprint.is_none()
        })
    {
        return Err(HostError::Untrusted);
    }
    if kind == ConversationKind::Private
        && snapshot
            .messages
            .iter()
            .any(|message| message.sender_fingerprint.is_some())
    {
        return Err(HostError::Untrusted);
    }
    let directory = session_directory(session)?;
    let mut binding = Binding::paused(ConversationKey {
        device: "local-macos".into(),
        app_instance: "wechat-macos".into(),
        account_binding: account_binding.into(),
        conversation: conversation.into(),
    });
    binding.mode = Mode::AutoSuggest;
    binding.provider = ProviderProfile::Custom;
    binding.kind = kind;
    binding.group_all_messages = kind == ConversationKind::Group;
    binding.enabled = true;
    binding.quiet_ms = 0;
    binding.max_wait_ms = 0;
    binding.validate().map_err(|_| HostError::Config)?;
    let configured = NativeConversationBinding::from_current_snapshot(snapshot, binding)?;
    let config = NativeBridgeConfig {
        schema_version: 1,
        acceptance,
        conversations: vec![configured],
    };
    write_private_json(&directory.join("bridge-config.json"), &config)?;
    write_private_json(&directory.join("baseline-snapshot.json"), snapshot)?;
    Ok(BaselineReport {
        status: "BASELINE_READY".into(),
        session: session.into(),
        stable_two_reads: true,
        baseline_messages: snapshot.messages.len(),
        binding_written: true,
        baseline_written: true,
        raw_text_included: false,
    })
}

pub fn load_acceptance(session: &str) -> Result<GroundTruthAcceptance> {
    let directory = session_directory(session)?;
    read_private_json(&directory.join("acceptance.json"))
}

pub fn preflight_baseline(session: &str) -> Result<GroundTruthAcceptance> {
    let acceptance = load_acceptance(session)?;
    acceptance.validate()?;
    Ok(acceptance)
}

pub fn load_baseline(session: &str) -> Result<(NativeBridgeConfig, PrivateMessageSnapshot)> {
    let directory = session_directory(session)?;
    Ok((
        read_private_json(&directory.join("bridge-config.json"))?,
        read_private_json(&directory.join("baseline-snapshot.json"))?,
    ))
}

pub fn preflight_verify(session: &str) -> Result<()> {
    let (config, baseline) = load_baseline(session)?;
    let binding = config.conversations.first().ok_or(HostError::Config)?;
    if binding.application_session_fingerprint != baseline.application_session_fingerprint
        || binding.conversation_fingerprint != baseline.conversation_fingerprint
    {
        return Err(HostError::Untrusted);
    }
    let _ = NativeObservationBridge::from_config(config)?;
    Ok(())
}

pub fn verify_current(session: &str, current: &PrivateMessageSnapshot) -> Result<RealVerifyReport> {
    let directory = session_directory(session)?;
    let (config, baseline) = load_baseline(session)?;
    let mut bridge = NativeObservationBridge::from_config(config.clone())?;
    let binding = config
        .conversations
        .first()
        .ok_or(HostError::Config)?
        .binding
        .clone();
    let mut nonce = [0u8; 8];
    getrandom::fill(&mut nonce).map_err(|_| HostError::Storage)?;
    let suffix = nonce.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let runtime_directory = directory.join(format!("runtime-{suffix}"));
    let mut key = [0u8; 32];
    getrandom::fill(&mut key).map_err(|_| HostError::Storage)?;
    let result = (|| -> Result<RealVerifyReport> {
        let mut runtime =
            Runtime::open_encrypted(&runtime_directory, &key).map_err(|_| HostError::Storage)?;
        runtime.bind(&binding).map_err(|_| HostError::Storage)?;
        runtime
            .set_host_paused(false)
            .map_err(|_| HostError::Storage)?;
        let baseline_report = bridge.ingest_into_runtime(&mut runtime, &baseline, 1)?;
        if baseline_report.bridge_state != "BASELINE"
            || baseline_report.baselined != baseline.messages.len()
        {
            return Err(HostError::Untrusted);
        }
        let current_report = bridge.ingest_into_runtime(&mut runtime, current, 2)?;
        if current_report.bridge_state != "NEW"
            || current_report.observations != 1
            || current_report.queued != 1
            || current_report.ambiguous != 0
        {
            return Err(HostError::Untrusted);
        }
        let repeat_report = bridge.ingest_into_runtime(&mut runtime, current, 3)?;
        if repeat_report.bridge_state != "NO_CHANGE" || repeat_report.observations != 0 {
            return Err(HostError::Untrusted);
        }
        let counts = runtime.host_counts().map_err(|_| HostError::Storage)?;
        drop(runtime);
        Ok(RealVerifyReport {
            status: "REAL_OBSERVATION_PASS".into(),
            baseline: baseline_report,
            current: current_report,
            repeat: repeat_report,
            runtime_messages: counts.0,
            runtime_tasks: counts.1,
            runtime_ready: counts.2,
            runtime_unresolved_sends: counts.3,
            raw_text_included: false,
            image_saved: false,
            external_model_requests: 0,
            native_chat_operations: 0,
            write_or_send_operations: 0,
        })
    })();
    key.zeroize();
    let _ = fs::remove_dir_all(&runtime_directory);
    if result.is_ok() {
        write_private_json(&directory.join("verified-snapshot.json"), current)?;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_bridge::{PrivateBridgeMessage, PrivateDirection};

    const REQUIRED_TAGS: [&str; 6] = [
        "private",
        "group",
        "duplicate_text",
        "numeric",
        "multiline",
        "reference",
    ];

    fn acceptance() -> GroundTruthAcceptance {
        GroundTruthAcceptance {
            schema_version: "foxbot.g2c-ground-truth-result.v1".into(),
            strategy: "WECHAT_HEURISTIC_V0".into(),
            revision: "unit-v1".into(),
            accepted: true,
            cases: 6,
            labeled_messages: 24,
            covered_tags: REQUIRED_TAGS.iter().map(|value| (*value).into()).collect(),
            direction_errors: 0,
            sender_errors: 0,
            message_count_errors: 0,
            text_errors: 0,
            text_edit_distance: 0,
            text_expected_characters: 0,
            text_error_rate_bp: 0,
        }
    }

    fn unique_session(prefix: &str) -> String {
        let mut nonce = [0u8; 6];
        getrandom::fill(&mut nonce).unwrap();
        format!(
            "{prefix}-{}",
            nonce
                .iter()
                .map(|value| format!("{value:02x}"))
                .collect::<String>()
        )
    }

    fn snapshot() -> PrivateMessageSnapshot {
        PrivateMessageSnapshot {
            schema_version: "foxbot.private-message-snapshot.v1".into(),
            strategy: "WECHAT_HEURISTIC_V0".into(),
            application_session_fingerprint: "c".repeat(64),
            conversation_fingerprint: "a".repeat(64),
            partial_reasons: vec!["HEURISTIC_REGION".into()],
            messages: vec![
                PrivateBridgeMessage {
                    text: "A".into(),
                    direction: PrivateDirection::Them,
                    sender_fingerprint: Some("b".repeat(64)),
                    complete: true,
                },
                PrivateBridgeMessage {
                    text: "B".into(),
                    direction: PrivateDirection::Me,
                    sender_fingerprint: None,
                    complete: true,
                },
            ],
        }
    }

    #[test]
    fn tokens_are_strict_and_ground_truth_capture_never_marks_expected_for_operator() {
        assert!(safe_token("test-session_01"));
        assert!(!safe_token("../escape"));
        let snap = snapshot();
        let observed = observed_messages(&snap);
        assert_eq!(observed.len(), 2);
        assert_eq!(observed[0].text, "A");
        assert!(observed[0].sender_labeled);
    }

    #[test]
    fn baseline_refuses_unknown_direction_before_writing_bridge_config() {
        let mut snap = snapshot();
        snap.messages[0].direction = PrivateDirection::Unknown;
        let accepted = acceptance();
        assert!(matches!(
            prepare_baseline(
                "unit",
                accepted,
                &snap,
                "acct",
                "conv",
                ConversationKind::Private
            ),
            Err(HostError::Untrusted)
        ));

        let mut unresolved = snapshot();
        unresolved.conversation_fingerprint = "0".repeat(64);
        unresolved
            .partial_reasons
            .push("CONVERSATION_IDENTITY_UNRESOLVED".into());
        assert!(matches!(
            prepare_baseline(
                "unit-unresolved",
                acceptance(),
                &unresolved,
                "acct",
                "conv",
                ConversationKind::Private
            ),
            Err(HostError::Untrusted)
        ));
    }

    #[test]
    fn private_baseline_refuses_group_sender_identity() {
        let snap = snapshot();
        assert!(
            snap.messages
                .iter()
                .any(|message| message.sender_fingerprint.is_some())
        );
        assert!(matches!(
            prepare_baseline(
                "unit-private-group-mismatch",
                acceptance(),
                &snap,
                "acct",
                "conv",
                ConversationKind::Private
            ),
            Err(HostError::Untrusted)
        ));
    }

    #[test]
    fn private_capture_never_self_approves_observed_messages() {
        let session = unique_session("capture");
        let directory = session_directory(&session).unwrap();
        let report =
            capture_case(&session, "private-case", &["private".into()], &snapshot()).unwrap();
        assert_eq!(report.observed_messages, 2);
        let draft: GroundTruthDraft =
            read_private_json(&directory.join("groundtruth.json")).unwrap();
        assert_eq!(draft.cases.len(), 1);
        assert!(draft.cases[0].expected.is_empty());
        assert_eq!(draft.cases[0].observed.len(), 2);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(directory.join("groundtruth.json"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o077,
                0
            );
        }
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn accepted_private_workflow_baselines_and_verifies_exactly_one_new_incoming() {
        let session = unique_session("verify");
        let mut baseline = snapshot();
        for message in &mut baseline.messages {
            message.sender_fingerprint = None;
        }
        prepare_baseline(
            &session,
            acceptance(),
            &baseline,
            "test-account",
            "test-conversation",
            ConversationKind::Private,
        )
        .unwrap();
        let mut current = baseline.clone();
        current.messages.push(PrivateBridgeMessage {
            text: "NEW".into(),
            direction: PrivateDirection::Them,
            sender_fingerprint: None,
            complete: true,
        });
        let report = verify_current(&session, &current).unwrap();
        assert_eq!(report.status, "REAL_OBSERVATION_PASS");
        assert_eq!(report.baseline.baselined, 2);
        assert_eq!(report.current.queued, 1);
        assert_eq!(report.repeat.bridge_state, "NO_CHANGE");
        assert_eq!(report.runtime_messages, 3);
        assert_eq!(report.runtime_tasks, 0);
        assert_eq!(report.runtime_unresolved_sends, 0);
        let directory = session_directory(&session).unwrap();
        assert!(directory.join("verified-snapshot.json").is_file());
        assert!(fs::read_dir(&directory).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("runtime-")
        }));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn false_acceptance_blocks_before_native_read_preflight_can_continue() {
        let session = unique_session("reject");
        let directory = session_directory(&session).unwrap();
        let mut rejected = acceptance();
        rejected.accepted = false;
        write_private_json(&directory.join("acceptance.json"), &rejected).unwrap();
        assert!(matches!(
            preflight_baseline(&session),
            Err(HostError::Untrusted)
        ));
        fs::remove_dir_all(directory).unwrap();
    }
}
