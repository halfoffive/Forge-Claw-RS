//! forgeclaw-server：Agent 编排器 + axum REST + WebSocket + tower-http 中间件。
//!
//! 模块组织：
//! - [`orchestrator`]：消息循环 / 子代理 / 事件流
//! - [`api`]：REST 路由 + `AppState`
//! - [`ws`]：WebSocket `/ws/chat`
//! - [`auth`]：多用户鉴权（User/UserStore/中间件/login/ticket）
//!
//! 装配入口：[`app`] 组装路由 + tower-http 中间件 + auth 中间件；[`run`] 监听并服务；
//! [`build_orchestrator`] 从配置构造 [`Orchestrator`]。

pub mod api;
pub mod auth;
pub mod orchestrator;
pub mod ws;

pub use api::{AppState, SessionData};
pub use auth::{User, UserPublic, UserStore};
pub use orchestrator::{
    default_sandbox_with_specs, restricted_sandbox_with_specs, Orchestrator, OrchestratorEvent,
    SubagentRole, ToolCallRecord,
};

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::DefaultBodyLimit;
use axum::http::{header, HeaderValue, Method, Request as HttpRequest};
use axum::middleware::from_fn_with_state;
use axum::routing::{get, post};
use axum::Router;
use forgeclaw_llm::OpenAiClient;
use tower_governor::governor::GovernorConfigBuilder;
use tower_governor::GovernorLayer;
use tower_http::{
    compression::CompressionLayer, cors::AllowOrigin, cors::CorsLayer, timeout::TimeoutLayer,
    trace::TraceLayer,
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

/// 构造 CORS 白名单层（SRV-001）：仅允许 `AppState.allowed_origins`，
/// methods=[GET,POST]，headers=[AUTHORIZATION,CONTENT_TYPE]。
fn build_cors_layer(state: &AppState) -> CorsLayer {
    let origins: Vec<HeaderValue> = state
        .allowed_origins
        .iter()
        .filter_map(|o| o.parse().ok())
        .collect();
    CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods([Method::GET, Method::POST])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
}

/// 装配 axum Router：REST + WebSocket + tower-http 中间件 + auth 中间件。
///
/// 路由组织：
/// - 需鉴权的 `/api/*` 路由挂 [`auth::auth_middleware`]（Bearer token），含 `/api/auth/ticket`（SRV-002）。
/// - `/api/auth/login` 不挂 header 中间件，但套 `tower_governor` 限流（5次/60s，key=IP，SRV-008）。
/// - `/ws/chat` 不挂 header 中间件（浏览器 WS 无法设 Authorization），
///   由 [`ws::ws_chat_handler`] 自行从 `?ticket=` query 参数核销一次性 ticket 鉴权（SRV-002）。
///
/// 中间件顺序（SRV-016，外→内）：TraceLayer → CorsLayer → CompressionLayer →
/// TimeoutLayer → DefaultBodyLimit（1MB，SRV-022）。TraceLayer 的 span 仅记录 path，
/// 脱敏 query（SRV-002）。
pub fn app(state: AppState) -> Router {
    let protected = Router::<AppState>::new()
        .route("/api/chat", post(api::chat_handler))
        .route("/api/sessions", get(api::list_sessions))
        .route("/api/sessions/{id}", get(api::get_session))
        .route("/api/tools", get(api::list_tools))
        .route("/api/prompts/compile", post(api::compile_prompt))
        .route("/api/prompts/sections", get(api::list_sections))
        .route("/api/auth/ticket", get(auth::ticket_handler))
        .layer(from_fn_with_state(state.clone(), auth::auth_middleware));

    // login 限流：5 次/60s，key=PeerIP（SRV-008）。
    let governor_config = GovernorConfigBuilder::default()
        .per_second(60)
        .burst_size(5)
        .finish()
        .expect("valid governor config");
    let login = Router::<AppState>::new()
        .route("/api/auth/login", post(auth::login_handler))
        .layer(GovernorLayer::new(governor_config));

    Router::<AppState>::new()
        .merge(protected)
        .merge(login)
        .route("/ws/chat", get(ws::ws_chat_handler))
        // 内→外依次添加：DefaultBodyLimit → TimeoutLayer → CompressionLayer → CorsLayer → TraceLayer
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .layer(TimeoutLayer::with_status_code(
            axum::http::StatusCode::GATEWAY_TIMEOUT,
            Duration::from_secs(300),
        ))
        .layer(CompressionLayer::new())
        .layer(build_cors_layer(&state))
        .layer(
            // SRV-002：span 只记录 path，脱敏 query。
            TraceLayer::new_for_http().make_span_with(
                |req: &HttpRequest<Body>| tracing::info_span!("http", path = %req.uri().path()),
            ),
        )
        .with_state(state)
}

/// 监听 `addr` 并服务。用 `into_make_service_with_connect_info` 注入 peer IP，
/// 供 `tower_governor` 的 `PeerIpKeyExtractor` 限流使用（SRV-008）。
pub async fn run(addr: SocketAddr, state: AppState) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(
        listener,
        app(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
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
