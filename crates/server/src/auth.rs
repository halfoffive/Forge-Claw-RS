//! 多用户鉴权：User/UserStore 模型、Bearer token 中间件、AuthUser 提取器、login 端点。
//!
//! MVP 策略：静态预配置 token 比对（无 JWT/无密码哈希）。会话隔离通过
//! [`crate::api::SessionData::user_id`] 过滤实现，跨用户访问返回 404 不泄漏存在性。
//!
//! WS 一次性 ticket：浏览器 WS 无法设 Authorization header，login/ticket 端点签发
//! 短期 ticket（60s TTL，用后即焚），WS 升级时凭 `?ticket=` 鉴权。

use std::collections::HashMap;

use axum::extract::{FromRequestParts, Request, State};
use axum::http::{header, request::Parts, HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use uuid::Uuid;

use crate::api::AppState;

/// 用户模型。`token` 字段序列化时跳过，避免泄漏到响应体。
#[derive(Clone, Serialize)]
pub struct User {
    pub id: Uuid,
    pub name: String,
    #[serde(skip_serializing)]
    pub token: String,
}

impl std::fmt::Debug for User {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("User")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("token", &"[REDACTED]")
            .finish()
    }
}

/// 用户公开信息（响应体用，不含 token）。
#[derive(Clone, Debug, Serialize)]
pub struct UserPublic {
    pub id: Uuid,
    pub name: String,
}

impl From<&User> for UserPublic {
    fn from(u: &User) -> Self {
        Self {
            id: u.id,
            name: u.name.clone(),
        }
    }
}

/// 用户存储：按 token / name 双索引查找。放入 `Arc` 后置入 [`AppState`] 共享。
pub struct UserStore {
    by_token: HashMap<String, User>,
    by_name: HashMap<String, User>,
}

impl UserStore {
    pub fn new(users: Vec<User>) -> Self {
        let mut by_token = HashMap::new();
        let mut by_name = HashMap::new();
        for u in users {
            by_token.insert(u.token.clone(), u.clone());
            by_name.insert(u.name.clone(), u);
        }
        Self { by_token, by_name }
    }

    /// 工厂：从 (name, token) 对构造，UUID 自动生成。
    pub fn from_config(pairs: Vec<(String, String)>) -> Self {
        let users = pairs
            .into_iter()
            .map(|(name, token)| User {
                id: Uuid::new_v4(),
                name,
                token,
            })
            .collect();
        Self::new(users)
    }

    /// 从环境变量 `FORGECLAW_USERS=name:token,name2:token2` 构造。
    /// 未设置或为空则返回空存储（此时无任何用户可登录）。
    pub fn from_env() -> Self {
        let raw = std::env::var("FORGECLAW_USERS").unwrap_or_default();
        if raw.trim().is_empty() {
            return Self::new(Vec::new());
        }
        let pairs: Vec<(String, String)> = raw
            .split(',')
            .filter_map(|entry| {
                let (name, token) = entry.trim().split_once(':')?;
                let name = name.trim().to_string();
                let token = token.trim().to_string();
                if name.is_empty() || token.is_empty() {
                    return None;
                }
                Some((name, token))
            })
            .collect();
        Self::from_config(pairs)
    }

    pub fn find_by_token(&self, token: &str) -> Option<&User> {
        // 全量常量时间比较：避免 HashMap 早退带来的时序侧信道（SRV-018）。
        // 即使已命中仍继续遍历，保证「存在/不存在」路径耗时基本一致。
        let mut found = None;
        for user in self.by_token.values() {
            if constant_time_eq(&user.token, token) {
                found = Some(user);
            }
        }
        found
    }

    pub fn find_by_name(&self, name: &str) -> Option<User> {
        self.by_name.get(name).cloned()
    }
}

/// 从 `Authorization: Bearer <token>` 提取 token。
fn extract_bearer_token(headers: &HeaderMap) -> Option<&str> {
    let header = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    header.strip_prefix("Bearer ").map(str::trim)
}

/// 常量时间字符串比较，避免计时侧信道泄漏 token 信息（SRV-018）。
fn constant_time_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    a.len() == b.len() && bool::from(a.ct_eq(b))
}

fn unauthorized_response() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({"error": "unauthorized"})),
    )
        .into_response()
}

