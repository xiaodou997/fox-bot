use super::*;
use crate::{local_config::*, tests::support::Server};
use serde_json::{Value, json};
fn state(path: &Path) -> State {
    let store = ConfigStore::new(path.join("settings/config.json"));
    store.ensure().unwrap();
    State {
        store,
        token: "test-session".into(),
        origin: "http://127.0.0.1:54321".into(),
        host: "127.0.0.1:54321".into(),
        test_slot: Semaphore::new(1),
        stop: CancellationToken::new(),
    }
}
fn request(method: &str, path: &str, body: Value) -> Request {
    Request {
        method: method.into(),
        path: path.into(),
        headers: BTreeMap::from([
            ("host".into(), "127.0.0.1:54321".into()),
            ("x-foxbot-session".into(), "test-session".into()),
            ("origin".into(), "http://127.0.0.1:54321".into()),
            ("content-type".into(), "application/json".into()),
        ]),
        body: body.to_string().into_bytes(),
    }
}
fn connection(id: &str, endpoint: &str, key: &str) -> Value {
    json!({"id":id,"name":format!("接口{id}"),"protocol":"chat_completions","endpoint":endpoint,"model":"synthetic-model","api_key":key})
}
fn edit(value: Value) -> Edit {
    serde_json::from_value(value).unwrap()
}
fn save(s: &State, c: Value) -> Value {
    let (_, revision) = s.store.load().unwrap();
    s.store
        .edit(&revision, edit(json!({"action":"save","connection":c})))
        .unwrap()
}
#[test]
fn multiple_connections_persist_inline_keys_but_ui_and_export_never_return_them() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path());
    let a = save(
        &s,
        connection(
            "a",
            "https://a.example/v1/chat/completions",
            "private-test-key-a",
        ),
    );
    assert_eq!(a["config"]["default_connection"], "a");
    let b = save(
        &s,
        connection(
            "b",
            "https://b.example/v1/chat/completions",
            "private-test-key-b",
        ),
    );
    assert!(!b.to_string().contains("private-test-key"));
    assert_eq!(b["config"]["connections"][0]["has_api_key"], true);
    let bytes = std::fs::read_to_string(s.store.path()).unwrap();
    assert!(bytes.contains("private-test-key-a"));
    let (loaded, rev) = s.store.load().unwrap();
    let exported = loaded.export().unwrap();
    assert!(!exported.to_string().contains("api_key"));
    serde_json::from_value::<LocalConfig>(exported)
        .unwrap()
        .validate()
        .unwrap();
    s.store
        .edit(&rev, edit(json!({"action":"set_default","id":"b"})))
        .unwrap();
    assert_eq!(s.store.load().unwrap().0.selected(None).unwrap().id, "b");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(s.store.path())
                .unwrap()
                .permissions()
                .mode()
                & 0o077,
            0
        );
    }
}
#[test]
fn omitted_key_preserves_explicit_empty_clears_and_stale_save_cannot_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path());
    let first = save(
        &s,
        connection(
            "a",
            "https://a.example/v1/chat/completions",
            "private-test-key",
        ),
    );
    let mut changed = connection("a", "https://a.example/v1/chat/completions", "");
    changed.as_object_mut().unwrap().remove("api_key");
    changed["name"] = "renamed".into();
    save(&s, changed);
    assert_eq!(
        s.store.load().unwrap().0.connections[0].api_key,
        "private-test-key"
    );
    let before = std::fs::read(s.store.path()).unwrap();
    assert_eq!(
        s.store
            .edit(
                first["revision"].as_str().unwrap(),
                edit(json!({"action":"delete","id":"a"}))
            )
            .err(),
        Some(HostError::ConfigChanged)
    );
    assert_eq!(std::fs::read(s.store.path()).unwrap(), before);
    save(
        &s,
        connection("a", "https://a.example/v1/chat/completions", ""),
    );
    assert!(s.store.load().unwrap().0.connections[0].api_key.is_empty());
}
#[test]
fn copy_preserves_key_default_delete_does_not_silently_switch_and_missing_pin_fails() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path());
    let v = save(
        &s,
        connection(
            "a",
            "https://a.example/v1/chat/completions",
            "private-test-key",
        ),
    );
    let v = s
        .store
        .edit(
            v["revision"].as_str().unwrap(),
            edit(json!({"action":"duplicate","id":"a"})),
        )
        .unwrap();
    let (config, _) = s.store.load().unwrap();
    assert_eq!(config.connections.len(), 2);
    assert_eq!(config.connections[1].api_key, "private-test-key");
    assert_eq!(config.default_connection.as_deref(), Some("a"));
    assert_eq!(
        s.store
            .edit(
                v["revision"].as_str().unwrap(),
                edit(json!({"action":"delete","id":"a"}))
            )
            .err(),
        Some(HostError::DefaultConnectionInUse)
    );
    assert!(config.selected(Some("missing")).is_err());
}
#[test]
fn changed_endpoint_cannot_implicitly_receive_a_saved_key() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path());
    save(
        &s,
        connection(
            "a",
            "https://a.example/v1/chat/completions",
            "private-test-key",
        ),
    );
    let mut c = connection("a", "https://other.example/v1/chat/completions", "");
    c.as_object_mut().unwrap().remove("api_key");
    assert!(matches!(
        s.store.test_draft(serde_json::from_value(c).unwrap()),
        Err(HostError::KeyRequiredForNewEndpoint)
    ));
}
#[test]
fn invalid_existing_config_is_not_replaced_and_bad_ids_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path());
    std::fs::write(s.store.path(), b"broken json").unwrap();
    assert!(s.store.ensure().is_err());
    assert_eq!(std::fs::read(s.store.path()).unwrap(), b"broken json");
    for id in ["", "../a", "a/b", "a\\b", "a\n"] {
        assert!(!identifier(id));
    }
}
#[tokio::test]
async fn settings_api_rejects_cross_origin_wrong_host_missing_token_and_unconfirmed_test() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path());
    for header in ["origin", "host", "x-foxbot-session"] {
        let mut q = request("GET", "/api/config", json!(null));
        q.headers.insert(header.into(), "wrong".into());
        assert_eq!(route(q, &s).await.status, 403);
    }
    let mut q = request("GET", "/api/config", json!(null));
    q.headers.remove("x-foxbot-session");
    assert_eq!(route(q, &s).await.status, 403);
    let c = connection("a", "https://not-contacted.example/v1/chat/completions", "");
    let r = route(
        request(
            "POST",
            "/api/test",
            json!({"connection":c,"confirm_billable":false}),
        ),
        &s,
    )
    .await;
    assert_eq!(r.status, 400);
    assert!(s.store.load().unwrap().0.connections.is_empty());
}
#[tokio::test]
async fn connection_test_uses_only_fixed_text_one_request_and_does_not_save() {
    let server = Server::start().await;
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path());
    let c = connection(
        "a",
        &format!("{}/chat/completions", server.url),
        "synthetic-api-key",
    );
    let original = std::fs::read(s.store.path()).unwrap();
    let r = route(
        request(
            "POST",
            "/api/test",
            json!({"connection":c,"confirm_billable":true}),
        ),
        &s,
    )
    .await;
    let body: Value = serde_json::from_slice(&r.body).unwrap();
    assert_eq!(body["ok"], true);
    assert_eq!(body["chat_operations"], 0);
    assert!(
        !String::from_utf8(r.body)
            .unwrap()
            .contains("synthetic-api-key")
    );
    assert_eq!(original, std::fs::read(s.store.path()).unwrap());
    let state = server.state.lock().unwrap();
    assert_eq!(state.seen.len(), 1);
    assert_eq!(
        state.seen[0].headers["authorization"],
        "Bearer synthetic-api-key"
    );
    assert!(state.seen[0].body.to_string().contains("连接测试"));
    assert!(!state.seen[0].body.to_string().contains("history-a"));
}
#[tokio::test]
async fn failed_test_has_friendly_error_no_response_body_and_does_not_prevent_saving() {
    let server = Server::start().await;
    server.state.lock().unwrap().generate_status = 401;
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path());
    let c = connection(
        "a",
        &format!("{}/chat/completions", server.url),
        "synthetic-key",
    );
    let r = route(
        request(
            "POST",
            "/api/test",
            json!({"connection":c.clone(),"confirm_billable":true}),
        ),
        &s,
    )
    .await;
    let body: Value = serde_json::from_slice(&r.body).unwrap();
    assert_eq!(body["ok"], false);
    assert!(body["message"].as_str().unwrap().contains("认证失败"));
    save(&s, c);
    assert_eq!(s.store.load().unwrap().0.connections.len(), 1);
    assert_eq!(server.state.lock().unwrap().seen.len(), 1);
}
#[tokio::test]
async fn browser_wire_parser_handles_post_and_rejects_duplicate_or_chunked_headers() {
    async fn parse(raw: &[u8]) -> std::result::Result<Request, ()> {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let bytes = raw.to_vec();
        let client = tokio::spawn(async move {
            let mut s = TcpStream::connect(addr).await.unwrap();
            s.write_all(&bytes).await.unwrap();
            s.shutdown().await.unwrap();
        });
        let (mut socket, _) = listener.accept().await.unwrap();
        let result = read_request(&mut socket).await;
        client.await.unwrap();
        result
    }
    let valid = b"POST /api/edit HTTP/1.1\r\nHost: 127.0.0.1:1\r\nContent-Length: 2\r\n\r\n{}";
    assert_eq!(parse(valid).await.unwrap().body, b"{}");
    for bad in [
        b"GET / HTTP/1.1\r\nHost: x\r\nHost: y\r\n\r\n".as_slice(),
        b"POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n".as_slice(),
    ] {
        assert!(parse(bad).await.is_err());
    }
}
#[test]
fn assets_use_no_remote_scripts_and_no_secret_interpolation() {
    assert!(HTML.contains("/app.js"));
    assert!(!HTML.contains("<script>"));
    assert!(!JS.contains("innerHTML"));
    assert!(!JS.contains("localStorage"));
    assert!(JS.contains("c.api_key = ''"));
    assert!(CSS.contains("@media"));
}
