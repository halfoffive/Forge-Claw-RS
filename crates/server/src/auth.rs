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
use chrono::{DateTime, Duration, Utc};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use uuid::Uuid;

use crate::api::AppState;

/// 用户模型（内部使用，token 用 `SecretString` 保护）。
#[derive(Clone, Debug)]
pub struct User {
    pub id: Uuid,
    pub name: String,
    pub token: SecretString,
}

/// 对外暴露的用户信息，不含 token。
#[derive(Debug, Clone, Serialize)]
pub struct UserPublic {
    pub id: Uuid,
    pub name: String,
}

impl From<&User> for UserPublic {
    fn from(user: &User) -> Self {
        Self {
            id: user.id,
            name: user.name.clone(),
        }
    }
}

/// 一次性 WebSocket ticket（60s 有效）。
#[derive(Clone, Debug)]
pub struct Ticket {
    pub user_id: Uuid,
    pub expires_at: DateTime<Utc>,
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
            by_token.insert(u.token.expose_secret().to_string(), u.clone());
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
                token: SecretString::new(token.into()),
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

    pub fn find_by_id(&self, id: Uuid) -> Option<User> {
        self.by_name.values().find(|u| u.id == id).cloned()
    }
}

impl AppState {
    /// 签发一个 60s 有效的一次性 WS ticket。
    pub async fn issue_ticket(&self, user_id: Uuid) -> Uuid {
        let ticket = Uuid::new_v4();
        let expires_at = Utc::now() + Duration::seconds(60);
        let mut tickets = self.tickets.write().await;
        tickets.insert(
            ticket,
            Ticket {
                user_id,
                expires_at,
            },
        );
        ticket
    }

    /// 消费 ticket：返回对应用户。ticket 用后即焚，过期也返回 None。
    pub async fn consume_ticket(&self, ticket: Uuid) -> Option<User> {
        let mut tickets = self.tickets.write().await;
        let t = tickets.remove(&ticket)?;
        if t.expires_at < Utc::now() {
            return None;
        }
        self.user_store.find_by_id(t.user_id)
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
    pub user: UserPublic,
    pub ticket: Uuid,
}

/// `POST /api/auth/login`：校验 `{name, token}`，返回 `{ok:true, user, ticket}`。
/// ticket 用于 `/ws/chat?ticket=...`，60s 内一次性有效。
/// 不套 auth 中间件。
fn constant_time_token_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && bool::from(a.as_bytes().ct_eq(b.as_bytes()))
}

pub async fn login_handler(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, Response> {
    let user = state
        .user_store
        .find_by_name(&req.name)
        .ok_or_else(unauthorized_response)?;
    if !constant_time_token_eq(user.token.expose_secret(), &req.token) {
        return Err(unauthorized_response());
    }
    let ticket = state.issue_ticket(user.id).await;
    Ok(Json(LoginResponse {
        ok: true,
        user: UserPublic::from(&user),
        ticket,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_tokens_match() {
        assert!(constant_time_token_eq("alice-token", "alice-token"));
    }

    #[test]
    fn different_tokens_do_not_match() {
        assert!(!constant_time_token_eq("alice-token", "alice-tokex"));
    }

    #[test]
    fn different_length_tokens_do_not_match() {
        assert!(!constant_time_token_eq("short", "longer"));
    }

    #[test]
    fn login_response_does_not_leak_token() {
        let secret_token = "super-secret-token";
        let user = User {
            id: Uuid::new_v4(),
            name: "alice".into(),
            token: SecretString::new(secret_token.into()),
        };
        let resp = LoginResponse {
            ok: true,
            user: UserPublic::from(&user),
            ticket: Uuid::new_v4(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(!json.contains(secret_token));
        assert!(json.contains("\"name\":\"alice\""));
    }

    #[test]
    fn user_debug_does_not_leak_token() {
        let secret_token = "debug-secret-token";
        let user = User {
            id: Uuid::new_v4(),
            name: "alice".into(),
            token: SecretString::new(secret_token.into()),
        };
        let out = format!("{:?}", user);
        assert!(!out.contains(secret_token));
    }
}