/// 鉴权中间件：校验 Bearer token，有效则把 [`User`] 注入 request extensions。
pub async fn auth_middleware(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Response {
    let token = match extract_bearer_token(req.headers()) {
        Some(t) => t,
        None => return unauthorized_response(),
    };
    // find_by_token 已做全量常量时间比较，直接信任结果。
    let user = match state.user_store.find_by_token(token) {
        Some(u) => u.clone(),
        _ => return unauthorized_response(),
    };
    req.extensions_mut().insert(user);
    next.run(req).await
}

/// 提取器：从 request extensions 取当前 [`User`]。未鉴权则 401。
///
/// 仅可用于已被 [`auth_middleware`] 保护的路由。
pub struct AuthUser(pub User);

impl<S> FromRequestParts<S> for AuthUser
where
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<User>()
            .cloned()
            .map(AuthUser)
            .ok_or_else(unauthorized_response)
    }
}

// ============ Login 端点 ============

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub name: String,
    pub token: String,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub ok: bool,
    pub user: UserPublic,
    pub ticket: String,
}

/// `POST /api/auth/login`：校验 `{name, token}`，返回 `{ok:true, user, ticket}`。
/// 不套 auth 中间件。响应不含 token（SRV-024），并签发一次性 WS ticket（SRV-002）。
pub async fn login_handler(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, Response> {
    let user = state
        .user_store
        .find_by_name(&req.name)
        .ok_or_else(unauthorized_response)?;
    // 常量时间比对，避免计时侧信道（SRV-018）。
    if !constant_time_eq(&user.token, &req.token) {
        return Err(unauthorized_response());
    }
    let ticket = state.issue_ticket(user.id);
    Ok(Json(LoginResponse {
        ok: true,
        user: UserPublic::from(&user),
        ticket,
    }))
}

// ============ Ticket 端点（WS 一次性 ticket） ============

#[derive(Debug, Serialize)]
pub struct TicketResponse {
    pub ticket: String,
}

/// `GET /api/auth/ticket`：需 Bearer 鉴权，签发一次性 WS ticket（60s TTL）。
pub async fn ticket_handler(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Json<TicketResponse> {
    let ticket = state.issue_ticket(user.id);
    Json(TicketResponse { ticket })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn sample_store() -> UserStore {
        UserStore::from_config(vec![
            ("alice".into(), "alice-token-aaaaaaaa".into()),
            ("bob".into(), "bob-token-bbbbbbbb".into()),
            ("carol".into(), "carol-token-cccccccc".into()),
            ("dave".into(), "dave-token-dddddddd".into()),
            ("eve".into(), "eve-token-eeeeeeee".into()),
        ])
    }

    #[test]
    fn debug_does_not_leak_token() {
        let store = sample_store();
        let user = store.find_by_name("alice").unwrap();
        let debug = format!("{:?}", user);
        assert!(
            !debug.contains("alice-token-aaaaaaaa"),
            "token leaked in Debug output: {}",
            debug
        );
        assert!(debug.contains("alice"), "name should appear: {}", debug);
        assert!(debug.contains("[REDACTED]"), "token should be redacted: {}", debug);
    }

    #[test]
    fn find_by_token_returns_correct_user() {
        let store = sample_store();
        let user = store.find_by_token("bob-token-bbbbbbbb").unwrap();
        assert_eq!(user.name, "bob");
        assert!(store.find_by_token("no-such-token").is_none());
        assert!(store.find_by_token("").is_none());
    }

    #[test]
    fn find_by_token_timing_is_stable() {
        let store = sample_store();
        const ITERS: usize = 10_000;

        // 预热，避免首次分配/缓存差异影响结果。
        for _ in 0..100 {
            std::hint::black_box(store.find_by_token("alice-token-aaaaaaaa"));
        }

        let start = Instant::now();
        for _ in 0..ITERS {
            std::hint::black_box(store.find_by_token("alice-token-aaaaaaaa"));
        }
        let valid_duration = start.elapsed();

        let start = Instant::now();
        // 使用与真实 token 等长的无效 token，避免长度检查引入额外差异。
        for _ in 0..ITERS {
            std::hint::black_box(store.find_by_token("zzzzzzzzzzzzzzzzzzzz"));
        }
        let invalid_duration = start.elapsed();

        let ratio = valid_duration.as_nanos() as f64 / invalid_duration.as_nanos().max(1) as f64;
        eprintln!(
            "find_by_token timing: valid={:?} invalid={:?} ratio={:.2}",
            valid_duration, invalid_duration, ratio
        );

        // 全量扫描+常量时间比较应使两者耗时处于同一数量级；放宽到 3 倍以避免 CI 抖动。
        assert!(
            ratio >= 0.33 && ratio <= 3.0,
            "timing ratio {:.2} out of tolerance",
            ratio
        );
    }
}
