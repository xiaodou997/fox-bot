//! Minimal local settings UI; it never operates a chat window or starts auto-reply.
use crate::{
    HostError, Result,
    local_config::{ConfigStore, ConnectionInput, Edit, MAX_CONFIG_BYTES, nonce},
};
use foxbot_http::{CancellationToken, HttpError, HttpReplyService};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::Semaphore,
};

const HTML: &str = include_str!("settings/index.html");
const JS: &str = include_str!("settings/app.js");
const CSS: &str = include_str!("settings/style.css");
struct Request {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}
struct State {
    store: ConfigStore,
    token: String,
    origin: String,
    host: String,
    test_slot: Semaphore,
    stop: CancellationToken,
}
struct Response {
    status: u16,
    content_type: &'static str,
    body: Vec<u8>,
}
impl Response {
    fn json(value: serde_json::Value) -> Self {
        Self {
            status: 200,
            content_type: "application/json; charset=utf-8",
            body: value.to_string().into_bytes(),
        }
    }
    fn error(status: u16, message: &str) -> Self {
        let mut r = Self::json(serde_json::json!({"error":message}));
        r.status = status;
        r
    }
    fn asset(content_type: &'static str, body: &str) -> Self {
        Self {
            status: 200,
            content_type,
            body: body.as_bytes().to_vec(),
        }
    }
}
fn display_error(error: &HostError) -> &'static str {
    match error {
        HostError::ConfigChanged => "配置已在其他窗口修改，请刷新后重试。",
        HostError::NoConnection => "请先添加接口并选择默认接口；已删除的接口不会自动替换。",
        HostError::DefaultConnectionInUse => "请先将另一个接口设为默认，再删除这个接口。",
        HostError::KeyRequiredForNewEndpoint => {
            "接口地址已更改，请重新填写 API Key，或勾选无需 API Key。"
        }
        HostError::Busy => "另一个保存或测试正在进行，请稍后重试。",
        HostError::Config => {
            "请检查名称、完整接口地址、模型名和高级选项；远程接口需使用 HTTPS，本地可用 127.0.0.1。"
        }
        _ => "本地配置读写失败，请检查配置目录。原配置未被重置。",
    }
}
fn probe_error(error: &HttpError) -> &'static str {
    match error {
        HttpError::Status {
            code: 401 | 403, ..
        } => "认证失败：请检查 API Key 和接口权限。",
        HttpError::Status { code: 404, .. } => "接口或模型不存在：请检查完整请求地址和模型名称。",
        HttpError::Status {
            code: 400 | 422, ..
        } => "请求参数被拒绝：请检查模型名称与接口协议。",
        HttpError::Status { code: 429, .. } => "服务限流或额度不足，请稍后重试或检查账户。",
        HttpError::Timeout => "连接测试超时，请检查网络或服务状态。",
        HttpError::Transport => "无法连接接口，请检查地址、网络和 HTTPS 证书。",
        HttpError::InvalidResponse | HttpError::BodyTooLarge => {
            "已收到响应，但格式或完整性不符合所选接口协议。"
        }
        HttpError::Cancelled => "测试已取消。",
        _ => "接口测试失败，请检查服务配置。",
    }
}
async fn read_request(socket: &mut TcpStream) -> std::result::Result<Request, ()> {
    let mut bytes = Vec::new();
    let end = loop {
        if let Some(i) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
            break i + 4;
        }
        if bytes.len() > 12_288 {
            return Err(());
        }
        let mut chunk = [0u8; 2048];
        let n = socket.read(&mut chunk).await.map_err(|_| ())?;
        if n == 0 {
            return Err(());
        }
        bytes.extend_from_slice(&chunk[..n]);
    };
    if end > 12_288 {
        return Err(());
    }
    let head = std::str::from_utf8(&bytes[..end]).map_err(|_| ())?;
    let mut lines = head.split("\r\n");
    let first = lines
        .next()
        .ok_or(())?
        .split_whitespace()
        .collect::<Vec<_>>();
    if first.len() != 3 || first[2] != "HTTP/1.1" || !matches!(first[0], "GET" | "POST") {
        return Err(());
    }
    let method = first[0].to_owned();
    let path = first[1].to_owned();
    let mut headers = BTreeMap::new();
    for line in lines.filter(|l| !l.is_empty()) {
        let (key, value) = line.split_once(':').ok_or(())?;
        if key.is_empty() || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            return Err(());
        }
        if headers
            .insert(key.to_ascii_lowercase(), value.trim().to_owned())
            .is_some()
        {
            return Err(());
        }
    }
    if headers.contains_key("transfer-encoding") {
        return Err(());
    }
    let len = headers
        .get("content-length")
        .map(|v| v.parse::<usize>())
        .transpose()
        .map_err(|_| ())?
        .unwrap_or(0);
    if len as u64 > MAX_CONFIG_BYTES || (method == "GET" && len != 0) {
        return Err(());
    }
    while bytes.len() < end + len {
        let mut chunk = [0u8; 8192];
        let n = socket.read(&mut chunk).await.map_err(|_| ())?;
        if n == 0 {
            return Err(());
        }
        bytes.extend_from_slice(&chunk[..n]);
    }
    if bytes.len() != end + len {
        return Err(());
    } // No request pipelining.
    Ok(Request {
        method,
        path,
        headers,
        body: bytes[end..].to_vec(),
    })
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Change {
    revision: String,
    edit: Edit,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Probe {
    connection: ConnectionInput,
    confirm_billable: bool,
}
async fn route(request: Request, state: &State) -> Response {
    if request.headers.get("host") != Some(&state.host)
        || request
            .headers
            .get("origin")
            .is_some_and(|o| o != &state.origin)
        || request
            .headers
            .get("sec-fetch-site")
            .is_some_and(|s| !matches!(s.as_str(), "same-origin" | "none"))
    {
        return Response::error(403, "仅允许本机设置页面访问。");
    }
    if request.method == "GET" {
        match request.path.as_str() {
            "/" => return Response::asset("text/html; charset=utf-8", HTML),
            "/app.js" => return Response::asset("text/javascript; charset=utf-8", JS),
            "/style.css" => return Response::asset("text/css; charset=utf-8", CSS),
            _ => {}
        }
    }
    if request.headers.get("x-foxbot-session") != Some(&state.token) {
        return Response::error(403, "设置会话已失效，请重新打开 FoxBot 设置。");
    }
    if request.method == "POST"
        && request
            .headers
            .get("content-type")
            .is_none_or(|v| v.split(';').next() != Some("application/json"))
    {
        return Response::error(415, "需要 JSON 请求。");
    }
    let result: Result<serde_json::Value> = match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/api/config") => state.store.view(),
        ("GET", "/api/export") => state.store.load().and_then(|(c, _)| c.export()),
        ("POST", "/api/edit") => match serde_json::from_slice::<Change>(&request.body) {
            Ok(change) => state.store.edit(&change.revision, change.edit),
            Err(_) => Err(HostError::Config),
        },
        ("POST", "/api/test") => {
            let input = match serde_json::from_slice::<Probe>(&request.body) {
                Ok(input) if input.confirm_billable => input,
                _ => return Response::error(400, "连接测试会产生一次 API 请求，需要确认。"),
            };
            let Ok(_permit) = state.test_slot.try_acquire() else {
                return Response::error(409, "已有连接测试正在进行。");
            };
            let connection = match state.store.test_draft(input.connection) {
                Ok(c) => c,
                Err(e) => return Response::error(400, display_error(&e)),
            };
            let mut http = connection.http();
            http.attempt_timeout_ms = 15_000;
            http.total_timeout_ms = 15_000;
            let service = match HttpReplyService::new(
                http,
                (!connection.api_key.is_empty()).then_some(connection.api_key.as_str()),
            ) {
                Ok(s) => s,
                Err(e) => return Response::error(400, probe_error(&e)),
            };
            let id = match nonce() {
                Ok(id) => id,
                Err(_) => return Response::error(500, "无法创建测试请求。"),
            };
            let start = Instant::now();
            let tested = service.test_connection(&id, state.stop.child_token()).await;
            return Response::json(
                serde_json::json!({"ok":tested.is_ok(),"elapsed_ms":start.elapsed().as_millis(),
                "message":tested.as_ref().err().map(probe_error).unwrap_or("连接成功，接口已返回符合协议的响应。"),
                "generation_attempts":1,"chat_operations":0,"saved":false}),
            );
        }
        ("POST", "/api/open-folder") => {
            match state
                .store
                .path()
                .parent()
                .ok_or(HostError::Config)
                .and_then(open_path)
            {
                Ok(()) => Ok(serde_json::json!({"opened":true})),
                Err(e) => Err(e),
            }
        }
        ("POST", "/api/shutdown") => {
            state.stop.cancel();
            Ok(serde_json::json!({"stopped":true}))
        }
        _ => return Response::error(404, "页面不存在。"),
    };
    match result {
        Ok(v) => Response::json(v),
        Err(e) => Response::error(
            if e == HostError::ConfigChanged {
                409
            } else {
                400
            },
            display_error(&e),
        ),
    }
}
async fn handle(mut socket: TcpStream, state: Arc<State>) {
    let response =
        match tokio::time::timeout(Duration::from_secs(5), read_request(&mut socket)).await {
            Ok(Ok(request)) => route(request, &state).await,
            _ => Response::error(400, "请求无效或读取超时。"),
        };
    let status = match response.status {
        200 => "OK",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        415 => "Unsupported Media Type",
        _ => "Bad Request",
    };
    let head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'\r\n\r\n",
        response.status,
        status,
        response.content_type,
        response.body.len()
    );
    let _ = tokio::time::timeout(Duration::from_secs(5), async {
        socket.write_all(head.as_bytes()).await?;
        socket.write_all(&response.body).await?;
        socket.shutdown().await
    })
    .await;
}
fn open_target(target: &std::ffi::OsStr) -> Result<()> {
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("/usr/bin/open");
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut c = std::process::Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let mut command = std::process::Command::new("xdg-open");
    let status = command
        .arg(target)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|_| HostError::Config)?;
    if status.success() {
        Ok(())
    } else {
        Err(HostError::Config)
    }
}
fn open_path(path: &Path) -> Result<()> {
    open_target(path.as_os_str())
}

