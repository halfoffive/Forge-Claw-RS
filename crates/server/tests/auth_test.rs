//! 鉴权集成测试：覆盖中间件 401、login 端点、用户会话隔离、WS query token 鉴权、WS 消息流。
//!
//! 用 `tower::ServiceExt::oneshot` 发请求，不打真实 LLM API（MockClient 返回空 Done）。

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::body::{to_bytes, Body};
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode};
use forgeclaw_core::model::Session;
use forgeclaw_llm::{ChatRequest, Event, History, LlmClient};
use forgeclaw_server::{app, AppState, Orchestrator, SessionData, User, UserStore};
use futures::stream::BoxStream;
use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tempfile::tempdir;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tower::ServiceExt;
use uuid::Uuid;

const ALICE_TOKEN: &str = "alice-token";
const BOB_TOKEN: &str = "bob-token";

/// 脚本化 Mock LLM 客户端：每次 `chat` 返回同一组事件。
struct MockClient {
    events: Vec<Event>,
    counter: AtomicUsize,
}

impl MockClient {
    fn new(events: Vec<Event>) -> Self {
        Self {
            events,
            counter: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl LlmClient for MockClient {
    async fn chat(&self, _req: ChatRequest) -> anyhow::Result<BoxStream<'static, Event>> {
        self.counter.fetch_add(1, Ordering::SeqCst);
        Ok(Box::pin(futures::stream::iter(self.events.clone())))
    }
}

/// 构造带两个用户（alice/bob）的 AppState。
fn build_state_with_llm(llm: Arc<dyn LlmClient>) -> (AppState, tempfile::TempDir) {
    let dir = tempdir().unwrap();
    let (sandbox, specs) = forgeclaw_server::default_sandbox_with_specs(dir.path().to_path_buf());
    let prompts_root =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../prompts/profiles");
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
    (
        AppState::new(
            Arc::new(orch),
            user_store,
            vec!["http://localhost:5173".to_string()],
        ),
        dir,
    )
}

fn build_state() -> (AppState, tempfile::TempDir) {
    build_state_with_llm(Arc::new(MockClient::new(vec![Event::Done])))
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
                .extension(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 8080))))
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let v = body_to_json(response.into_body()).await;
    assert_eq!(v["ok"], true);
    assert_eq!(v["user"]["name"], "alice");
    // SRV-024：token 不在响应体中
    assert!(v["user"].get("token").is_none());
    assert!(v["user"]["id"].is_string());
    // SRV-002：login 签发一次性 WS ticket
    assert!(v.get("ticket").and_then(|t| t.as_str()).is_some());
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
                .extension(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 8080))))
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
                .extension(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 8080))))
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
                .extension(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 8080))))
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
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
            session: Session {
                id: sid,
                created_at: chrono::Utc::now(),
                messages: Vec::new(),
            },
            history: Arc::new(tokio::sync::RwLock::new(History::new())),
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

