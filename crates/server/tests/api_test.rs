//! HTTP 集成测试：用 `tower::ServiceExt::oneshot` 对 `app(state)` 发请求，
//! 验证 REST 路由返回正确 JSON。不打真实 LLM API（用 MockClient）。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use forgeclaw_llm::{ChatRequest, Event, LlmClient};
use forgeclaw_server::{app, AppState, Orchestrator, UserStore};
use futures::stream::BoxStream;
use serde_json::{json, Value};
use tempfile::tempdir;
use tower::ServiceExt;

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
            vec!["http://localhost:5173".to_string()],
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
