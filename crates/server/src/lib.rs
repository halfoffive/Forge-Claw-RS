//! forgeclaw-server：Agent 编排器 + axum REST + WebSocket + tower-http 中间件。
//!
//! 模块组织：
//! - [`orchestrator`]：消息循环 / 子代理 / 事件流
//! - [`api`]：REST 路由 + `AppState`
//! - [`ws`]：WebSocket `/ws/chat`
//! - [`auth`]：多用户鉴权（User/UserStore/中间件/login）
//!
//! 装配入口：[`app`] 组装路由 + tower-http 中间件 + auth 中间件；[`run`] 监听并服务；
//! [`build_orchestrator`] 从配置构造 [`Orchestrator`]。

pub mod api;
pub mod auth;
pub mod orchestrator;
pub mod ws;

pub use api::{AppState, SessionData};
pub use auth::{User, UserStore};
pub use orchestrator::{
    default_sandbox_with_specs, restricted_sandbox_with_specs, Orchestrator, OrchestratorEvent,
    SubagentRole, ToolCallRecord,
};

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use axum::middleware::from_fn_with_state;
use axum::routing::{get, post};
use axum::Router;
use forgeclaw_llm::OpenAiClient;
use tower_http::{
    compression::CompressionLayer, cors::CorsLayer, timeout::TimeoutLayer, trace::TraceLayer,
};

/// Orchestrator 构造配置。
pub struct OrchestratorConfig {
    pub base_url: String,
    pub api_key: String,
    pub prompts_root: PathBuf,
    pub working_dir: PathBuf,
    pub model: String,
    pub profile: String,
}

/// 装配 axum Router：REST + WebSocket + tower-http 中间件 + auth 中间件。
///
/// 路由组织：
/// - 需鉴权的 `/api/*` 路由挂 [`auth::auth_middleware`]（Bearer token）。
/// - `/api/auth/login` 不挂中间件（登录获取 token）。
/// - `/ws/chat` 不挂 header 中间件（浏览器 WS 无法设 Authorization），
///   由 [`ws::ws_chat_handler`] 自行从 `?token=` query 参数鉴权。
pub fn app(state: AppState) -> Router {
    let protected = Router::<AppState>::new()
        .route("/api/chat", post(api::chat_handler))
        .route("/api/sessions", get(api::list_sessions))
        .route("/api/sessions/{id}", get(api::get_session))
        .route("/api/tools", get(api::list_tools))
        .route("/api/prompts/compile", post(api::compile_prompt))
        .route("/api/prompts/sections", get(api::list_sections))
        .layer(from_fn_with_state(state.clone(), auth::auth_middleware));

    Router::<AppState>::new()
        .merge(protected)
        .route("/api/auth/login", post(auth::login_handler))
        .route("/ws/chat", get(ws::ws_chat_handler))
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
        .layer(CompressionLayer::new())
        .layer(TimeoutLayer::with_status_code(
            StatusCode::GATEWAY_TIMEOUT,
            Duration::from_secs(300),
        ))
        .with_state(state)
}

/// 监听 `addr` 并服务。
pub async fn run(addr: SocketAddr, state: AppState) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app(state)).await?;
    Ok(())
}

/// 工厂：从配置构造 `Arc<Orchestrator>`（OpenAiClient + 默认沙箱 + PromptEngine）。
pub fn build_orchestrator(config: OrchestratorConfig) -> anyhow::Result<Arc<Orchestrator>> {
    let llm: Arc<dyn forgeclaw_llm::LlmClient> =
        Arc::new(OpenAiClient::new(config.base_url, config.api_key)?);
    let (sandbox, tool_specs) = default_sandbox_with_specs(config.working_dir.clone());
    let orch = Orchestrator::new(
        llm,
        Arc::new(sandbox),
        tool_specs,
        config.prompts_root,
        config.profile,
        config.model,
        config.working_dir,
    );
    Ok(Arc::new(orch))
}
