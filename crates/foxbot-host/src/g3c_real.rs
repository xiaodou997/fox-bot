//! Operator-authorized G3c-1 test. Fixed synthetic reply, real current-session GUI, encrypted outbox.
use crate::{
    HostError, Result,
    credentials::{CredentialRef, CredentialStore, NativeCredentials},
    g2d_real,
    native_bridge::{
        BridgeOutcome, NativeBridgeConfig, NativeConversationBinding, NativeObservationBridge,
        PrivateMessageSnapshot,
    },
    native_send::{NativeSendChannel, NativeSendStats},
    ownership::{DeviceOwner, private_directory},
};
use foxbot_core::{
    ActionState, ConversationKind, Mode, ReplyOutcome, ReplyResponse, Runtime,
    simulation::fixture_observation,
};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    action_id: String,
    credential: CredentialRef,
    binding: NativeConversationBinding,
}

fn component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 40
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}
fn binding(session: &str) -> Result<NativeConversationBinding> {
    if !component(session) {
        return Err(HostError::Config);
    }
    g2d_real::preflight_verify(session)?;
    let (config, baseline) = g2d_real::load_baseline(session)?;
    let verified = g2d_real::read_private_json(
        &g2d_real::session_directory(session)?.join("verified-snapshot.json"),
    )?;
    verified_binding(config, &baseline, &verified)
}

fn verified_binding(
    config: NativeBridgeConfig,
    baseline: &PrivateMessageSnapshot,
    verified: &PrivateMessageSnapshot,
) -> Result<NativeConversationBinding> {
    if config.conversations.len() != 1 {
        return Err(HostError::Config);
    }
    let entry = config.conversations[0].clone();
    if entry.binding.kind != ConversationKind::Private
        || !entry.binding.enabled
        || baseline.application_session_fingerprint != entry.application_session_fingerprint
        || baseline.conversation_fingerprint != entry.conversation_fingerprint
        || verified.application_session_fingerprint != entry.application_session_fingerprint
    {
        return Err(HostError::Untrusted);
    }
    // Reuse G2's verified title/continuity binding. No new target is guessed or selected.
    let mut bridge = NativeObservationBridge::from_config(config)?;
    if !matches!(bridge.bridge(baseline, 1)?, BridgeOutcome::Baseline(_))
        || !matches!(
            bridge.bridge(verified, 2)?,
            BridgeOutcome::New(_) | BridgeOutcome::NoChange
        )
    {
        return Err(HostError::Untrusted);
    }
    NativeConversationBinding::from_current_snapshot(verified, entry.binding)
}
fn now_ms() -> Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .map_err(|_| HostError::Config)
}

pub fn inspect(binary: &Path, session: &str) -> Result<serde_json::Value> {
    let binding = binding(session)?;
    let owner = DeviceOwner::acquire()?;
    let mut channel = NativeSendChannel::start(binary, binding, &owner, PathBuf::new(), false)?;
    let observation = match channel.observe() {
        Ok(observation) => observation,
        Err(_) => {
            return Ok(
                serde_json::json!({"status":"INSPECT_BLOCKED", "native":channel.stats,
            "read_only":true, "write_operations":0, "send_operations":0, "raw_text_included":false}),
            );
        }
    };
    Ok(serde_json::json!({
        "status":"BOUND_TEST_CONVERSATION_OBSERVED", "application_session_matches":true,
        "conversation_matches":true, "frontmost":observation.frontmost,
        "draft_state":observation.draft_state, "draft_characters":observation.draft_text.as_ref().map(|t| t.chars().count()),
        "send_button_located":observation.send_button.is_some(), "message_count":observation.messages.len(),
        "read_only":true, "write_operations":0, "send_operations":0, "raw_text_included":false
    }))
}

/// Public diagnostics contain fixed phase labels and error classes, never credentials or chat text.
fn phase<T>(name: &str, operation: impl FnOnce() -> Result<T>) -> Result<T> {
    let started = std::time::Instant::now();
    eprintln!(
        "{}",
        serde_json::json!({"event":"g3c_phase", "phase":name, "state":"STARTED"})
    );
    let result = operation();
    eprintln!(
        "{}",
        serde_json::json!({"event":"g3c_phase", "phase":name,
        "state":if result.is_ok() { "COMPLETED" } else { "FAILED" },
        "elapsed_ms":started.elapsed().as_millis()})
    );
    result
}

