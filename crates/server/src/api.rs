//! axum REST 路由：chat / sessions / tools / prompts。
//!
//! 会话存储用 `RwLock<HashMap<Uuid, SessionData>>`（不引入 dashmap）。
//! `AppState` 为 `Clone`（内部全 `Arc`），便于套 auth 中间件时直接 clone 进 extractor。
//!
//! 会话隔离：每个 [`SessionData`] 绑定 `user_id`，`list_sessions`/`get_session`/
//! `chat_handler` 按当前 [`crate::auth::AuthUser`] 过滤，跨用户访问返回 404 不泄漏存在性。
//!
//! WS 一次性 ticket：[`AppState`] 维护 `tickets` 表（`Mutex<HashMap>`），由
//! `issue_ticket`/`consume_ticket` 签发与核销（60s TTL，用后即焚）。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use uuid::Uuid;

use forgeclaw_core::model::{AssistantMsg, Message, Session, ToolCall};
use forgeclaw_llm::{History, ToolSpec};

use crate::auth::{AuthUser, UserStore};
use crate::orchestrator::{Orchestrator, OrchestratorEvent, ToolCallRecord};

/// WS ticket TTL：签发后 60s 内有效。
const TICKET_TTL: Duration = Duration::from_secs(60);

/// 共享应用状态。`Clone` 廉价（仅 `Arc` 引用计数）。
#[derive(Clone)]
pub struct AppState {
    pub orchestrator: Arc<Orchestrator>,
    pub sessions: Arc<RwLock<HashMap<Uuid, SessionData>>>,
    pub user_store: Arc<UserStore>,
    /// CORS 白名单（SRV-001）。
    pub allowed_origins: Vec<String>,
    /// WS 一次性 ticket 表：`ticket -> (user_id, issued_at)`（SRV-002）。
    pub tickets: Arc<std::sync::Mutex<HashMap<String, (Uuid, Instant)>>>,
}

/// 单会话数据：核心 Session（含展示用 messages）+ LLM History（cache-first 前缀）+ 所属用户。
///
/// `history` 用 `Arc<RwLock<History>>` 共享，`chat_handler` 持写锁跑 LLM 防丢失更新（SRV-007）。
#[derive(Clone)]
pub struct SessionData {
    pub session: Session,
    pub history: Arc<RwLock<History>>,
    pub user_id: Uuid,
}

impl AppState {
    pub fn new(orchestrator: Arc<Orchestrator>, user_store: UserStore) -> Self {
        Self {
            orchestrator,
            sessions: Arc::new(RwLock::new(HashMap::new())),
            user_store: Arc::new(user_store),
            allowed_origins: vec![
                "http://localhost:5173".to_string(),
                "http://127.0.0.1:5173".to_string(),
            ],
            tickets: Arc::new(std::sync::Mutex::new(HashMap::new())),
        }
    }

    /// 签发一次性 WS ticket，绑定 `user_id`，60s TTL。
    pub fn issue_ticket(&self, user_id: Uuid) -> String {
        let ticket = Uuid::new_v4().to_string();
        let mut tickets = self.tickets.lock().expect("tickets mutex poisoned");
        tickets.insert(ticket.clone(), (user_id, Instant::now()));
        ticket
    }

    /// 核销 ticket：返回对应 `user_id`。TTL 过期或不存在返回 `None`。用后即焚。
    pub fn consume_ticket(&self, ticket: &str) -> Option<Uuid> {
        let mut tickets = self.tickets.lock().expect("tickets mutex poisoned");
        match tickets.remove(ticket) {
            Some((user_id, issued_at))
                if Instant::now().duration_since(issued_at) <= TICKET_TTL =>
            {
                Some(user_id)
            }
            _ => None,
        }
    }
}

/// 统一构造 500 响应：详细错误落 `tracing::error!`，响应体仅返回通用文案（SRV-010）。
fn internal_error(e: impl std::fmt::Display) -> (StatusCode, String) {
    tracing::error!(error = %e, "internal server error");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal server error".to_string(),
    )
}

// ============ DTO ============

