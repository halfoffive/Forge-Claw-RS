//! HTTP 集成测试：用 `tower::ServiceExt::oneshot` 对 `app(state)` 发请求，
//! 验证 REST 路由返回正确 JSON。不打真实 LLM API（用 MockClient）。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use axum::body::{to_bytes, Body};
use axum::http::{HeaderValue, Request, StatusCode};
use forgeclaw_core::model::Session;
use forgeclaw_llm::{ChatRequest, Event, History, LlmClient, Role};
use forgeclaw_server::{app, AppState, Orchestrator, SessionData, UserStore};
use futures::stream::BoxStream;
use serde_json::{json, Value};
use tempfile::tempdir;
use tokio::sync::RwLock;
use tower::ServiceExt;
use uuid::Uuid;

/// 测试用有效 token（alice 用户）。
const TEST_TOKEN: &str = "alice-token";

/// 脚本化 Mock LLM 客户端：返回空 Done 流（HTTP 测试中 chat 不触发）。
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
    let user_store = UserStore::from_config(vec![("alice".into(), TEST_TOKEN.into())]);
    (
        AppState::new(
            Arc::new(orch),
            user_store,
            vec![
                "http://localhost:5173".to_string(),
                "http://localhost:8080".to_string(),
            ],
        ),
        dir,
    )
}

/// 返回 LLM Error 事件的 Mock 客户端，用于验证 500 响应体被统一。
struct ErrorMockClient;

#[async_trait]
impl LlmClient for ErrorMockClient {
    async fn chat(&self, _req: ChatRequest) -> anyhow::Result<BoxStream<'static, Event>> {
        Ok(Box::pin(futures::stream::iter(vec![Event::Error(
            "llm boom".into(),
        )])))
    }
}

fn build_state_with_error_client() -> (AppState, tempfile::TempDir) {
    let dir = tempdir().unwrap();
    let (sandbox, specs) = forgeclaw_server::default_sandbox_with_specs(dir.path().to_path_buf());
    let prompts_root =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../prompts/profiles");
    let llm: Arc<dyn LlmClient> = Arc::new(ErrorMockClient);
    let orch = Orchestrator::new(
        llm,
        Arc::new(sandbox),
        specs,
        prompts_root,
        "default".into(),
        "deepseek-chat".into(),
        dir.path().to_path_buf(),
    );
    let user_store = UserStore::from_config(vec![("alice".into(), TEST_TOKEN.into())]);
    (
        AppState::new(
            Arc::new(orch),
            user_store,
            vec![
                "http://localhost:5173".to_string(),
                "http://localhost:8080".to_string(),
            ],
        ),
        dir,
    )
}

