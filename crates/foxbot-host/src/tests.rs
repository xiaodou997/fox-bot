use crate::*;
use crate::{
    credentials::{CredentialRef, CredentialStore, Secret},
    ownership::DeviceOwner,
};
use foxbot_core::{simulation::*, *};
use foxbot_http::*;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
#[path = "../../foxbot-http/tests/support/mod.rs"]
pub(crate) mod support;
use support::{Server, private_dir};

fn configuration(server: &Server, count: usize) -> HostConfig {
    let bindings = (0..count)
        .map(|i| {
            let mut key = fixture_key();
            key.conversation = format!("synthetic-{i}");
            let mut b = Binding::paused(key);
            b.enabled = true;
            b.quiet_ms = 0;
            b.max_wait_ms = 0;
            b
        })
        .collect();
    HostConfig {
        schema_version: 1,
        http: server.config(Protocol::BusinessV1, ContextMode::ServiceManaged),
        token: None,
        storage: StorageConfig::SyntheticPlaintext,
        bindings,
        tick_ms: 10,
        max_jobs: 2,
        shutdown_ms: 500,
    }
}
#[derive(Clone, Default)]
struct Channels {
    channels: Arc<Mutex<std::collections::HashMap<String, MockChannel>>>,
}
impl Channels {
    fn count(&self) -> usize {
        self.channels
            .lock()
            .unwrap()
            .values()
            .map(|c| c.send_calls)
            .sum()
    }
    fn with<T>(
        &self,
        key: &ConversationKey,
        op: impl FnOnce(&mut MockChannel) -> foxbot_core::Result<T>,
    ) -> foxbot_core::Result<T> {
        let encoded = serde_json::to_string(key)?;
        let mut map = self.channels.lock().unwrap();
        op(map.entry(encoded).or_insert_with(|| MockChannel::new(key)))
    }
}
impl MessageChannel for Channels {
    fn inspect(&mut self, target: &ConversationKey) -> foxbot_core::Result<LiveTarget> {
        self.with(target, |c| c.inspect(target))
    }
    fn fill(&mut self, a: &OutboundAction, e: &LiveTarget) -> foxbot_core::Result<()> {
        self.with(&a.target, |c| c.fill(a, e))
    }
    fn send(&mut self, a: &OutboundAction, e: &LiveTarget) -> foxbot_core::Result<SendEvidence> {
        self.with(&a.target, |c| c.send(a, e))
    }
    fn reconcile(&mut self, a: &OutboundAction) -> foxbot_core::Result<SendEvidence> {
        self.with(&a.target, |c| c.reconcile(a))
    }
}
fn setup(config: HostConfig) -> (tempfile::TempDir, Host<Channels>, HostHandle, Channels) {
    let dir = private_dir();
    let owner = DeviceOwner::acquire_at(&dir.path().join("execution")).unwrap();
    let runtime = Runtime::open_simulation(dir.path().join("ledger")).unwrap();
    let service = HttpReplyService::new(config.http.clone(), None).unwrap();
    let channels = Channels::default();
    let (host, handle) = Host::new(runtime, service, config, channels.clone(), owner).unwrap();
    (dir, host, handle, channels)
}
fn observation(config: &HostConfig, index: usize, id: &str) -> Observation {
    fixture_observation(
        &config.bindings[index].key,
        id,
        "合成 G1c 消息",
        RunClock::default().now_ms(),
    )
}
async fn until(handle: &HostHandle, p: impl Fn(&HostSnapshot) -> bool) -> HostSnapshot {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let s = handle.status().await.unwrap();
            if p(&s) {
                return s;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("host fixture deadline")
}
async fn stop(
    handle: &HostHandle,
    task: tokio::task::JoinHandle<crate::Result<HostSnapshot>>,
) -> HostSnapshot {
    handle.stop();
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn starts_paused_and_does_not_backfill_then_continues_multiple_conversations() {
    let server = Server::start().await;
    let cfg = configuration(&server, 3);
    let (_dir, host, h, c) = setup(cfg.clone());
    let task = tokio::spawn(host.run());
    h.observe(observation(&cfg, 0, "history")).await.unwrap();
    assert!(h.status().await.unwrap().paused);
    assert!(server.state.lock().unwrap().seen.is_empty());
    h.resume().await.unwrap();
    for i in 0..3 {
        h.observe(observation(&cfg, i, "new")).await.unwrap();
    }
    let s = until(&h, |s| {
        s.observed_outgoing_this_run == 3 && s.feedback_acked_this_run >= 3
    })
    .await;
    assert_eq!(s.provider_jobs_started, 3);
    assert_eq!(c.count(), 3);
    let s = stop(&h, task).await;
    assert!(s.paused);
    assert_eq!(s.active_jobs, 0);
    assert_eq!(s.active_feedback, 0);
}

#[tokio::test]
async fn pause_is_responsive_during_http_and_late_reply_cannot_send() {
    let server = Server::start().await;
    server.state.lock().unwrap().generate_delay_ms = 800;
    let cfg = configuration(&server, 1);
    let (dir, host, h, c) = setup(cfg.clone());
    let task = tokio::spawn(host.run());
    h.resume().await.unwrap();
    h.observe(observation(&cfg, 0, "old")).await.unwrap();
    server.wait_requests(1).await;
    tokio::time::timeout(Duration::from_millis(500), h.pause())
        .await
        .unwrap()
        .unwrap();
    h.observe(observation(&cfg, 0, "during-pause"))
        .await
        .unwrap();
    until(&h, |s| s.active_jobs == 0 && s.feedback_acked_this_run >= 1).await;
    assert_eq!(c.count(), 0);
    server.state.lock().unwrap().generate_delay_ms = 0;
    h.resume().await.unwrap();
    h.observe(observation(&cfg, 0, "after-resume"))
        .await
        .unwrap();
    until(&h, |s| s.observed_outgoing_this_run == 1).await;
    stop(&h, task).await;
    let rt = Runtime::open_simulation(dir.path().join("ledger")).unwrap();
    assert!(rt.host_is_paused().unwrap());
}

#[tokio::test]
async fn stale_queued_observations_cannot_become_new_work_after_resume() {
    let server = Server::start().await;
    let cfg = configuration(&server, 1);
    let (_dir, host, h, c) = setup(cfg.clone());
    let mut inputs = Vec::new();
    for i in 0..4 {
        let handle = h.clone();
        let o = observation(&cfg, 0, &format!("queued-{i}"));
        inputs.push(tokio::spawn(async move { handle.observe(o).await }));
    }
    tokio::task::yield_now().await;
    let control = h.clone();
    let resume = tokio::spawn(async move { control.resume().await });
    tokio::task::yield_now().await;
    let task = tokio::spawn(host.run());
    resume.await.unwrap().unwrap();
    for input in inputs {
        input.await.unwrap().unwrap();
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(c.count(), 0);
    assert_eq!(h.status().await.unwrap().provider_jobs_started, 0);
    h.observe(observation(&cfg, 0, "actually-new"))
        .await
        .unwrap();
    until(&h, |s| s.observed_outgoing_this_run == 1).await;
    stop(&h, task).await;
}

#[tokio::test]
async fn new_message_cancels_inflight_and_only_latest_batch_sends() {
    let server = Server::start().await;
    server.state.lock().unwrap().generate_delay_ms = 300;
    let cfg = configuration(&server, 1);
    let (_dir, host, h, c) = setup(cfg.clone());
    let task = tokio::spawn(host.run());
    h.resume().await.unwrap();
    h.observe(observation(&cfg, 0, "one")).await.unwrap();
    server.wait_requests(1).await;
    h.observe(observation(&cfg, 0, "two")).await.unwrap();
    let s = until(&h, |s| s.observed_outgoing_this_run == 1).await;
    assert_eq!(s.provider_jobs_started, 2);
    assert_eq!(c.count(), 1);
    stop(&h, task).await;
}

#[tokio::test]
async fn stop_cancels_and_drains_http_without_detached_sends() {
    let server = Server::start().await;
    server.state.lock().unwrap().generate_delay_ms = 800;
    let cfg = configuration(&server, 2);
    let (dir, host, h, c) = setup(cfg.clone());
    let task = tokio::spawn(host.run());
    h.resume().await.unwrap();
    h.observe(observation(&cfg, 0, "one")).await.unwrap();
    server.wait_requests(1).await;
    let s = stop(&h, task).await;
    assert_eq!(s.active_jobs, 0);
    assert_eq!(s.active_feedback, 0);
    assert_eq!(c.count(), 0);
    let rt = Runtime::open_simulation(dir.path().join("ledger")).unwrap();
    assert!(rt.host_is_paused().unwrap());
    assert!(rt.summary().unwrap().actions.is_empty());
    assert!(h.resume().await.is_err());
}

#[tokio::test]
async fn feedback_retries_automatically_without_a_second_chat_send() {
    let server = Server::start().await;
    server.state.lock().unwrap().receipt_status = 503;
    let cfg = configuration(&server, 1);
    let (_dir, host, h, c) = setup(cfg.clone());
    let task = tokio::spawn(host.run());
    h.resume().await.unwrap();
    h.observe(observation(&cfg, 0, "one")).await.unwrap();
    until(&h, |s| s.observed_outgoing_this_run == 1).await;
    server.wait_requests(2).await;
    server.state.lock().unwrap().receipt_status = 200;
    until(&h, |s| s.feedback_acked_this_run == 1).await;
    assert_eq!(c.count(), 1);
    assert_eq!(server.state.lock().unwrap().generations.len(), 1);
    stop(&h, task).await;
}

#[tokio::test]
async fn global_job_limit_and_round_robin_do_not_starve_later_conversations() {
    let server = Server::start().await;
    server.state.lock().unwrap().generate_delay_ms = 60;
    let mut cfg = configuration(&server, 5);
    cfg.max_jobs = 1;
    let (_dir, host, h, c) = setup(cfg.clone());
    let task = tokio::spawn(host.run());
    h.resume().await.unwrap();
    for i in 0..5 {
        h.observe(observation(&cfg, i, "one")).await.unwrap();
    }
    for _ in 0..10 {
        assert!(h.status().await.unwrap().active_jobs <= 1);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    until(&h, |s| s.observed_outgoing_this_run == 5).await;
    assert_eq!(c.count(), 5);
    stop(&h, task).await;
}

#[tokio::test]
async fn auto_suggest_never_dispatches_and_unconfigured_input_is_rejected() {
    let server = Server::start().await;
    let mut cfg = configuration(&server, 1);
    cfg.bindings[0].mode = Mode::AutoSuggest;
    let (_dir, host, h, c) = setup(cfg.clone());
    let task = tokio::spawn(host.run());
    h.resume().await.unwrap();
    let mut bad = observation(&cfg, 0, "bad");
    bad.key.account_binding = "not-authorized".into();
    assert!(h.observe(bad).await.is_err());
    h.observe(observation(&cfg, 0, "one")).await.unwrap();
    until(&h, |s| s.ready == 1).await;
    assert_eq!(c.count(), 0);
    stop(&h, task).await;
}

#[tokio::test]
async fn restart_requires_resume_and_does_not_resurrect_manual_handoff() {
    let server = Server::start().await;
    let cfg = configuration(&server, 1);
    let (dir, host, h, _) = setup(cfg.clone());
    let task = tokio::spawn(host.run());
    h.resume().await.unwrap();
    stop(&h, task).await;
    let mut rt = Runtime::open_simulation(dir.path().join("ledger")).unwrap();
    rt.pause(&cfg.bindings[0].key).unwrap();
    drop(rt);
    let rt = Runtime::open_simulation(dir.path().join("ledger")).unwrap();
    let service = HttpReplyService::new(cfg.http.clone(), None).unwrap();
    let owner = DeviceOwner::acquire_at(&dir.path().join("execution")).unwrap();
    let (host, h) = Host::new(rt, service, cfg.clone(), Channels::default(), owner).unwrap();
    let task = tokio::spawn(host.run());
    assert!(h.status().await.unwrap().paused);
    h.resume().await.unwrap();
    h.observe(observation(&cfg, 0, "new")).await.unwrap();
    tokio::time::sleep(Duration::from_millis(40)).await;
    assert_eq!(h.status().await.unwrap().provider_jobs_started, 0);
    stop(&h, task).await;
}

#[test]
fn device_scope_is_independent_of_ledger_and_replaced_lock_is_detected() {
    let root = private_dir();
    let first = DeviceOwner::acquire_at(root.path()).unwrap();
    let _ledger_a = private_dir();
    let _ledger_b = private_dir();
    assert!(matches!(
        DeviceOwner::acquire_at(root.path()),
        Err(HostError::Busy)
    ));
    drop(first);
    let second = DeviceOwner::acquire_at(root.path()).unwrap();
    second.verify().unwrap();
    #[cfg(unix)]
    {
        std::fs::rename(
            root.path().join("device-owner.lock"),
            root.path().join("old.lock"),
        )
        .unwrap();
        let _replacement = DeviceOwner::acquire_at(root.path()).unwrap();
        assert_eq!(second.verify().unwrap_err(), HostError::Ownership);
    }
}

struct MemoryStore {
    value: Option<Vec<u8>>,
    fail: bool,
    calls: Mutex<usize>,
}
impl CredentialStore for MemoryStore {
    fn load(&self, _: &CredentialRef, _: &str) -> crate::Result<Secret> {
        *self.calls.lock().unwrap() += 1;
        if self.fail {
            return Err(HostError::CredentialUnavailable);
        }
        Secret::new(self.value.clone().ok_or(HostError::CredentialMissing)?)
    }
    fn create(&self, _: &CredentialRef, _: &str, _: &Secret) -> crate::Result<()> {
        Err(HostError::CredentialExists)
    }
}
#[tokio::test]
async fn missing_or_locked_credential_never_creates_plaintext_fallback() {
    let server = Server::start().await;
    let mut cfg = configuration(&server, 1);
    cfg.storage = StorageConfig::Protected {
        key: CredentialRef {
            id: "unit-test".into(),
        },
    };
    for fail in [false, true] {
        let dir = private_dir();
        let path = dir.path().join("not-created");
        let store = MemoryStore {
            value: None,
            fail,
            calls: Mutex::new(0),
        };
        assert!(cfg.open(&path, &store, true).is_err());
        assert!(!path.exists());
        assert!(server.state.lock().unwrap().seen.is_empty());
    }
}
#[tokio::test]
async fn config_rejects_secrets_and_plaintext_requires_explicit_synthetic_opt_in() {
    let server = Server::start().await;
    let cfg = configuration(&server, 1);
    let mut value = serde_json::to_value(&cfg).unwrap();
    value["api_key"] = serde_json::json!("should-never-be-accepted");
    assert!(serde_json::from_value::<HostConfig>(value).is_err());
    let dir = private_dir();
    let store = MemoryStore {
        value: None,
        fail: true,
        calls: Mutex::new(0),
    };
    assert!(cfg.open(&dir.path().join("db"), &store, false).is_err());
    assert_eq!(*store.calls.lock().unwrap(), 0);
    assert!(cfg.open(&dir.path().join("db"), &store, true).is_ok());
}

#[cfg(feature = "encrypted-ledger")]
#[test]
fn encrypted_database_and_wal_hide_body_reopen_and_reject_wrong_key() {
    let dir = private_dir();
    let key = [31u8; 32];
    let marker = "SYNTHETIC_SECRET_BODY_G1C_098712";
    let mut rt = Runtime::open_encrypted(dir.path(), &key).unwrap();
    assert!(rt.is_encrypted());
    let mut b = Binding::paused(fixture_key());
    b.enabled = true;
    b.quiet_ms = 0;
    b.max_wait_ms = 0;
    rt.bind(&b).unwrap();
    rt.ingest(&fixture_observation(&b.key, "one", marker, 1))
        .unwrap();
    for entry in std::fs::read_dir(dir.path()).unwrap() {
        let bytes = std::fs::read(entry.unwrap().path()).unwrap();
        assert!(!bytes.windows(marker.len()).any(|v| v == marker.as_bytes()));
    }
    drop(rt);
    assert!(Runtime::open_encrypted(dir.path(), &[32u8; 32]).is_err());
    assert!(Runtime::open_simulation(dir.path()).is_err());
    let mut reopened = Runtime::open_encrypted(dir.path(), &key).unwrap();
    let request = reopened.begin_reply(&b.key, 2, None).unwrap().unwrap();
    assert_eq!(
        request.input_events[0].message.text.as_deref(),
        Some(marker)
    );
}
#[cfg(feature = "encrypted-ledger")]
#[test]
fn plaintext_is_not_silently_imported_and_ciphertext_tamper_fails() {
    let plain = private_dir();
    let rt = Runtime::open_simulation(plain.path()).unwrap();
    drop(rt);
    let original = std::fs::read(plain.path().join("ledger.sqlite3")).unwrap();
    assert!(Runtime::open_encrypted(plain.path(), &[1; 32]).is_err());
    assert_eq!(
        original,
        std::fs::read(plain.path().join("ledger.sqlite3")).unwrap()
    );
    let cipher = private_dir();
    let rt = Runtime::open_encrypted(cipher.path(), &[2; 32]).unwrap();
    drop(rt);
    let path = cipher.path().join("ledger.sqlite3");
    let mut bytes = std::fs::read(&path).unwrap();
    bytes[100] ^= 1;
    std::fs::write(&path, bytes).unwrap();
    assert!(Runtime::open_encrypted(cipher.path(), &[2; 32]).is_err());
}

#[cfg(feature = "encrypted-ledger")]
#[tokio::test]
async fn protected_config_to_continuous_host_http_and_reopen_is_one_working_path() {
    let server = Server::start().await;
    let mut cfg = configuration(&server, 1);
    cfg.storage = StorageConfig::Protected {
        key: CredentialRef {
            id: "synthetic-encryption-key".into(),
        },
    };
    let dir = private_dir();
    let path = dir.path().join("protected-ledger");
    let store = MemoryStore {
        value: Some(vec![87u8; 32]),
        fail: false,
        calls: Mutex::new(0),
    };
    let (rt, service) = cfg.open(&path, &store, false).unwrap();
    assert!(rt.is_encrypted());
    let owner = DeviceOwner::acquire_at(&dir.path().join("execution")).unwrap();
    let channels = Channels::default();
    let (host, h) = Host::new(rt, service, cfg.clone(), channels.clone(), owner).unwrap();
    let task = tokio::spawn(host.run());
    h.resume().await.unwrap();
    h.observe(observation(&cfg, 0, "protected")).await.unwrap();
    until(&h, |s| {
        s.observed_outgoing_this_run == 1 && s.feedback_acked_this_run >= 1
    })
    .await;
    stop(&h, task).await;
    assert_eq!(channels.count(), 1);
    let rt = Runtime::open_encrypted(&path, &[87u8; 32]).unwrap();
    assert_eq!(
        rt.summary().unwrap().actions[0].1,
        ActionState::VerifiedOutgoing
    );
    assert!(rt.host_is_paused().unwrap());
    assert_eq!(*store.calls.lock().unwrap(), 1);
}

#[tokio::test]
async fn mismatched_http_configuration_is_rejected_before_mutating_the_ledger() {
    let server = Server::start().await;
    let cfg = configuration(&server, 1);
    let mut other = cfg.http.clone();
    other.endpoint = format!("{}/other", server.url);
    let service = HttpReplyService::new(other, None).unwrap();
    let dir = private_dir();
    let path = dir.path().join("ledger");
    let rt = Runtime::open_simulation(&path).unwrap();
    let owner = DeviceOwner::acquire_at(&dir.path().join("execution")).unwrap();
    assert!(matches!(
        Host::new(rt, service, cfg, Channels::default(), owner),
        Err(HostError::Config)
    ));
    let rt = Runtime::open_simulation(&path).unwrap();
    assert!(!rt.host_is_paused().unwrap());
    assert!(server.state.lock().unwrap().seen.is_empty());
}

#[cfg(not(feature = "encrypted-ledger"))]
#[test]
fn disabled_cipher_feature_rejects_protected_entry_without_creating_plaintext() {
    let dir = private_dir();
    let path = dir.path().join("must-not-exist");
    assert!(matches!(
        Runtime::open_encrypted(&path, &[1u8; 32]),
        Err(foxbot_core::Error::ProtectedStore)
    ));
    assert!(!path.exists());
}

#[test]
fn credential_identifiers_and_token_controls_are_not_arbitrary_keychain_queries() {
    for id in ["", "../key", "other:account", "a\nsecret"] {
        assert!(CredentialRef { id: id.into() }.validate().is_err());
    }
    assert!(Secret::new(vec![]).is_err());
    assert!(
        Secret::new(b"bad\ntoken".to_vec())
            .unwrap()
            .token()
            .is_err()
    );
    assert!(Secret::new(vec![1; 31]).unwrap().ledger_key().is_err());
    let secret = credentials::generate_ledger_key().unwrap();
    assert_eq!(secret.bytes().len(), 32);
}
