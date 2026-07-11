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
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use uuid::Uuid;

use forgeclaw_core::model::{AssistantMsg, Message, Session, ToolCall, ToolResult};
use forgeclaw_llm::{History, Role, ToolSpec};

use crate::auth::{AuthUser, UserStore};
use crate::orchestrator::{Orchestrator, OrchestratorEvent, ToolCallRecord};

/// WS ticket TTL：签发后 60s 内有效。
const TICKET_TTL: Duration = Duration::from_secs(60);

/// tickets 表上限，防止无限增长。
const MAX_TICKETS: usize = 10_000;

/// 共享应用状态。`Clone` 廉价（仅 `Arc` 引用计数）。
#[derive(Clone)]
pub struct AppState {
    pub orchestrator: Arc<Orchestrator>,
    pub sessions: Arc<RwLock<HashMap<Uuid, SessionData>>>,
    pub user_store: Arc<UserStore>,
    /// CORS 白名单（SRV-001）。
    pub allowed_origins: Vec<String>,
    /// WS 一次性 ticket 表：`ticket -> (user_id, issued_at)`（SRV-002）。
    pub tickets: Arc<Mutex<HashMap<String, (Uuid, Instant)>>>,
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
    pub fn new(
        orchestrator: Arc<Orchestrator>,
        user_store: UserStore,
        allowed_origins: Vec<String>,
    ) -> Self {
        Self {
            orchestrator,
            sessions: Arc::new(RwLock::new(HashMap::new())),
            user_store: Arc::new(user_store),
            allowed_origins,
            tickets: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// 签发一次性 WS ticket，绑定 `user_id`，60s TTL。
    ///
    /// 签发前先清理过期条目；若清理后仍超过上限则拒绝签发，返回空字符串。
    /// 对 std::sync::Mutex 的 poison 进行恢复，避免其他线程 panic 导致主进程 panic。
    pub fn issue_ticket(&self, user_id: Uuid) -> String {
        let mut tickets = match self.tickets.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };

        let now = Instant::now();
        tickets.retain(|_, (_, issued_at)| now.duration_since(*issued_at) <= TICKET_TTL);

        if tickets.len() >= MAX_TICKETS {
            return String::new();
        }

        let ticket = Uuid::new_v4().to_string();
        tickets.insert(ticket.clone(), (user_id, Instant::now()));
        ticket
    }

    /// 核销 ticket：返回对应 `user_id`。TTL 过期或不存在返回 `None`。用后即焚。
    pub fn consume_ticket(&self, ticket: &str) -> Option<Uuid> {
        let mut tickets = match self.tickets.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
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

#[derive(Debug, Deserialize)]
pub struct SaveSectionsRequestDto {
    pub profile: String,
    pub sections: Vec<forgeclaw_core::model::Section>,
}

/// 将 LLM [`History`] 派生为展示用的 [`Vec<Message>`]。
///
/// 作为 session.messages 的唯一真源：history 与 session.messages 不再分别维护，
/// 写回时直接替换，避免手动 extend 导致不同步（B-010）。
pub fn history_to_messages(history: &History) -> Vec<Message> {
    let msgs = history.messages();
    let mut out = Vec::with_capacity(msgs.len());

    for (i, m) in msgs.iter().enumerate() {
        match m.role {
            Role::System => {}
            Role::User => out.push(Message::User(m.content.clone())),
            Role::Assistant => {
                let tool_calls = m
                    .tool_calls
                    .as_ref()
                    .map(|dtos| {
                        dtos.iter()
                            .map(|dto| ToolCall {
                                id: dto.id.clone(),
                                tool: dto.function.name.clone(),
                                input: serde_json::from_str(&dto.function.arguments)
                                    .unwrap_or(serde_json::Value::Null),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                out.push(Message::Assistant(AssistantMsg {
                    text: m.content.clone(),
                    tool_calls,
                }));
            }
            Role::Tool => {
                let tool_call_id = m.tool_call_id.clone().unwrap_or_default();
                let mut found = None;
                for prev in msgs[..i].iter().rev() {
                    if prev.role == Role::Assistant {
                        if let Some(ref dtos) = prev.tool_calls {
                            if let Some(dto) = dtos.iter().find(|d| d.id == tool_call_id) {
                                found = Some(ToolCall {
                                    id: dto.id.clone(),
                                    tool: dto.function.name.clone(),
                                    input: serde_json::from_str(&dto.function.arguments)
                                        .unwrap_or(serde_json::Value::Null),
                                });
                                break;
                            }
                        }
                    }
                }
                let tool_call = found.unwrap_or(ToolCall {
                    id: tool_call_id,
                    tool: "unknown".into(),
                    input: serde_json::Value::Null,
                });

                let (error, output) = if let Some(rest) = m.content.strip_prefix("error: ") {
                    if let Some(idx) = rest.find('\n') {
                        (Some(rest[..idx].to_string()), rest[idx + 1..].to_string())
                    } else {
                        (Some(rest.to_string()), String::new())
                    }
                } else {
                    (None, m.content.clone())
                };

                out.push(Message::Tool(
                    tool_call,
                    ToolResult {
                        output,
                        error,
                        duration_ms: 0,
                    },
                ));
            }
        }
    }

    out
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

    // 取出或新建会话：单次写锁 + entry/or_insert_with 消除 read-write 之间的 TOCTOU。
    // 同一 session_id 的并发请求始终拿到同一个 Arc<RwLock<History>>，history 与 session.messages 不会丢消息。
    // 跨用户访问既存 session_id 返回 404 不泄漏存在性。
    let history_arc = {
        let mut sessions = state.sessions.write().await;
        let d = sessions.entry(session_id).or_insert_with(|| SessionData {
            session: Session {
                id: session_id,
                created_at: Utc::now(),
                messages: Vec::new(),
            },
            history: Arc::new(RwLock::new(History::new())),
            user_id: user.id,
        });
        if d.user_id != user.id {
            return Err((StatusCode::NOT_FOUND, "session not found".into()));
        }
        d.history.clone()
    };

    // 持 history 写锁跑 run_once，避免并发请求丢失更新（SRV-007）。
    let event = {
        let mut history_guard = history_arc.write().await;
        state
            .orchestrator
            .run_once(&mut history_guard, req.message.clone())
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

    {
        // 会话已在 entry/or_insert_with 时创建，此处用 history 派生 messages，
        // 保证 session.messages 与 history 真源一致（B-010）。
        // C-NEW-003：写回前再次复核 user_id，防止跨用户竞态污染。
        let mut sessions = state.sessions.write().await;
        let d = sessions
            .get_mut(&session_id)
            .expect("session must exist after entry/or_insert_with");
        if d.user_id != user.id {
            return Err((StatusCode::NOT_FOUND, "session not found".into()));
        }
        d.session.messages = history_to_messages(&*history_arc.read().await);
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
    let mut summaries = Vec::new();
    for d in sessions.values() {
        if d.user_id != user.id {
            continue;
        }
        summaries.push(SessionSummaryDto {
            id: d.session.id.to_string(),
            created_at: d.session.created_at,
            message_count: history_to_messages(&*d.history.read().await).len(),
        });
    }
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
    let messages = history_to_messages(&*data.history.read().await);
    Ok(Json(SessionDetailDto {
        id: data.session.id.to_string(),
        created_at: data.session.created_at,
        messages,
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

pub async fn save_sections(
    State(state): State<AppState>,
    Json(req): Json<SaveSectionsRequestDto>,
) -> Result<StatusCode, (StatusCode, String)> {
    state
        .orchestrator
        .save_sections(&req.profile, req.sections)
        .await
        .map_err(internal_error)?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use async_trait::async_trait;
    use forgeclaw_llm::{ChatRequest, Event, LlmClient};
    use futures::stream::BoxStream;

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
        let dir = tempfile::tempdir().unwrap();
        let (sandbox, specs) = crate::default_sandbox_with_specs(dir.path().to_path_buf());
        let prompts_root =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../prompts/profiles");
        let llm: Arc<dyn LlmClient> = Arc::new(MockClient {
            counter: AtomicUsize::new(0),
        });
        let orch = crate::Orchestrator::new(
            llm,
            Arc::new(sandbox),
            specs,
            prompts_root,
            "default".into(),
            "deepseek-chat".into(),
            dir.path().to_path_buf(),
        );
        let user_store = crate::UserStore::from_config(vec![(
            "alice".into(),
            "alice-token-aaaaaaaa".into(),
        )]);
        (
            AppState::new(
                Arc::new(orch),
                user_store,
                vec!["http://localhost:5173".to_string()],
            ),
            dir,
        )
    }

    #[test]
    fn issue_ticket_sweeps_expired_entries() {
        let (state, _dir) = build_state();
        let user_id = Uuid::new_v4();

        // 直接写入一条已过期 ticket。
        {
            let mut tickets = state.tickets.lock().unwrap();
            let expired_at = Instant::now() - TICKET_TTL - Duration::from_secs(1);
            tickets.insert("expired-ticket".into(), (user_id, expired_at));
        }

        // 签发新 ticket 时应清理过期项。
        let new_ticket = state.issue_ticket(user_id);
        assert!(!new_ticket.is_empty());

        let tickets = state.tickets.lock().unwrap();
        assert!(
            !tickets.contains_key("expired-ticket"),
            "expired ticket should be swept"
        );
        assert!(tickets.contains_key(&new_ticket));
        assert_eq!(tickets.len(), 1);
    }

    #[test]
    fn ticket_mutex_poison_is_recovered() {
        let (state, _dir) = build_state();
        let user_id = Uuid::new_v4();

        // 故意在持锁时 panic，制造 poison。
        let result = std::panic::catch_unwind(|| {
            let mut guard = state.tickets.lock().unwrap();
            guard.insert("poisoned".into(), (user_id, Instant::now()));
            panic!("intentional poison");
        });
        assert!(result.is_err(), "panic should be caught");

        // 后续调用不应传播 panic。
        let ticket = state.issue_ticket(user_id);
        assert!(!ticket.is_empty());

        let consumed = state.consume_ticket(&ticket);
        assert_eq!(consumed, Some(user_id));
    }

    #[test]
    fn issue_ticket_rejects_when_over_limit() {
        let (state, _dir) = build_state();
        let user_id = Uuid::new_v4();

        // 填满到上限。
        {
            let mut tickets = state.tickets.lock().unwrap();
            for i in 0..MAX_TICKETS {
                tickets.insert(format!("ticket-{i}"), (user_id, Instant::now()));
            }
        }

        let ticket = state.issue_ticket(user_id);
        assert!(ticket.is_empty(), "should reject new ticket when at limit");
    }
}
