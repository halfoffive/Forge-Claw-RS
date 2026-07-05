//! axum REST 路由：chat / sessions / tools / prompts。
//!
//! 会话存储用 `RwLock<HashMap<Uuid, SessionData>>`（不引入 dashmap）。
//! `AppState` 为 `Clone`（内部全 `Arc`），便于套 auth 中间件时直接 clone 进 extractor。
//!
//! 会话隔离：每个 [`SessionData`] 绑定 `user_id`，`list_sessions`/`get_session`/
//! `chat_handler` 按当前 [`crate::auth::AuthUser`] 过滤，跨用户访问返回 404 不泄漏存在性。

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, StatusCode};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use uuid::Uuid;

use forgeclaw_core::model::{AssistantMsg, Message, Session, ToolCall};
use forgeclaw_llm::{History, ToolSpec};

use crate::auth::{AuthUser, Ticket, UserStore};
use crate::orchestrator::{Orchestrator, OrchestratorEvent, ToolCallRecord};

/// 共享应用状态。`Clone` 廉价（仅 `Arc` 引用计数）。
#[derive(Clone)]
pub struct AppState {
    pub orchestrator: Arc<Orchestrator>,
    pub sessions: Arc<RwLock<HashMap<Uuid, SessionData>>>,
    pub user_store: Arc<UserStore>,
    pub tickets: Arc<RwLock<HashMap<Uuid, Ticket>>>,
    pub allowed_origins: Arc<[HeaderValue]>,
}

/// 单会话数据：核心 Session（含展示用 messages）+ LLM History（cache-first 前缀）+ 所属用户。
///
/// `history` 用 `Arc<RwLock<History>>` 共享，避免并发请求下 read-clone-replace 覆盖消息。
#[derive(Clone)]
pub struct SessionData {
    pub session: Session,
    pub history: Arc<RwLock<History>>,
    pub user_id: Uuid,
}

impl AppState {
    pub fn new(orchestrator: Arc<Orchestrator>, user_store: UserStore) -> Self {
        Self::with_allowed_origins(orchestrator, user_store, Vec::new())
    }

    pub fn with_allowed_origins(
        orchestrator: Arc<Orchestrator>,
        user_store: UserStore,
        allowed_origins: Vec<String>,
    ) -> Self {
        let origins: Arc<[HeaderValue]> = if allowed_origins.is_empty() {
            default_allowed_origins()
        } else {
            allowed_origins
                .into_iter()
                .map(|o| {
                    HeaderValue::from_str(&o)
                        .unwrap_or_else(|_| panic!("invalid allowed origin: {o}"))
                })
                .collect()
        };
        Self {
            orchestrator,
            sessions: Arc::new(RwLock::new(HashMap::new())),
            user_store: Arc::new(user_store),
            tickets: Arc::new(RwLock::new(HashMap::new())),
            allowed_origins: origins,
        }
    }
}

fn default_allowed_origins() -> Arc<[HeaderValue]> {
    vec![
        HeaderValue::from_static("http://localhost:8080"),
        HeaderValue::from_static("http://127.0.0.1:8080"),
    ]
    .into()
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

    // 取出或新建会话（克隆出副本，避免在长 LLM 调用期间持锁）。
    // 跨用户访问既存 session_id 返回 404 不泄漏存在性。
    let mut data = {
        let sessions = state.sessions.read().await;
        match sessions.get(&session_id) {
            Some(d) if d.user_id == user.id => d.clone(),
            Some(_) => {
                return Err((StatusCode::NOT_FOUND, "session not found".into()));
            }
            None => SessionData {
                session: Session::new(session_id),
                history: Arc::new(RwLock::new(History::new())),
                user_id: user.id,
            },
        }
    };
    data.session.append(Message::User(req.message.clone()));

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
        OrchestratorEvent::Error { message } => return Err(internal_error(message)),
        other => return Err(internal_error(other)),
    };

    let assistant_tool_calls: Vec<ToolCall> = tool_calls
        .iter()
        .map(|r| ToolCall {
            id: r.id.clone(),
            tool: r.name.clone(),
            input: serde_json::Value::Null,
        })
        .collect();
    data.session.append(Message::Assistant(AssistantMsg {
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
            message_count: d.session.message_count(),
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
        messages: data.session.messages().to_vec(),
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

fn internal_error<E: std::fmt::Debug>(e: E) -> (StatusCode, String) {
    tracing::error!(?e, "internal server error");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal server error".into(),
    )
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
