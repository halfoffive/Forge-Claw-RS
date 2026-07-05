//! 鉴权集成测试：覆盖中间件 401、login 端点、用户会话隔离、WS query token 鉴权。
//!
//! 用 `tower::ServiceExt::oneshot` 发请求，不打真实 LLM API（MockClient 返回空 Done）。

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use forgeclaw_core::model::Session;
use forgeclaw_llm::{ChatRequest, Event, History, LlmClient};
use forgeclaw_server::{app, AppState, Orchestrator, SessionData, UserStore};
use futures::stream::BoxStream;
use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tempfile::tempdir;
use tokio::sync::{Notify, RwLock};
use tokio::time;
use tower::ServiceExt;
use uuid::Uuid;

const ALICE_TOKEN: &str = "alice-token";
const BOB_TOKEN: &str = "bob-token";

/// 脚本化 Mock LLM 客户端：返回空 Done 流（本测试套不触发 chat）。
struct MockClient {
    counter: AtomicUsize,
}

#[async_trait]
impl LlmClient for MockClient {
    async fn chat(&self, _req: ChatRequest) -> anyhow::Result<BoxStream<'static, Event>> {
        self.counter.fetch_add(1, Ordering::SeqCst);
        Ok(Box::pin(futures::stream::iter(vec![Event::Done])))
    }
}

/// 构造带两个用户（alice/bob）的 AppState。
fn build_state() -> (AppState, tempfile::TempDir) {
    let dir = tempdir().unwrap();
    let (sandbox, specs) = forgeclaw_server::default_sandbox_with_specs(dir.path().to_path_buf());
    let prompts_root =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../prompts/profiles");
    let llm: Arc<dyn LlmClient> = Arc::new(MockClient {
        counter: AtomicUsize::new(0),
    });
    build_state_with_llm(llm, dir, sandbox, specs, prompts_root)
}

fn build_state_with_llm(
    llm: Arc<dyn LlmClient>,
    dir: tempfile::TempDir,
    sandbox: forgeclaw_tools::Sandbox,
    specs: Vec<forgeclaw_llm::ToolSpec>,
    prompts_root: std::path::PathBuf,
) -> (AppState, tempfile::TempDir) {
    let orch = Orchestrator::new(
        llm,
        Arc::new(sandbox),
        specs,
        prompts_root,
        "default".into(),
        "deepseek-chat".into(),
        dir.path().to_path_buf(),
    );
    let user_store = UserStore::from_config(vec![
        ("alice".into(), ALICE_TOKEN.into()),
        ("bob".into(), BOB_TOKEN.into()),
    ]);
    (AppState::new(Arc::new(orch), user_store), dir)
}