async fn body_to_json(body: Body) -> Value {
    let bytes = to_bytes(body, 1024 * 1024).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn get_api_tools_returns_tool_specs() {
    let (state, _dir) = build_state();
    let response = app(state)
        .oneshot(
            Request::builder()
                .uri("/api/tools")
                .header("authorization", format!("Bearer {TEST_TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let v = body_to_json(response.into_body()).await;
    let tools = v
        .get("tools")
        .and_then(|t| t.as_array())
        .expect("tools array");
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["shell", "read", "write", "search", "grep"]);
    // 抽查 shell 工具字段完整
    let shell = tools.iter().find(|t| t["name"] == "shell").unwrap();
    assert!(shell["description"].is_string());
    assert!(shell["parameters"].is_object());
}

#[tokio::test]
async fn post_api_prompts_compile_returns_prompt() {
    let (state, _dir) = build_state();
    let body = serde_json::to_vec(&json!({"profile":"default"})).unwrap();
    let response = app(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/prompts/compile")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {TEST_TOKEN}"))
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let v = body_to_json(response.into_body()).await;
    let prompt = v
        .get("prompt")
        .and_then(|p| p.as_str())
        .expect("prompt str");
    assert!(prompt.contains("## 身份与产品信息"));
    assert!(prompt.contains("deepseek-chat"));
    assert!(prompt.contains("read"));
}

#[tokio::test]
async fn post_api_prompts_compile_invalid_name_returns_404() {
    let (state, _dir) = build_state();
    let body = serde_json::to_vec(&json!({"profile":"../etc"})).unwrap();
    let response = app(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/prompts/compile")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {TEST_TOKEN}"))
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let bytes = to_bytes(response.into_body(), 1024).await.unwrap();
    assert_eq!(String::from_utf8(bytes.to_vec()).unwrap(), "profile not found");
}

#[tokio::test]
async fn get_api_sessions_empty_returns_empty_array() {
    let (state, _dir) = build_state();
    let response = app(state)
        .oneshot(
            Request::builder()
                .uri("/api/sessions")
                .header("authorization", format!("Bearer {TEST_TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let v = body_to_json(response.into_body()).await;
    let arr = v.as_array().expect("array");
    assert!(arr.is_empty());
}

#[tokio::test]
async fn get_api_sessions_unknown_id_returns_404() {
    let (state, _dir) = build_state();
    let response = app(state)
        .oneshot(
            Request::builder()
                .uri("/api/sessions/00000000-0000-0000-0000-000000000000")
                .header("authorization", format!("Bearer {TEST_TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn get_api_prompts_sections_returns_array() {
    let (state, _dir) = build_state();
    let response = app(state)
        .oneshot(
            Request::builder()
                .uri("/api/prompts/sections?profile=default")
                .header("authorization", format!("Bearer {TEST_TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let v = body_to_json(response.into_body()).await;
    let arr = v.as_array().expect("sections array");
    // default profile 含 identity/safety/tools/style 四章节
    assert_eq!(arr.len(), 4);
    let ids: Vec<&str> = arr.iter().map(|s| s["id"].as_str().unwrap()).collect();
    assert!(ids.contains(&"identity"));
    assert!(ids.contains(&"tools"));
}

#[tokio::test]
async fn get_api_prompts_sections_invalid_name_returns_404() {
    let (state, _dir) = build_state();
    let response = app(state)
        .oneshot(
            Request::builder()
                .uri("/api/prompts/sections?profile=../etc")
                .header("authorization", format!("Bearer {TEST_TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let bytes = to_bytes(response.into_body(), 1024).await.unwrap();
    assert_eq!(String::from_utf8(bytes.to_vec()).unwrap(), "profile not found");
}

#[tokio::test]
async fn parallel_chat_requests_same_session_preserve_history() {
    // 同一 session 的两个并发 /api/chat 请求不应因 read-clone-replace 覆盖而丢失消息。
    let (state, _dir) = build_state();
    let user = state.user_store.find_by_token(TEST_TOKEN).unwrap();
    let session_id = Uuid::new_v4();
    {
        let mut sessions = state.sessions.write().await;
        sessions.insert(
            session_id,
            SessionData {
                session: Session {
                    id: session_id,
                    created_at: chrono::Utc::now(),
                    messages: Vec::new(),
                },
                history: Arc::new(RwLock::new(History::new())),
                user_id: user.id,
            },
        );
    }

    let body1 =
        serde_json::to_vec(&json!({"message":"msg1","session_id":session_id.to_string()})).unwrap();
    let body2 =
        serde_json::to_vec(&json!({"message":"msg2","session_id":session_id.to_string()})).unwrap();

    let app = app(state.clone());
    let fut1 = app.clone().oneshot(
        Request::builder()
            .method("POST")
            .uri("/api/chat")
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {TEST_TOKEN}"))
            .body(Body::from(body1))
            .unwrap(),
    );
    let fut2 = app.clone().oneshot(
        Request::builder()
            .method("POST")
            .uri("/api/chat")
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {TEST_TOKEN}"))
            .body(Body::from(body2))
            .unwrap(),
    );

    let (r1, r2) = tokio::join!(fut1, fut2);
    assert_eq!(r1.unwrap().status(), StatusCode::OK);
    assert_eq!(r2.unwrap().status(), StatusCode::OK);

    let sessions = state.sessions.read().await;
    let data = sessions.get(&session_id).expect("session exists");
    let history = data.history.read().await;
    let user_msgs: Vec<_> = history
        .messages()
        .iter()
        .filter(|m| m.role == Role::User)
        .collect();
    let assistant_msgs: Vec<_> = history
        .messages()
        .iter()
        .filter(|m| m.role == Role::Assistant)
        .collect();
    assert_eq!(user_msgs.len(), 2, "both user messages should be preserved");
    assert_eq!(
        assistant_msgs.len(),
        2,
        "both assistant messages should be preserved"
    );
}

#[tokio::test]
async fn post_oversized_body_returns_413() {
    let (state, _dir) = build_state();
    let padding = "x".repeat(1024 * 1024 + 1);
    let body = serde_json::to_vec(&json!({"profile":"default","pad":padding})).unwrap();

    let response = app(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/prompts/compile")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {TEST_TOKEN}"))
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn cors_preflight_allows_whitelisted_origin() {
    let (state, _dir) = build_state();
    let response = app(state)
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/api/sessions")
                .header("origin", "http://localhost:8080")
                .header("access-control-request-method", "GET")
                .header("access-control-request-headers", "authorization")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get("access-control-allow-origin"),
        Some(&HeaderValue::from_static("http://localhost:8080"))
    );
}

#[tokio::test]
async fn cors_preflight_rejects_non_whitelisted_origin() {
    let (state, _dir) = build_state();
    let response = app(state)
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/api/sessions")
                .header("origin", "http://evil.com")
                .header("access-control-request-method", "GET")
                .header("access-control-request-headers", "authorization")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(
        response
            .headers()
            .get("access-control-allow-origin")
            .is_none(),
        "非白名单来源不应返回 CORS 响应头"
    );
}

#[tokio::test]
async fn chat_handler_returns_generic_500_on_orchestrator_error() {
    // Orchestrator 返回 Error 事件时，api.rs 应统一返回 500 + "internal server error"。
    let (state, _dir) = build_state_with_error_client();
    let body = serde_json::to_vec(&json!({"message":"hello"})).unwrap();
    let response = app(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/chat")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {TEST_TOKEN}"))
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let bytes = to_bytes(response.into_body(), 1024).await.unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    assert_eq!(text, "internal server error");
}
