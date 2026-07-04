//! 多用户鉴权：User/UserStore 模型、Bearer token 中间件、AuthUser 提取器、login 端点。
//!
//! MVP 策略：静态预配置 token 比对（无 JWT/无密码哈希）。会话隔离通过
//! [`crate::api::SessionData::user_id`] 过滤实现，跨用户访问返回 404 不泄漏存在性。

use std::collections::HashMap;

use axum::extract::{FromRequestParts, Request, State};
use axum::http::{header, request::Parts, HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api::AppState;

/// 用户模型。
#[derive(Clone, Debug, Serialize)]
pub struct User {
    pub id: Uuid,
    pub name: String,
    pub token: String,
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

    pub fn find_by_token(&self, token: &str) -> Option<User> {
        self.by_token.get(token).cloned()
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
    let user = match state.user_store.find_by_token(token) {
        Some(u) => u,
        None => return unauthorized_response(),
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
    pub user: User,
}

/// `POST /api/auth/login`：校验 `{name, token}`，返回 `{ok:true, user}`。
/// 不套 auth 中间件。
pub async fn login_handler(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, Response> {
    let user = state
        .user_store
        .find_by_name(&req.name)
        .ok_or_else(unauthorized_response)?;
    if user.token != req.token {
        return Err(unauthorized_response());
    }
    Ok(Json(LoginResponse { ok: true, user }))
}
