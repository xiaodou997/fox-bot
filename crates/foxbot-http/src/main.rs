//! Explicit synthetic-data HTTP harness. It never reads or operates chat applications.
use foxbot_core::{simulation::*, *};
use foxbot_http::{CancellationToken, HttpConfig, HttpError, HttpReplyService, RunClock};
use std::{env, fs::File, io::Read, path::PathBuf};

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("foxbot-http: {error}");
        std::process::exit(1);
    }
}
async fn run() -> foxbot_http::Result<()> {
    let args: Vec<_> = env::args().skip(1).collect();
    if args.is_empty() || matches!(args[0].as_str(), "help" | "--help" | "-h") {
        println!(
            "foxbot-http <synthetic-run|feedback|inspect> CONFIG_JSON STATE_DIR [--allow-network]\n\
            Synthetic data only. Generation and feedback require explicit --allow-network.\n\
            Optional bearer token: FOXBOT_HTTP_TOKEN environment variable. Never include keys in config or argv."
        );
        return Ok(());
    }
    if args.len() < 3
        || args.len() > 4
        || !matches!(args[0].as_str(), "synthetic-run" | "feedback" | "inspect")
        || (args[0] != "inspect" && args.get(3).map(String::as_str) != Some("--allow-network"))
        || (args.len() == 4 && args[3] != "--allow-network")
    {
        return Err(HttpError::Config);
    }
    let mut bytes = Vec::new();
    File::open(&args[1])
        .map_err(|_| HttpError::Config)?
        .take(65_537)
        .read_to_end(&mut bytes)
        .map_err(|_| HttpError::Config)?;
    if bytes.len() > 65_536 {
        return Err(HttpError::Config);
    }
    let config: HttpConfig = serde_json::from_slice(&bytes).map_err(|_| HttpError::Config)?;
    let token = match env::var("FOXBOT_HTTP_TOKEN") {
        Ok(token) => Some(token),
        Err(env::VarError::NotPresent) => None,
        Err(_) => return Err(HttpError::Config),
    };
    let service = HttpReplyService::new(config, token.as_deref())?;
    let clock = RunClock::default();
    let mut runtime = Runtime::open_simulation(PathBuf::from(&args[2]))?;
    let mut provider_calls = 0;
    let mut send_calls = 0;
    let mut feedback_acks = 0;
    if args[0] == "synthetic-run" {
        let key = fixture_key();
        let mut binding = Binding::paused(key.clone());
        binding.enabled = true;
        binding.quiet_ms = 0;
        binding.max_wait_ms = 0;
        // Do not override a configured service's role or knowledge base.
        // A host may opt into ProviderProfile::Generic with an explicit prompt.
        binding.provider = ProviderProfile::Custom;
        binding.profile_version = (u64::from_str_radix(&service.profile_tag()[..16], 16)
            .map_err(|_| HttpError::Config)?
            & i64::MAX as u64)
            .max(1);
        runtime.bind(&binding)?;
        let mut history = fixture_observation(
            &key,
            "http-history",
            "合成历史，不应触发回复",
            clock.now_ms(),
        );
        history.historical = true;
        runtime.ingest(&history)?;
        runtime.ingest(&fixture_observation(
            &key,
            "http-incoming",
            "合成测试：请介绍安装步骤",
            clock.now_ms(),
        ))?;
        if let Some(job) = service.begin(&mut runtime, &key, clock.now_ms(), None)? {
            provider_calls += 1;
            let completion = service.run(job, CancellationToken::new()).await;
            let request = service.finish(&mut runtime, completion, clock.now_ms())?;
            if runtime.task_state(&request)? == "READY" {
                let action = runtime.prepare_send(&request, clock.now_ms(), false)?;
                let mut channel = MockChannel::new(&key);
                runtime.dispatch(&action, clock.now_ms(), &mut channel)?;
                send_calls = channel.send_calls;
            }
        }
    }
    if args[0] != "inspect" {
        // Bounded drain; a failed receipt has its own durable backoff and exits here.
        for _ in 0..8 {
            let Some(claim) = service.begin_feedback(&mut runtime, clock.now_ms())? else {
                break;
            };
            let completion = service.run_feedback(claim, CancellationToken::new()).await;
            service.finish_feedback(&mut runtime, completion, clock.now_ms())?;
            feedback_acks += 1;
        }
    }
    println!("{}",serde_json::to_string_pretty(&serde_json::json!({
        "synthetic_data_only":true,"native_chat_operations":0,
        "provider_jobs_this_run":provider_calls,"mock_send_calls_this_run":send_calls,
        "feedback_acks_this_run":feedback_acks,"ledger":runtime.summary()?,"feedback_queue":runtime.service_queue_summary()?
    })).map_err(|_|HttpError::Core)?);
    Ok(())
}