#[derive(Debug, Deserialize)]
pub struct ChatRequestDto {
    pub message: String,
    pub session_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ChatResponseDto {
    pub session_id: String,
    pub text: String,
    pub tool_calls: Vec<ToolCallRecord>,
}

#[derive(Debug, Serialize)]
pub struct SessionSummaryDto {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub message_count: usize,
}

#[derive(Debug, Serialize)]
pub struct SessionDetailDto {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub messages: Vec<Message>,
}

#[derive(Debug, Serialize)]
pub struct ToolsResponseDto {
    pub tools: Vec<ToolInfoDto>,
}

#[derive(Debug, Serialize)]
pub struct ToolInfoDto {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct CompilePromptRequestDto {
    pub profile: String,
}

#[derive(Debug, Serialize)]
pub struct CompilePromptResponseDto {
    pub prompt: String,
}

#[derive(Debug, Deserialize)]
pub struct SectionsQueryDto {
    pub profile: String,
}

// ============ Handlers ============

pub async fn chat_handler(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Json(req): Json<ChatRequestDto>,
) -> Result<Json<ChatResponseDto>, (StatusCode, String)> {
    let session_id = match req.session_id.as_deref() {
        Some(id) => Uuid::parse_str(id)
            .map_err(|_| (StatusCode::BAD_REQUEST, "invalid session_id".into()))?,
        None => Uuid::new_v4(),
    };

    // 取出或新建会话（克隆出副本，避免在长 LLM 调用期间持 sessions 锁）。
    // 跨用户访问既存 session_id 返回 404 不泄漏存在性。
    let mut data = {
        let sessions = state.sessions.read().await;
        match sessions.get(&session_id) {
            Some(d) if d.user_id == user.id => d.clone(),
            Some(_) => {
                return Err((StatusCode::NOT_FOUND, "session not found".into()));
            }
            None => SessionData {
                session: Session {
                    id: session_id,
                    created_at: Utc::now(),
                    messages: Vec::new(),
                },
                history: Arc::new(RwLock::new(History::new())),
                user_id: user.id,
            },
        }
    };
    data.session
        .messages
        .push(Message::User(req.message.clone()));

    // 持 history 写锁跑 run_once，避免并发请求丢失更新（SRV-007）。
    let event = {
        let mut history_guard = data.history.write().await;
        state
            .orchestrator
            .run_once(&mut history_guard, req.message)
            .await
            .map_err(internal_error)?
    };

    let (text, tool_calls) = match event {
        OrchestratorEvent::Complete { text, tool_calls } => (text, tool_calls),
        OrchestratorEvent::Error { message } => {
            tracing::error!(error = %message, "orchestrator error");
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".to_string(),
            ));
        }
        other => {
            tracing::error!(event = ?other, "unexpected orchestrator event");
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".to_string(),
            ));
        }
    };

    let assistant_tool_calls: Vec<ToolCall> = tool_calls
        .iter()
        .map(|r| ToolCall {
            id: r.id.clone(),
            tool: r.name.clone(),
            input: serde_json::Value::Null,
        })
        .collect();
    data.session.messages.push(Message::Assistant(AssistantMsg {
        text: text.clone(),
        tool_calls: assistant_tool_calls,
    }));

    {
        let mut sessions = state.sessions.write().await;
        sessions.insert(session_id, data);
    }

    Ok(Json(ChatResponseDto {
        session_id: session_id.to_string(),
        text,
        tool_calls,
    }))
}

pub async fn list_sessions(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Json<Vec<SessionSummaryDto>> {
    let sessions = state.sessions.read().await;
    let summaries = sessions
        .values()
        .filter(|d| d.user_id == user.id)
        .map(|d| SessionSummaryDto {
            id: d.session.id.to_string(),
            created_at: d.session.created_at,
            message_count: d.session.messages.len(),
        })
        .collect();
    Json(summaries)
}

pub async fn get_session(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<String>,
) -> Result<Json<SessionDetailDto>, (StatusCode, String)> {
    let uuid =
        Uuid::parse_str(&id).map_err(|_| (StatusCode::BAD_REQUEST, "invalid session id".into()))?;
    let sessions = state.sessions.read().await;
    let data = sessions
        .get(&uuid)
        .filter(|d| d.user_id == user.id)
        .ok_or((StatusCode::NOT_FOUND, "session not found".into()))?;
    Ok(Json(SessionDetailDto {
        id: data.session.id.to_string(),
        created_at: data.session.created_at,
        messages: data.session.messages.clone(),
    }))
}

pub async fn list_tools(State(state): State<AppState>) -> Json<ToolsResponseDto> {
    let tools: Vec<ToolInfoDto> = state
        .orchestrator
        .tool_specs()
        .iter()
        .map(spec_to_dto)
        .collect();
    Json(ToolsResponseDto { tools })
}

fn spec_to_dto(spec: &ToolSpec) -> ToolInfoDto {
    ToolInfoDto {
        name: spec.function.name.clone(),
        description: spec.function.description.clone(),
        parameters: spec.function.parameters.clone(),
    }
}

pub async fn compile_prompt(
    State(state): State<AppState>,
    Json(req): Json<CompilePromptRequestDto>,
) -> Result<Json<CompilePromptResponseDto>, (StatusCode, String)> {
    let prompt = state
        .orchestrator
        .compile_prompt(&req.profile)
        .await
        .map_err(internal_error)?;
    Ok(Json(CompilePromptResponseDto { prompt }))
}

pub async fn list_sections(
    State(state): State<AppState>,
    Query(q): Query<SectionsQueryDto>,
) -> Result<Json<Vec<forgeclaw_core::model::Section>>, (StatusCode, String)> {
    let sections = state
        .orchestrator
        .list_sections(&q.profile)
        .await
        .map_err(internal_error)?;
    Ok(Json(sections))
}
