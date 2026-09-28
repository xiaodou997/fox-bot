#![allow(dead_code)]
use foxbot_core::*;
use foxbot_http::*;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};

#[derive(Clone, Debug)]
pub struct Seen {
    pub path: String,
    pub headers: BTreeMap<String, String>,
    pub body: Value,
    pub raw: Vec<u8>,
}

pub struct State {
    pub seen: Vec<Seen>,
    pub generations: HashMap<String, Value>,
    pub receipts: HashMap<String, Value>,
    pub committed_turns: usize,
    pub generate_delay_ms: u64,
    pub receipt_delay_ms: u64,
    pub generate_status: u16,
    pub receipt_status: u16,
    pub fail_generate_count: usize,
    pub drop_generate_response: usize,
    pub drop_receipt_response: usize,
    pub response_override: Option<Vec<u8>>,
    pub content_type: String,
    pub retry_after: Option<String>,
    pub redirect_to: Option<String>,
    pub outcome: Value,
    pub ack_wrong: bool,
    pub chunked: bool,
}
impl Default for State {
    fn default() -> Self {
        Self {
            seen: vec![],
            generations: HashMap::new(),
            receipts: HashMap::new(),
            committed_turns: 0,
            generate_delay_ms: 0,
            receipt_delay_ms: 0,
            generate_status: 200,
            receipt_status: 200,
            fail_generate_count: 0,
            drop_generate_response: 0,
            drop_receipt_response: 0,
            response_override: None,
            content_type: "application/json".into(),
            retry_after: None,
            redirect_to: None,
            outcome: json!({"result":"reply","text":"合成 HTTP 回答"}),
            ack_wrong: false,
            chunked: false,
        }
    }
}
pub struct Server {
    pub url: String,
    pub state: Arc<Mutex<State>>,
    worker: JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.worker.abort();
    }
}
impl Server {
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let state = Arc::new(Mutex::new(State::default()));
        let shared = state.clone();
        let worker = tokio::spawn(async move {
            let mut children = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let Ok((socket,_)) = accepted else { break; };
                        let state = shared.clone();
                        children.spawn(async move {
                            let _ = tokio::time::timeout(Duration::from_secs(5), handle(socket,state)).await;
                        });
                    }
                    _ = children.join_next(), if !children.is_empty() => {}
                }
            }
            // JoinSet aborts connection workers if this server task is aborted.
        });
        Self { url, state, worker }
    }
    pub fn config(&self, protocol: Protocol, mode: ContextMode) -> HttpConfig {
        let business = protocol == Protocol::BusinessV1;
        HttpConfig {
            protocol,
            endpoint: format!(
                "{}/{}",
                self.url,
                if business {
                    "generate"
                } else {
                    "chat/completions"
                }
            ),
            model: (!business).then(|| "synthetic-model".into()),
            context_mode: mode,
            receipt_endpoint: business.then(|| format!("{}/feedback", self.url)),
            idempotency_supported: business,
            staging_contract: business,
            allow_loopback_http: true,
            attempt_timeout_ms: 1000,
            total_timeout_ms: 3000,
            max_attempts: if business { 2 } else { 1 },
            max_response_bytes: 131_072,
            max_in_flight: 2,
        }
    }
    pub async fn wait_requests(&self, count: usize) {
        tokio::time::timeout(Duration::from_secs(3), async {
            while self.state.lock().unwrap().seen.len() < count {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .expect("local fixture did not receive the expected request");
    }
}

async fn handle(mut socket: TcpStream, state: Arc<Mutex<State>>) -> std::io::Result<()> {
    let mut bytes = Vec::new();
    let header_end = loop {
        if let Some(pos) = bytes.windows(4).position(|s| s == b"\r\n\r\n") {
            break pos + 4;
        }
        let mut buf = [0; 2048];
        let n = socket.read(&mut buf).await?;
        if n == 0 || bytes.len() > 32768 {
            return Ok(());
        }
        bytes.extend_from_slice(&buf[..n]);
    };
    let header = String::from_utf8_lossy(&bytes[..header_end]);
    let path = header
        .lines()
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .nth(1)
        .unwrap_or_default()
        .to_string();
    let headers: BTreeMap<_, _> = header
        .lines()
        .skip(1)
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.to_lowercase(), value.trim().to_string()))
        .collect();
    let size: usize = headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    if size > 1_048_576 {
        return Ok(());
    }
    while bytes.len() < header_end + size {
        let mut buf = [0; 4096];
        let n = socket.read(&mut buf).await?;
        if n == 0 {
            return Ok(());
        }
        bytes.extend_from_slice(&buf[..n]);
    }
    let raw = bytes[header_end..header_end + size].to_vec();
    let body: Value = serde_json::from_slice(&raw).unwrap();
    let (status, response, delay, drop_response, ctype, retry_after, redirect_to, chunked) = {
        let mut s = state.lock().unwrap();
        s.seen.push(Seen {
            path: path.clone(),
            headers,
            body: body.clone(),
            raw,
        });
        let is_feedback = path == "/feedback";
        let mut status = if is_feedback {
            s.receipt_status
        } else {
            s.generate_status
        };
        if !is_feedback && s.fail_generate_count > 0 {
            s.fail_generate_count -= 1;
            status = 503;
        }
        let response = if is_feedback {
            let request_id = body["request_id"].as_str().unwrap().to_string();
            let revision = body["revision"].as_u64().unwrap();
            if status == 200 {
                let prior = s
                    .receipts
                    .get(&request_id)
                    .and_then(|v| v["revision"].as_u64())
                    .unwrap_or(0);
                if revision > prior {
                    if revision == 2 && body["disposition"] != "cancelled" {
                        s.committed_turns += 1;
                    }
                    s.receipts.insert(request_id, body.clone());
                }
            }
            json!({"schema_version":"0.1","receipt_id":if s.ack_wrong {json!("wrong")} else {body["receipt_id"].clone()},
                "revision":revision,"accepted":true})
        } else if path == "/chat/completions" {
            json!({"choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":"合成通用模型回复"}}]})
        } else {
            let request_id = body["request_id"].as_str().unwrap().to_string();
            let response = json!({"schema_version":"0.1","request_id":request_id,"conversation_ref":body["conversation_ref"],
                "in_reply_to":body["input_events"].as_array().unwrap().iter().map(|v|v["event_id"].clone()).collect::<Vec<_>>(),
                "complete":true,"outcome":s.outcome});
            let tombstoned = s
                .receipts
                .get(&request_id)
                .is_some_and(|r| r["revision"] == 2 && r["disposition"] == "cancelled");
            if status == 200 && !tombstoned {
                s.generations
                    .entry(request_id)
                    .or_insert_with(|| response.clone())
                    .clone()
            } else {
                response
            }
        };
        let drop_response = if is_feedback && s.drop_receipt_response > 0 {
            s.drop_receipt_response -= 1;
            true
        } else if !is_feedback && s.drop_generate_response > 0 {
            s.drop_generate_response -= 1;
            true
        } else {
            false
        };
        let data = if !is_feedback {
            s.response_override.clone()
        } else {
            None
        }
        .unwrap_or_else(|| serde_json::to_vec(&response).unwrap());
        (
            status,
            data,
            if is_feedback {
                s.receipt_delay_ms
            } else {
                s.generate_delay_ms
            },
            drop_response,
            s.content_type.clone(),
            s.retry_after.clone(),
            s.redirect_to.clone(),
            s.chunked,
        )
    };
    if delay > 0 {
        tokio::time::sleep(Duration::from_millis(delay)).await;
    }
    if drop_response {
        return Ok(());
    }
    let extra = retry_after
        .map(|v| format!("Retry-After: {v}\r\n"))
        .unwrap_or_default();
    let extra = extra
        + &redirect_to
            .map(|v| format!("Location: {v}\r\n"))
            .unwrap_or_default();
    let framing = if chunked {
        "Transfer-Encoding: chunked\r\n".into()
    } else {
        format!("Content-Length: {}\r\n", response.len())
    };
    let head = format!(
        "HTTP/1.1 {status} Test\r\nContent-Type: {ctype}\r\n{extra}{framing}Connection: close\r\n\r\n"
    );
    socket.write_all(head.as_bytes()).await?;
    if chunked {
        for chunk in response.chunks(64) {
            socket
                .write_all(format!("{:x}\r\n", chunk.len()).as_bytes())
                .await?;
            socket.write_all(chunk).await?;
            socket.write_all(b"\r\n").await?;
        }
        socket.write_all(b"0\r\n\r\n").await?;
    } else {
        socket.write_all(&response).await?;
    }
    socket.shutdown().await
}

