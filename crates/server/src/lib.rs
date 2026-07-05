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

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::extract::{ConnectInfo, DefaultBodyLimit, Path};
use axum::http::{header, HeaderValue, Method, Request, StatusCode};
use axum::middleware::{from_fn, from_fn_with_state, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use forgeclaw_llm::OpenAiClient;
use rust_embed::RustEmbed;
use tower_governor::errors::GovernorError;
use tower_governor::governor::GovernorConfigBuilder;
use tower_governor::key_extractor::KeyExtractor;
use tower_governor::GovernorLayer;
use tower_http::{
    compression::CompressionLayer,
    cors::{AllowOrigin, CorsLayer},
    timeout::TimeoutLayer,
    trace::{MakeSpan, TraceLayer},
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

/// 内嵌的 WebUI 静态资源。
#[derive(RustEmbed)]
#[folder = "../../web/dist"]
struct Asset;

/// 将内嵌资源作为 axum handler 提供。
async fn static_handler(Path(path): Path<String>) -> impl IntoResponse {
    let path = path.trim_start_matches('/');
    match Asset::get(path) {
        Some(content) => Response::builder()
            .header(header::CONTENT_TYPE, mime_for_path(path))
            .body(Body::from(content.data))
            .unwrap(),
        None => match Asset::get("index.html") {
            // SPA fallback：任何未命中资源都返回 index.html，让 Vue Router 处理。
            Some(content) => Response::builder()
                .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                .body(Body::from(content.data))
                .unwrap(),
            None => (StatusCode::NOT_FOUND, "embedded index.html not found").into_response(),
        },
    }
}

/// 自定义 span 构造器：URI 只记录 path，不记录 query string（避免 ticket/token 泄漏到日志）。
#[derive(Clone)]
struct SanitizedMakeSpan;

impl<B> MakeSpan<B> for SanitizedMakeSpan {
    fn make_span(&mut self, request: &axum::http::Request<B>) -> tracing::Span {
        tracing::info_span!(
            "request",
            method = %request.method(),
            uri = %request.uri().path(),
            version = ?request.version(),
        )
    }
}

fn mime_for_path(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") | Some("htm") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "text/javascript",
        Some("css") => "text/css",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("ico") => "image/x-icon",
        Some("json") => "application/json",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}

// ============ Login 限流（IP + name，5 次/分钟） ============

/// 登录限流 key。
#[derive(Clone, Debug, Hash, Eq, PartialEq)]
struct LoginRateLimitKey {
    ip: IpAddr,
    name: String,
}

/// 由前置中间件从请求体解析出的 `name`，供限流 key 提取器使用。
#[derive(Clone, Debug)]
struct LoginName(String);

#[derive(Clone, Copy, Debug)]
struct LoginRateLimitKeyExtractor;

impl KeyExtractor for LoginRateLimitKeyExtractor {
    type Key = LoginRateLimitKey;

    fn extract<T>(&self, req: &Request<T>) -> Result<Self::Key, GovernorError> {
        let ip = req
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ci| ci.0.ip())
            .unwrap_or_else(|| IpAddr::from([127, 0, 0, 1]));
        let name = req
            .extensions()
            .get::<LoginName>()
            .map(|n| n.0.clone())
            .unwrap_or_default();
        Ok(LoginRateLimitKey { ip, name })
    }
}

/// 在请求进入限流器前从 JSON body 中解析 `name`，并把 body 原样传回后续处理。
async fn login_rate_limit_key_middleware(mut req: Request<Body>, next: Next) -> Response {
    let name = if req.uri().path() == "/api/auth/login" && req.method() == axum::http::Method::POST
    {
        let (parts, body) = req.into_parts();
        let bytes = to_bytes(body, 4096).await.unwrap_or_default();
        let name = serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .and_then(|v| v.get("name").and_then(|n| n.as_str().map(String::from)))
            .unwrap_or_default();
        req = Request::from_parts(parts, Body::from(bytes));
        name
    } else {
        String::new()
    };
    req.extensions_mut().insert(LoginName(name));
    next.run(req).await
}

/// 装配 axum Router：REST + WebSocket + 内嵌 WebUI + tower-http 中间件 + auth 中间件。
///
/// 路由组织：
/// - 需鉴权的 `/api/*` 路由挂 [`auth::auth_middleware`]（Bearer token）。
/// - `/api/auth/login` 不挂中间件（登录获取 token）。
/// - `/ws/chat` 不挂 header 中间件（浏览器 WS 无法设 Authorization），
///   由 [`ws::ws_chat_handler`] 自行从 query 参数消费一次性 ticket。
pub fn app(state: AppState) -> Router {
    let allowed_origins: Vec<HeaderValue> = state.allowed_origins.to_vec();
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::list(allowed_origins))
        .allow_methods([Method::GET, Method::POST])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE]);

    let protected = Router::<AppState>::new()
        .route("/api/chat", post(api::chat_handler))
        .route("/api/sessions", get(api::list_sessions))
        .route("/api/sessions/{id}", get(api::get_session))
        .route("/api/tools", get(api::list_tools))
        .route("/api/prompts/compile", post(api::compile_prompt))
        .route("/api/prompts/sections", get(api::list_sections))
        .layer(from_fn_with_state(state.clone(), auth::auth_middleware));

    let login_governor = GovernorConfigBuilder::default()
        .key_extractor(LoginRateLimitKeyExtractor)
        .period(Duration::from_secs(60))
        .burst_size(5)
        .finish()
        .expect("valid login governor config");
    let login_routes = Router::<AppState>::new()
        .route("/api/auth/login", post(auth::login_handler))
        .layer(GovernorLayer::new(Arc::new(login_governor)))
        .layer(from_fn(login_rate_limit_key_middleware));

    Router::<AppState>::new()
        .merge(protected)
        .merge(login_routes)
        .route("/ws/chat", get(ws::ws_chat_handler))
        .route("/{*path}", get(static_handler))
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::GATEWAY_TIMEOUT,
            Duration::from_secs(120),
        ))
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http().make_span_with(SanitizedMakeSpan))
        .layer(cors)
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