pub fn send_once(
    binary: &Path,
    session: &str,
    run: &str,
    key_name: &str,
) -> Result<serde_json::Value> {
    if !component(session) || !component(run) {
        return Err(HostError::Config);
    }
    let owner = DeviceOwner::acquire()?;
    let root = g2d_real::session_directory(session)?.join("g3c-1");
    private_directory(&root)?;
    let directory = root.join(run);
    private_directory(&directory)?;
    let manifest_path = directory.join("manifest.json");
    let ledger = directory.join("runtime");
    let receipt = directory.join("receipt-context.json");
    let credential = CredentialRef {
        id: key_name.into(),
    };
    credential.validate()?;
    let secret = phase("CREDENTIAL_READ", || {
        NativeCredentials.load(&credential, "ledger")
    })?;
    let manifest: Manifest;
    let mut runtime;
    let mut initial_channel = None;
    if manifest_path.exists() {
        manifest = g2d_real::read_private_json(&manifest_path)?;
        if manifest.schema_version != 1 || manifest.credential.id != key_name {
            return Err(HostError::Config);
        }
        runtime = phase("OPEN_LEDGER", || {
            Ok(Runtime::open_encrypted(&ledger, secret.ledger_key()?)?)
        })?;
    } else {
        // An interrupted preparation is not silently rebuilt under a fresh action id.
        if ledger.exists() {
            return Err(HostError::Config);
        }
        let mut bound = binding(session)?;
        let mut channel = phase("START_NATIVE_WORKER", || {
            NativeSendChannel::start(binary, bound.clone(), &owner, receipt.clone(), true)
        })?;
        let current = channel.observe()?;
        if current.draft_state != "EMPTY_HEURISTIC" || current.draft_text.as_deref() != Some("") {
            return Ok(
                serde_json::json!({"status":"INITIAL_DRAFT_NOT_EMPTY", "action_created":false,
                "native":channel.stats, "raw_text_included":false, "external_model_requests":0}),
            );
        }
        bound.binding.mode = Mode::Assisted;
        bound.binding.max_auto_sends = 1;
        bound.binding.quiet_ms = 0;
        bound.binding.max_wait_ms = 0;
        let now = now_ms()?;
        runtime = phase("OPEN_LEDGER", || {
            Ok(Runtime::open_encrypted(&ledger, secret.ledger_key()?)?)
        })?;
        runtime.bind(&bound.binding)?;
        runtime.set_host_paused(false)?;
        // A labeled operator test trigger, never presented as an actual incoming chat message.
        runtime.ingest(&fixture_observation(
            &bound.binding.key,
            &format!("g3c1-{run}"),
            "Operator-authorized single-send test",
            now,
        ))?;
        let request = runtime
            .begin_reply(&bound.binding.key, now, Some("G3c-1 fixed test reply"))?
            .ok_or(HostError::Config)?;
        runtime.accept_reply(
            &request.request_id,
            &ReplyResponse::for_request(
                &request,
                ReplyOutcome::Reply {
                    text: format!("FoxBot G3c1 {run}"),
                },
            ),
            now,
        )?;
        let action_id = runtime.prepare_send(&request.request_id, now, true)?;
        manifest = Manifest {
            schema_version: 1,
            action_id,
            credential,
            binding: bound,
        };
        g2d_real::write_private_json(&manifest_path, &manifest)?;
        initial_channel = Some(channel);
    }
    let (_, previous) = runtime.action(&manifest.action_id)?;
    let mut stats = NativeSendStats::default();
    let final_state = match previous {
        ActionState::Prepared => {
            let mut channel = if let Some(channel) = initial_channel {
                channel
            } else {
                NativeSendChannel::start(
                    binary,
                    manifest.binding.clone(),
                    &owner,
                    receipt.clone(),
                    true,
                )?
            };
            let state = phase("DISPATCH", || {
                Ok(runtime.dispatch(&manifest.action_id, now_ms()?, &mut channel)?)
            })?;
            stats = channel.stats;
            state
        }
        ActionState::Unknown | ActionState::Submitted if receipt.exists() => {
            // This worker has NO native write capability. Repeating the command cannot resend.
            let mut channel = phase("START_READ_ONLY_WORKER", || {
                NativeSendChannel::start(binary, manifest.binding.clone(), &owner, receipt, false)
            })?;
            let state = phase("RECONCILE", || {
                Ok(runtime.reconcile(&manifest.action_id, &mut channel)?)
            })?;
            stats = channel.stats;
            state
        }
        state => state,
    };
    let report = serde_json::json!({
        "schema_version":"foxbot.g3c-single-send.v1", "run":run, "action_state":final_state,
        "previous_state":previous, "native":stats, "encrypted_outbox":runtime.is_encrypted(),
        "trigger_source":"SYNTHETIC_OPERATOR_TEST", "reply_source":"FIXED_TEST_TEXT",
        "external_model_requests":0, "raw_text_included":false, "image_saved":false,
        "delivery_confirmed":false, "read_confirmed":false,
        "transition_history":runtime.transition_history(&manifest.action_id)?
    });
    g2d_real::write_private_json(&directory.join("last-report.json"), &report)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_bridge::{GroundTruthAcceptance, PrivateBridgeMessage, PrivateDirection};
    use foxbot_core::{Binding, simulation::fixture_key};

    fn fixtures() -> (
        NativeBridgeConfig,
        PrivateMessageSnapshot,
        PrivateMessageSnapshot,
    ) {
        let mut binding = Binding::paused(fixture_key());
        binding.enabled = true;
        let baseline = PrivateMessageSnapshot {
            schema_version: "foxbot.private-message-snapshot.v1".into(),
            strategy: "WECHAT_HEURISTIC_V0".into(),
            application_session_fingerprint: "c".repeat(64),
            conversation_fingerprint: "a".repeat(64),
            partial_reasons: vec!["HEURISTIC_REGION".into()],
            messages: ["anchor-a", "anchor-b"]
                .into_iter()
                .map(|text| PrivateBridgeMessage {
                    text: text.into(),
                    direction: PrivateDirection::Them,
                    sender_fingerprint: None,
                    complete: true,
                })
                .collect(),
        };
        let config = NativeBridgeConfig {
            schema_version: 1,
            conversations: vec![
                NativeConversationBinding::from_current_snapshot(&baseline, binding).unwrap(),
            ],
            acceptance: GroundTruthAcceptance {
                schema_version: "foxbot.g2c-ground-truth-result.v1".into(),
                strategy: baseline.strategy.clone(),
                revision: "synthetic-test".into(),
                accepted: true,
                cases: 6,
                labeled_messages: 24,
                covered_tags: [
                    "private",
                    "group",
                    "duplicate_text",
                    "numeric",
                    "multiline",
                    "reference",
                ]
                .into_iter()
                .map(String::from)
                .collect(),
                direction_errors: 0,
                sender_errors: 0,
                message_count_errors: 0,
                text_errors: 0,
                text_edit_distance: 0,
                text_expected_characters: 0,
                text_error_rate_bp: 0,
            },
        };
        let mut verified = baseline.clone();
        verified.conversation_fingerprint = "b".repeat(64);
        verified.messages.push(PrivateBridgeMessage {
            text: "new message".into(),
            direction: PrivateDirection::Them,
            sender_fingerprint: None,
            complete: true,
        });
        (config, baseline, verified)
    }

    #[test]
    fn previously_verified_title_drift_reuses_g2_continuity_without_changing_target() {
        let (config, baseline, verified) = fixtures();
        let old_key = config.conversations[0].binding.key.clone();
        let resolved = verified_binding(config, &baseline, &verified).unwrap();
        assert_eq!(resolved.binding.key, old_key);
        assert_eq!(
            resolved.conversation_fingerprint,
            verified.conversation_fingerprint
        );
    }

    #[test]
    fn restart_or_unrelated_history_cannot_rebind_the_send_target() {
        let (config, baseline, mut verified) = fixtures();
        verified.application_session_fingerprint = "d".repeat(64);
        assert!(verified_binding(config.clone(), &baseline, &verified).is_err());
        verified.application_session_fingerprint = baseline.application_session_fingerprint.clone();
        verified.messages.drain(..2);
        assert!(verified_binding(config, &baseline, &verified).is_err());
    }

    #[test]
    fn test_identifiers_cannot_escape_test_state_or_inject_text_controls() {
        for value in ["", ".", "..", "../a", "a/b", "a\\b", "foo\n", "a b"] {
            assert!(!component(value));
        }
        assert!(component("g2d-test"));
        assert!(component("742961"));
        assert!(!component(&"x".repeat(41)));
    }
}