pub async fn serve(path: PathBuf, open_browser: bool) -> Result<()> {
    let store = ConfigStore::new(path);
    store.ensure()?;
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|_| HostError::Busy)?;
    let host = listener
        .local_addr()
        .map_err(|_| HostError::Config)?
        .to_string();
    let token = nonce()?;
    let origin = format!("http://{host}");
    let url = format!("{origin}/#{token}");
    let stop = CancellationToken::new();
    let state = Arc::new(State {
        store,
        token,
        origin,
        host,
        test_slot: Semaphore::new(1),
        stop: stop.clone(),
    });
    println!(
        "{}",
        serde_json::json!({"event":"settings_ready","url":url,"config_path":state.store.path(),"chat_operations":0})
    );
    if open_browser && open_target(std::ffi::OsStr::new(&url)).is_err() {
        eprintln!("无法自动打开浏览器，请打开上面的本机设置地址。");
    }
    run(listener, state).await
}
async fn run(listener: TcpListener, state: Arc<State>) -> Result<()> {
    let permits = Arc::new(Semaphore::new(8));
    let mut tasks = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            _=state.stop.cancelled()=>break,
            _=tokio::signal::ctrl_c()=>{state.stop.cancel();break;},
            Some(_)=tasks.join_next(),if !tasks.is_empty()=>{},
            connection=listener.accept()=>{
                let (socket,peer)=connection.map_err(|_| HostError::Closed)?;
                if !peer.ip().is_loopback() { continue; }
                let Ok(permit)=permits.clone().try_acquire_owned() else { continue; };
                let shared=state.clone();
                tasks.spawn(async move {let _permit=permit;handle(socket,shared).await;});
            }
        }
    }
    // Allow the shutdown response and cancelled probe to finish, without detached tasks.
    let _ = tokio::time::timeout(Duration::from_secs(6), async {
        while tasks.join_next().await.is_some() {}
    })
    .await;
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    Ok(())
}

#[cfg(test)]
#[path = "settings_tests.rs"]
mod tests;