pub fn private_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    dir
}
pub fn setup(service: &HttpReplyService) -> (tempfile::TempDir, Runtime, Binding) {
    let dir = private_dir();
    let mut runtime = Runtime::open_simulation(dir.path()).unwrap();
    let mut binding = Binding::paused(simulation::fixture_key());
    binding.enabled = true;
    binding.quiet_ms = 0;
    binding.max_wait_ms = 0;
    binding.provider = service.required_provider_profile("仅使用提供的合成上下文回复".into());
    runtime.bind(&binding).unwrap();
    (dir, runtime, binding)
}
pub fn incoming(runtime: &mut Runtime, key: &ConversationKey, id: &str, now: u64) {
    runtime
        .ingest(&simulation::fixture_observation(key, id, "合成新消息", now))
        .unwrap();
}
pub async fn generate(
    service: &HttpReplyService,
    runtime: &mut Runtime,
    key: &ConversationKey,
    now: u64,
) -> foxbot_http::Result<String> {
    let job = service
        .begin(runtime, key, now, None)?
        .expect("queued input");
    let completion = service.run(job, CancellationToken::new()).await;
    service.finish(runtime, completion, now + 1)
}
pub async fn feedback(
    service: &HttpReplyService,
    runtime: &mut Runtime,
    now: u64,
) -> foxbot_http::Result<bool> {
    let Some(claim) = service.begin_feedback(runtime, now)? else {
        return Ok(false);
    };
    let completion = service.run_feedback(claim, CancellationToken::new()).await;
    service.finish_feedback(runtime, completion, now + 1)?;
    Ok(true)
}
