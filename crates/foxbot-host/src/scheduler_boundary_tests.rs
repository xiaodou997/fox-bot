//! Deterministic tick-boundary regressions. No external model or native input.
use super::*;
use foxbot_core::{Direction, simulation::*};

async fn ready_host() -> (
    tempfile::TempDir,
    Host<MockChannel>,
    HostHandle,
    tokio::net::TcpListener,
    String,
) {
    let directory = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut config: HostConfig =
        serde_json::from_str(include_str!("../../../examples/host-synthetic.json")).unwrap();
    config.http.endpoint = format!("http://{}/generate", listener.local_addr().unwrap());
    config.http.receipt_endpoint = Some(format!(
        "http://{}/feedback",
        listener.local_addr().unwrap()
    ));
    config.bindings.truncate(1);
    config.bindings[0].quiet_ms = 0;
    config.bindings[0].max_wait_ms = 0;
    let key = config.bindings[0].key.clone();
    let owner = DeviceOwner::acquire_at(&directory.path().join("owner")).unwrap();
    let runtime = Runtime::open_simulation(directory.path().join("ledger")).unwrap();
    let service = HttpReplyService::new(config.http.clone(), None).unwrap();
    let (mut host, handle) =
        Host::new(runtime, service, config, MockChannel::new(&key), owner).unwrap();
    host.runtime.set_host_paused(false).unwrap();
    host.stats.paused = false;
    let now = host.clock.now_ms();
    host.runtime
        .ingest(&fixture_observation(
            &key,
            "old",
            "synthetic old question",
            now,
        ))
        .unwrap();
    let request = host
        .runtime
        .generate_once(
            &key,
            now,
            None,
            &mut FixedReply::new("synthetic old answer"),
        )
        .unwrap()
        .unwrap();
    (directory, host, handle, listener, request)
}

#[tokio::test]
async fn queued_new_input_is_ingested_before_a_ready_reply_can_send() {
    let (_dir, mut host, handle, _listener, old) = ready_host().await;
    let key = host.bindings[0].key.clone();
    let (ack, receipt) = oneshot::channel();
    handle
        .input
        .try_send(Input {
            observation: fixture_observation(
                &key,
                "new",
                "synthetic follow-up",
                host.clock.now_ms(),
            ),
            epoch: handle.epoch.load(Ordering::Acquire),
            ack,
        })
        .unwrap();
    host.tick().unwrap();
    assert_eq!(
        host.channel.send_calls, 0,
        "tick must not overtake accepted input"
    );
    assert_eq!(host.runtime.task_state(&old).unwrap(), "STALE");
    assert!(receipt.await.unwrap().is_ok());
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn queued_manual_output_cancels_ready_reply_before_dispatch() {
    let (_dir, mut host, handle, _listener, old) = ready_host().await;
    let key = host.bindings[0].key.clone();
    let mut observation =
        fixture_observation(&key, "human", "synthetic human answer", host.clock.now_ms());
    observation.message.direction = Direction::Own;
    let (ack, receipt) = oneshot::channel();
    handle
        .input
        .try_send(Input {
            observation,
            epoch: handle.epoch.load(Ordering::Acquire),
            ack,
        })
        .unwrap();
    host.tick().unwrap();
    assert_eq!(host.channel.send_calls, 0);
    assert_eq!(host.runtime.task_state(&old).unwrap(), "STALE");
    assert!(receipt.await.unwrap().is_ok());
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_stop_signal_observed_at_tick_boundary_prevents_side_effects() {
    let (_dir, mut host, handle, _listener, _) = ready_host().await;
    handle.stop();
    host.tick().unwrap();
    assert_eq!(host.channel.fill_calls, 0);
    assert_eq!(host.channel.send_calls, 0);
    host.shutdown().await.unwrap();
}
