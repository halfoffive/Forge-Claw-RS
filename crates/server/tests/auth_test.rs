//! 鉴权集成测试：覆盖中间件 401、login 端点、用户会话隔离、WS query token 鉴权。
//!
//! 用 `tower::ServiceExt::oneshot` 发请求，不打真实 LLM API（MockClient 返回空 Done）。

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use axum::body::{to_bytes, Body};
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode};
use forgeclaw_core::model::Session;
use forgeclaw_llm::{ChatRequest, Event, History, LlmClient};
use forgeclaw_server::{app, AppState, Orchestrator, SessionData, UserStore};
use futures::stream::BoxStream;
use serde_json::{json, Value};
use tempfile::tempdir;
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