async fn body_to_json(body: Body) -> Value {
    let bytes = to_bytes(body, 1024 * 1024).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

// ============ 中间件 401 用例 ============

#[tokio::test]
async fn no_token_returns_401() {
    let (state, _dir) = build_state();
    let response = app(state)
        .oneshot(
            Request::builder()
                .uri("/api/sessions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let v = body_to_json(response.into_body()).await;
    assert_eq!(v["error"], "unauthorized");
}

#[tokio::test]
async fn wrong_token_returns_401() {
    let (state, _dir) = build_state();
    let response = app(state)
        .oneshot(
            Request::builder()
                .uri("/api/sessions")
                .header("authorization", "Bearer not-a-real-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn malformed_authorization_header_returns_401() {
    let (state, _dir) = build_state();
    // 缺少 Bearer 前缀
    let response = app(state)
        .oneshot(
            Request::builder()
                .uri("/api/sessions")
                .header("authorization", ALICE_TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn valid_token_returns_200() {
    let (state, _dir) = build_state();
    let response = app(state)
        .oneshot(
            Request::builder()
                .uri("/api/sessions")
                .header("authorization", format!("Bearer {ALICE_TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

// ============ Login 端点 ============

#[tokio::test]
async fn login_with_correct_credentials_returns_user() {
    let (state, _dir) = build_state();
    let body = serde_json::to_vec(&json!({"name":"alice","token":ALICE_TOKEN})).unwrap();
    let response = app(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let v = body_to_json(response.into_body()).await;
    assert_eq!(v["ok"], true);
    assert_eq!(v["user"]["name"], "alice");
    assert!(v["user"]["token"].is_null(), "LoginResponse 不应包含 token");
    assert!(v["user"]["id"].is_string());
    assert!(v["ticket"].is_string());
}

#[tokio::test]
async fn login_with_wrong_token_returns_401() {
    let (state, _dir) = build_state();
    let body = serde_json::to_vec(&json!({"name":"alice","token":"wrong"})).unwrap();
    let response = app(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn login_with_unknown_user_returns_401() {
    let (state, _dir) = build_state();
    let body = serde_json::to_vec(&json!({"name":"eve","token":"whatever"})).unwrap();
    let response = app(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn login_endpoint_does_not_require_auth() {
    // login 端点本身不挂中间件：无 authorization header 也能命中
    let (state, _dir) = build_state();
    let body = serde_json::to_vec(&json!({"name":"alice","token":ALICE_TOKEN})).unwrap();
    let response = app(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn login_rate_limit_blocks_after_5_attempts() {
    // 同一 IP + name 在 60s 窗口内最多 5 次尝试，第 6 次应返回 429。
    // 必须复用同一个 app（Router）， GovernorLayer 内部状态才共享。
    let (state, _dir) = build_state();
    let app = app(state);
    let body = serde_json::to_vec(&json!({"name":"alice","token":"wrong"})).unwrap();
    for i in 0..6 {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/login")
                    .header("content-type", "application/json")
                    .body(Body::from(body.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        if i < 5 {
            assert_eq!(
                response.status(),
                StatusCode::UNAUTHORIZED,
                "attempt {} should be 401",
                i + 1
            );
        } else {
            assert_eq!(
                response.status(),
                StatusCode::TOO_MANY_REQUESTS,
                "attempt {} should be rate limited",
                i + 1
            );
        }
    }
}

// ============ 用户会话隔离 ============

/// 直接往 sessions 存储注入一个属于 alice 的会话，绕过 LLM。
async fn insert_session_for(state: &AppState, user_name: &str) -> Uuid {
    let user = state
        .user_store
        .find_by_name(user_name)
        .expect("user exists");
    let sid = Uuid::new_v4();
    let mut sessions = state.sessions.write().await;
    sessions.insert(
        sid,
        SessionData {
            session: Session::new(sid),
            history: Arc::new(RwLock::new(History::new())),
            user_id: user.id,
        },
    );
    sid
}

#[tokio::test]
async fn isolation_alice_sees_own_sessions_bob_sees_none() {
    let (state, _dir) = build_state();
    let alice_sid = insert_session_for(&state, "alice").await;

    // Alice 列表 → 看到 1 个
    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .uri("/api/sessions")
                .header("authorization", format!("Bearer {ALICE_TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let v = body_to_json(response.into_body()).await;
    let arr = v.as_array().expect("array");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"], alice_sid.to_string());

    // Bob 列表 → 看到 0 个
    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .uri("/api/sessions")
                .header("authorization", format!("Bearer {BOB_TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let v = body_to_json(response.into_body()).await;
    assert_eq!(v.as_array().expect("array").len(), 0);
}

#[tokio::test]
async fn isolation_bob_get_alice_session_returns_404() {
    let (state, _dir) = build_state();
    let alice_sid = insert_session_for(&state, "alice").await;

    // Bob 直接访问 alice 的 session_id → 404（不泄漏存在性）
    let response = app(state)
        .oneshot(
            Request::builder()
                .uri(format!("/api/sessions/{alice_sid}"))
                .header("authorization", format!("Bearer {BOB_TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn isolation_alice_get_own_session_returns_200() {
    let (state, _dir) = build_state();
    let alice_sid = insert_session_for(&state, "alice").await;

    let response = app(state)
        .oneshot(
            Request::builder()
                .uri(format!("/api/sessions/{alice_sid}"))
                .header("authorization", format!("Bearer {ALICE_TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let v = body_to_json(response.into_body()).await;
    assert_eq!(v["id"], alice_sid.to_string());
}

// ============ WebSocket ticket 鉴权（升级前 401） ============

/// 构造一个 WS 升级请求（带必要头）。
fn ws_request(uri: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .header("upgrade", "websocket")
        .header("connection", "upgrade")
        .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
        .header("sec-websocket-version", "13")
        .body(Body::empty())
        .unwrap()
}

async fn login_ticket(state: AppState, name: &str, token: &str) -> String {
    let body = serde_json::to_vec(&json!({"name": name, "token": token})).unwrap();
    let response = app(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let v = body_to_json(response.into_body()).await;
    v["ticket"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn ws_without_ticket_returns_401() {
    let (state, _dir) = build_state();
    let response = app(state).oneshot(ws_request("/ws/chat")).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn ws_with_wrong_ticket_returns_401() {
    let (state, _dir) = build_state();
    let response = app(state)
        .oneshot(ws_request(
            "/ws/chat?ticket=00000000-0000-0000-0000-000000000000",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn ws_with_valid_ticket_passes_auth() {
    let (state, _dir) = build_state();
    let ticket = login_ticket(state.clone(), "alice", ALICE_TOKEN).await;
    let response = app(state)
        .oneshot(ws_request(&format!("/ws/chat?ticket={ticket}")))
        .await
        .unwrap();
    // 有效 ticket 通过鉴权后，oneshot 无真实 hyper 连接 → WebSocketUpgrade 返回 426
    // （ConnectionNotUpgradable）。关键：不是 401，证明 ticket 校验已通过。
    assert_ne!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(response.status(), StatusCode::UPGRADE_REQUIRED);
}

#[tokio::test]
async fn ws_ticket_is_single_use() {
    let (state, _dir) = build_state();
    let ticket = login_ticket(state.clone(), "alice", ALICE_TOKEN).await;
    let response = app(state.clone())
        .oneshot(ws_request(&format!("/ws/chat?ticket={ticket}")))
        .await
        .unwrap();
    assert_ne!(response.status(), StatusCode::UNAUTHORIZED);

    // 第二次使用同一张 ticket → 401。
    let response = app(state)
        .oneshot(ws_request(&format!("/ws/chat?ticket={ticket}")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn ws_cross_user_session_returns_error_and_does_not_overwrite() {
    let (state, _dir) = build_state();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let test_state = state.clone();
    tokio::spawn(async move {
        axum::serve(listener, app(state)).await.unwrap();
    });

    let alice = test_state.user_store.find_by_name("alice").unwrap();
    let bob = test_state.user_store.find_by_name("bob").unwrap();

    // Alice 创建会话。
    let alice_ticket = test_state.issue_ticket(alice.id).await;
    let (mut ws, _) = tokio_tungstenite::connect_async(format!(
        "ws://127.0.0.1:{}/ws/chat?ticket={}",
        addr.port(),
        alice_ticket
    ))
    .await
    .unwrap();
    let sid = Uuid::new_v4().to_string();
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        json!({"message": "hello", "session_id": sid})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    while let Some(Ok(msg)) = ws.next().await {
        if let tokio_tungstenite::tungstenite::Message::Text(t) = msg {
            let ev: Value = serde_json::from_str(&t).unwrap();
            if ev.get("type") == Some(&json!("complete")) || ev.get("type") == Some(&json!("error"))
            {
                break;
            }
        }
    }
    ws.close(None).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Bob 尝试访问同一会话 → 应收到 Error，且原会话未被覆盖。
    let bob_ticket = test_state.issue_ticket(bob.id).await;
    let (mut ws2, _) = tokio_tungstenite::connect_async(format!(
        "ws://127.0.0.1:{}/ws/chat?ticket={}",
        addr.port(),
        bob_ticket
    ))
    .await
    .unwrap();
    ws2.send(tokio_tungstenite::tungstenite::Message::Text(
        json!({"message": "hi", "session_id": sid})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    let mut saw_error = false;
    while let Some(Ok(msg)) = ws2.next().await {
        if let tokio_tungstenite::tungstenite::Message::Text(t) = msg {
            let ev: Value = serde_json::from_str(&t).unwrap();
            if ev.get("type") == Some(&json!("error")) {
                saw_error = true;
                break;
            }
            if ev.get("type") == Some(&json!("complete")) {
                break;
            }
        }
    }
    assert!(saw_error, "跨用户访问应返回 Error");

    let sessions = test_state.sessions.read().await;
    let parsed_sid = Uuid::parse_str(&sid).unwrap();
    let data = sessions.get(&parsed_sid).expect("alice 的会话应仍存在");
    assert_eq!(data.user_id, alice.id);
    assert!(!data.session.messages().is_empty());
}

// ============ WebSocket 生命周期（心跳 / 断连 abort） ============

/// 可控 MockClient：`chat` 阻塞直到被通知，可观测是否被 abort。
struct BlockingMockClient {
    notify: Arc<Notify>,
    started: Arc<AtomicBool>,
    completed: Arc<AtomicBool>,
}

impl BlockingMockClient {
    fn new() -> (Self, Arc<Notify>, Arc<AtomicBool>, Arc<AtomicBool>) {
        let notify = Arc::new(Notify::new());
        let started = Arc::new(AtomicBool::new(false));
        let completed = Arc::new(AtomicBool::new(false));
        (
            Self {
                notify: notify.clone(),
                started: started.clone(),
                completed: completed.clone(),
            },
            notify,
            started,
            completed,
        )
    }
}

#[async_trait]
impl LlmClient for BlockingMockClient {
    async fn chat(&self, _req: ChatRequest) -> anyhow::Result<BoxStream<'static, Event>> {
        self.started.store(true, Ordering::SeqCst);
        self.notify.notified().await;
        self.completed.store(true, Ordering::SeqCst);
        Ok(Box::pin(futures::stream::iter(vec![Event::Done])))
    }
}

async fn wait_for_flag(flag: &AtomicBool, timeout: Duration) {
    let deadline = time::Instant::now() + timeout;
    while !flag.load(Ordering::SeqCst) {
        time::sleep(Duration::from_millis(10)).await;
        assert!(time::Instant::now() < deadline, "flag not set in time");
    }
}

#[tokio::test]
async fn ws_heartbeat_sends_ping_and_accepts_pong() {
    let (client, notify, started, completed) = BlockingMockClient::new();
    let dir = tempdir().unwrap();
    let (sandbox, specs) = forgeclaw_server::default_sandbox_with_specs(dir.path().to_path_buf());
    let prompts_root =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../prompts/profiles");
    let (state, _dir) = build_state_with_llm(Arc::new(client), dir, sandbox, specs, prompts_root);
    std::env::set_var("FORGECLAW_WS_HEARTBEAT_SECS", "1");
    std::env::set_var("FORGECLAW_WS_PONG_TIMEOUT_SECS", "2");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let test_state = state.clone();
    tokio::spawn(async move {
        axum::serve(listener, app(state)).await.unwrap();
    });

    let ticket = test_state
        .issue_ticket(test_state.user_store.find_by_name("alice").unwrap().id)
        .await;
    let (mut ws, _) = tokio_tungstenite::connect_async(format!(
        "ws://127.0.0.1:{}/ws/chat?ticket={}",
        addr.port(),
        ticket
    ))
    .await
    .unwrap();

    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        json!({"message": "hello"}).to_string().into(),
    ))
    .await
    .unwrap();

    wait_for_flag(&started, Duration::from_secs(5)).await;

    // 等待 1.5 个心跳周期，服务端应发出 Ping；tungstenite 客户端会自动回 Pong。
    let mut saw_ping = false;
    let deadline = time::Instant::now() + Duration::from_secs(2);
    while time::Instant::now() < deadline {
        match time::timeout(Duration::from_millis(100), ws.next()).await {
            Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Ping(_)))) => {
                saw_ping = true;
                break;
            }
            Ok(Some(Ok(_))) => continue,
            _ => continue,
        }
    }
    assert!(
        saw_ping,
        "server should send Ping within heartbeat interval"
    );

    // 通知 LLM 结束，应收到 Complete。
    notify.notify_one();
    let mut saw_complete = false;
    let deadline = time::Instant::now() + Duration::from_secs(5);
    while time::Instant::now() < deadline {
        match time::timeout(Duration::from_millis(100), ws.next()).await {
            Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Text(t)))) => {
                let ev: Value = serde_json::from_str(&t).unwrap();
                if ev.get("type") == Some(&json!("complete")) {
                    saw_complete = true;
                    break;
                }
            }
            _ => continue,
        }
    }
    assert!(saw_complete, "should receive Complete after LLM finishes");
    assert!(
        completed.load(Ordering::SeqCst),
        "LLM should complete normally"
    );

    ws.close(None).await.unwrap();
}

#[tokio::test]
async fn ws_disconnect_aborts_llm_task() {
    let (client, _notify, started, completed) = BlockingMockClient::new();
    let dir = tempdir().unwrap();
    let (sandbox, specs) = forgeclaw_server::default_sandbox_with_specs(dir.path().to_path_buf());
    let prompts_root =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../prompts/profiles");
    let (state, _dir) = build_state_with_llm(Arc::new(client), dir, sandbox, specs, prompts_root);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let test_state = state.clone();
    tokio::spawn(async move {
        axum::serve(listener, app(state)).await.unwrap();
    });

    let ticket = test_state
        .issue_ticket(test_state.user_store.find_by_name("alice").unwrap().id)
        .await;
    let (mut ws, _) = tokio_tungstenite::connect_async(format!(
        "ws://127.0.0.1:{}/ws/chat?ticket={}",
        addr.port(),
        ticket
    ))
    .await
    .unwrap();

    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        json!({"message": "hello"}).to_string().into(),
    ))
    .await
    .unwrap();

    wait_for_flag(&started, Duration::from_secs(5)).await;

    // 客户端主动关闭，服务端应 abort LLM 任务。
    ws.close(None).await.unwrap();
    time::sleep(Duration::from_millis(300)).await;

    assert!(
        !completed.load(Ordering::SeqCst),
        "LLM task should be aborted, not complete normally"
    );
}