// ============ WebSocket 一次性 ticket 鉴权（升级前 401） ============

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
        .oneshot(ws_request("/ws/chat?ticket=wrong-ticket"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn ws_with_valid_ticket_passes_auth() {
    let (state, _dir) = build_state();
    let alice = state.user_store.find_by_name("alice").expect("user exists");
    let ticket = state.issue_ticket(alice.id);
    let response = app(state)
        .oneshot(ws_request(&format!("/ws/chat?ticket={ticket}")))
        .await
        .unwrap();
    // 有效 ticket 通过鉴权后，oneshot 无真实 hyper 连接 → WebSocketUpgrade 返回 426
    // （ConnectionNotUpgradable）。关键：不是 401，证明 ticket 校验已通过。
    assert_ne!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(response.status(), StatusCode::UPGRADE_REQUIRED);
}

// ============ WebSocket 真实消息流集成测试 ============

type TestWs =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn start_server(state: AppState) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(
            listener,
            app(state).into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    (addr, handle)
}

async fn connect_ws(addr: SocketAddr, ticket: &str) -> TestWs {
    let url = format!("ws://127.0.0.1:{}/ws/chat?ticket={}", addr.port(), ticket);
    let (ws, _) = connect_async(url).await.unwrap();
    ws
}

async fn recv_text(ws: &mut TestWs) -> String {
    loop {
        match ws.next().await.unwrap().unwrap() {
            WsMessage::Text(text) => return text.to_string(),
            WsMessage::Ping(_) => ws.send(WsMessage::Pong(vec![].into())).await.unwrap(),
            other => panic!("unexpected ws message: {:?}", other),
        }
    }
}

#[tokio::test]
async fn ws_invalid_session_id_returns_error_frame() {
    let (state, _dir) = build_state();
    let alice = state.user_store.find_by_name("alice").expect("user exists");
    let ticket = state.issue_ticket(alice.id);
    let (addr, handle) = start_server(state).await;

    let mut ws = connect_ws(addr, &ticket).await;
    ws.send(WsMessage::Text(
        r#"{"message":"hi","session_id":"not-a-uuid"}"#.into(),
    ))
    .await
    .unwrap();

    let text = recv_text(&mut ws).await;
    let ev: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(ev["type"], "error");
    assert_eq!(ev["message"], "invalid session_id");

    let _ = ws.close(None).await;
    handle.abort();
}

#[tokio::test]
async fn ws_orchestrator_error_returns_generic_error_frame() {
    // P1-C-011/E-008：上游 Error 事件不应直接透传给前端。
    let llm: Arc<dyn LlmClient> = Arc::new(MockClient::new(vec![Event::Error(
        "upstream LLM failed: https://api.example.com 401 Unauthorized".into(),
    )]));
    let (state, _dir) = build_state_with_llm(llm);
    let alice = state.user_store.find_by_name("alice").expect("user exists");
    let ticket = state.issue_ticket(alice.id);
    let (addr, handle) = start_server(state).await;

    let mut ws = connect_ws(addr, &ticket).await;
    ws.send(WsMessage::Text(r#"{"message":"hi"}"#.into()))
        .await
        .unwrap();

    let text = recv_text(&mut ws).await;
    let ev: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(ev["type"], "error");
    assert_eq!(ev["message"], "internal error");
    let msg = ev["message"].as_str().unwrap();
    assert!(!msg.contains("api.example.com"), "不应泄漏上游 URL");
    assert!(!msg.contains("401"), "不应泄漏上游状态码");

    let _ = ws.close(None).await;
    handle.abort();
}

#[tokio::test]
async fn ws_timeout_aborts_task_and_returns_error_frame() {
    // 让 LLM 永远挂起，验证任务超时后收到 Error 帧且不会无限阻塞。
    struct SleepClient;

    #[async_trait]
    impl LlmClient for SleepClient {
        async fn chat(&self, _req: ChatRequest) -> anyhow::Result<BoxStream<'static, Event>> {
            Ok(Box::pin(futures::stream::once(async {
                tokio::time::sleep(Duration::from_secs(600)).await;
                Event::Done
            })))
        }
    }

    std::env::set_var("FORGECLAW_WS_TASK_TIMEOUT_SECS", "1");
    let (state, _dir) = build_state_with_llm(Arc::new(SleepClient));
    let alice = state.user_store.find_by_name("alice").expect("user exists");
    let ticket = state.issue_ticket(alice.id);
    let (addr, handle) = start_server(state).await;

    let mut ws = connect_ws(addr, &ticket).await;
    ws.send(WsMessage::Text(r#"{"message":"hi"}"#.into()))
        .await
        .unwrap();

    let text = tokio::time::timeout(Duration::from_secs(5), recv_text(&mut ws))
        .await
        .expect("should receive error frame before 5s");
    let ev: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(ev["type"], "error");
    assert_eq!(ev["message"], "orchestrator task timed out");

    let _ = ws.close(None).await;
    handle.abort();
    std::env::remove_var("FORGECLAW_WS_TASK_TIMEOUT_SECS");
}

#[tokio::test]
async fn ws_cross_user_session_does_not_pollute() {
    // P1-D-015/E-006/C-009：bob 用 alice 的 session_id 发 WS 消息，应收到 error 帧且 alice 会话不被污染。
    let (state, _dir) = build_state();
    let alice_sid = insert_session_for(&state, "alice").await;

    let bob = state.user_store.find_by_name("bob").expect("user exists");
    let ticket = state.issue_ticket(bob.id);
    let (addr, handle) = start_server(state.clone()).await;

    let mut ws = connect_ws(addr, &ticket).await;
    ws.send(WsMessage::Text(
        format!(r#"{{"message":"bob hi","session_id":"{}"}}"#, alice_sid).into(),
    ))
    .await
    .unwrap();

    let text = recv_text(&mut ws).await;
    let ev: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(ev["type"], "error");

    let sessions = state.sessions.read().await;
    let data = sessions.get(&alice_sid).expect("alice session exists");
    assert!(
        data.session.messages.is_empty(),
        "alice 的会话不应被 bob 写回污染"
    );

    let _ = ws.close(None).await;
    handle.abort();
}

#[tokio::test]
async fn ws_timeout_does_not_block_subsequent_same_session_request() {
    // P1-D-002/D-003/E-007/C-010：超时后同 session 的后续请求不应因未释放锁而阻塞。
    struct SleepThenDoneClient {
        counter: AtomicUsize,
    }

    #[async_trait]
    impl LlmClient for SleepThenDoneClient {
        async fn chat(&self, _req: ChatRequest) -> anyhow::Result<BoxStream<'static, Event>> {
            let n = self.counter.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                Ok(Box::pin(futures::stream::once(async {
                    tokio::time::sleep(Duration::from_secs(600)).await;
                    Event::Done
                })))
            } else {
                Ok(Box::pin(futures::stream::iter(vec![Event::Done])))
            }
        }
    }

    std::env::set_var("FORGECLAW_WS_TASK_TIMEOUT_SECS", "1");
    let (state, _dir) = build_state_with_llm(Arc::new(SleepThenDoneClient {
        counter: AtomicUsize::new(0),
    }));
    let alice = state.user_store.find_by_name("alice").expect("user exists");
    let ticket = state.issue_ticket(alice.id);
    let (addr, handle) = start_server(state).await;

    let mut ws = connect_ws(addr, &ticket).await;
    let session_id = Uuid::new_v4();

    ws.send(WsMessage::Text(
        format!(r#"{{"message":"first","session_id":"{}"}}"#, session_id).into(),
    ))
    .await
    .unwrap();
    let text = tokio::time::timeout(Duration::from_secs(5), recv_text(&mut ws))
        .await
        .expect("should receive timeout error frame before 5s");
    let ev: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(ev["type"], "error");
    assert_eq!(ev["message"], "orchestrator task timed out");

    ws.send(WsMessage::Text(
        format!(r#"{{"message":"second","session_id":"{}"}}"#, session_id).into(),
    ))
    .await
    .unwrap();
    let text = tokio::time::timeout(Duration::from_secs(5), recv_text(&mut ws))
        .await
        .expect("subsequent request should not be blocked");
    let ev: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(ev["type"], "complete");

    let _ = ws.close(None).await;
    handle.abort();
    std::env::remove_var("FORGECLAW_WS_TASK_TIMEOUT_SECS");
}

// ============ 安全侧信道测试 ============

#[test]
fn find_by_token_returns_correct_user_or_none() {
    let store = UserStore::from_config(vec![
        ("alice".into(), ALICE_TOKEN.into()),
        ("bob".into(), BOB_TOKEN.into()),
    ]);
    assert_eq!(
        store.find_by_token(ALICE_TOKEN).map(|u| u.name),
        Some("alice".to_string())
    );
    assert_eq!(
        store.find_by_token(BOB_TOKEN).map(|u| u.name),
        Some("bob".to_string())
    );
    assert!(store.find_by_token("not-a-token").is_none());
}

#[test]
fn find_by_token_timing_is_independent_of_token_existence() {
    // C-006：构造足够多的用户，使遍历成本显著，便于检测是否存在早退。
    let mut pairs = Vec::new();
    for i in 0..100 {
        pairs.push((format!("user-{i}"), format!("token-{i:0>3}")));
    }
    let store = UserStore::from_config(pairs);
    let valid_token = "token-050";
    let invalid_token = "token-999";

    let iterations = 500;

    let start = std::time::Instant::now();
    for _ in 0..iterations {
        let _ = store.find_by_token(valid_token);
    }
    let valid_duration = start.elapsed();

    let start = std::time::Instant::now();
    for _ in 0..iterations {
        let _ = store.find_by_token(invalid_token);
    }
    let invalid_duration = start.elapsed();

    let ratio = valid_duration.as_secs_f64() / invalid_duration.as_secs_f64().max(1e-9);
    assert!(
        ratio >= 0.25 && ratio <= 4.0,
        "valid/invalid lookup time ratio {ratio} is outside expected range; \
         valid={valid_duration:?}, invalid={invalid_duration:?}"
    );
}

#[test]
fn user_debug_does_not_leak_token() {
    // C-007：Debug 输出中 token 字段应被脱敏，不泄漏真实值。
    let user = User {
        id: Uuid::new_v4(),
        name: "alice".into(),
        token: "super-secret-token-12345".into(),
    };
    let debug = format!("{:?}", user);
    assert!(
        !debug.contains("super-secret-token-12345"),
        "Debug output leaked token: {debug}"
    );
    assert!(
        debug.contains("alice"),
        "Debug output should contain name: {debug}"
    );
    assert!(
        debug.contains("<redacted>"),
        "token field should be redacted: {debug}"
    );
}
