//! WebSocket `/ws/chat`：接收 `{message, session_id}` 文本帧，调 orchestrator.run_streaming，
//! 把 [`OrchestratorEvent`] 序列化为 JSON 文本帧发回，直到 Complete/Error。
//!
//! 流式桥接：run_streaming 不内部 spawn，故在此 spawn 一个任务驱动 LLM 循环 + 推送事件，
//! 主循环并发排空 receiver 并转发给 WS 客户端。Complete 后把更新后的 history 回写会话存储。
//!
//! 鉴权（SRV-002）：浏览器 WS 无法设 Authorization header，故从 `?ticket=<ticket>` query
//! 参数取一次性 ticket，经 `AppState::consume_ticket` 核销（60s TTL、用后即焚）；无效则 401。
//!
//! 心跳与超时（SRV-003/SRV-009）：拆分 socket 为 reader/writer，独立 ping 任务每 30s 发 Ping；
//! 每帧读 60s 超时，单连接整体 600s 超时。WS 单帧/单消息上限 256KB（SRV-014）。

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{FromRequestParts, Query, Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio::sync::RwLock;
use uuid::Uuid;

use chrono::Utc;
use forgeclaw_core::model::{AssistantMsg, Message as CoreMessage, Session, ToolCall};
use forgeclaw_llm::History;

use crate::api::{AppState, SessionData};
use crate::orchestrator::{OrchestratorEvent, ToolCallRecord};

/// WS 读帧超时（SRV-003）：60s 无帧即关闭。
const READ_TIMEOUT: Duration = Duration::from_secs(60);
/// WS 整体超时（SRV-009）：单连接最长 600s。
const SESSION_TIMEOUT: Duration = Duration::from_secs(600);
/// 心跳间隔（SRV-003）：每 30s 发 Ping。
const PING_INTERVAL: Duration = Duration::from_secs(30);
/// 单轮 orchestrator 任务超时（P1-SRV-002）：防止 spawn 任务持 history 写锁无限运行。
const TASK_TIMEOUT: Duration = Duration::from_secs(300);
/// WS 单帧/单消息大小上限（SRV-014）。
const MAX_WS_FRAME_SIZE: usize = 256 * 1024;

/// 读取任务超时，支持测试通过环境变量覆盖。
fn task_timeout() -> Duration {
    if let Ok(v) = std::env::var("FORGECLAW_WS_TASK_TIMEOUT_SECS") {
        if let Ok(secs) = v.parse::<u64>() {
            return Duration::from_secs(secs);
        }
    }
    TASK_TIMEOUT
}

/// 包装 `JoinHandle`，在离开作用域时显式 `abort`，保证 SESSION_TIMEOUT 或断连时
/// spawned 的 orchestrator 任务不会泄漏（P1-SRV-002）。对已完成的任务调用 abort 是 no-op。
struct AbortOnDrop<T>(Option<tokio::task::JoinHandle<T>>);

impl<T> AbortOnDrop<T> {
    fn new(handle: tokio::task::JoinHandle<T>) -> Self {
        Self(Some(handle))
    }
}

impl<T> std::ops::Deref for AbortOnDrop<T> {
    type Target = tokio::task::JoinHandle<T>;

    fn deref(&self) -> &Self::Target {
        self.0.as_ref().expect("handle present")
    }
}

impl<T> std::ops::DerefMut for AbortOnDrop<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0.as_mut().expect("handle present")
    }
}

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        if let Some(h) = self.0.take() {
            h.abort();
        }
    }
}

impl<T> std::future::Future for AbortOnDrop<T> {
    type Output = Result<T, tokio::task::JoinError>;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let inner = self.get_mut().0.as_mut().expect("handle present");
        std::pin::Pin::new(inner).poll(cx)
    }
}

#[derive(Debug, Deserialize)]
struct WsChatRequest {
    message: String,
    session_id: Option<String>,
}

/// `/ws/chat` 升级处理器。从 `?ticket=` query 参数核销一次性 ticket 鉴权（SRV-002）。
///
/// 浏览器 WS 无法设 Authorization header，故用一次性 ticket：客户端先调
/// `/api/auth/login` 或 `/api/auth/ticket`（Bearer 鉴权）获取短期 ticket，
/// 再凭 `?ticket=<ticket>` 升级 WS。ticket 60s TTL、用后即焚。
pub async fn ws_chat_handler(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
    req: Request,
) -> Response {
    let user_id = match params.get("ticket").and_then(|t| state.consume_ticket(t)) {
        Some(u) => u,
        None => return (StatusCode::UNAUTHORIZED, "unauthorized").into_response(),
    };
    let (mut parts, _body) = req.into_parts();
    match WebSocketUpgrade::from_request_parts(&mut parts, &state).await {
        Ok(ws) => ws
            .max_message_size(MAX_WS_FRAME_SIZE)
            .max_frame_size(MAX_WS_FRAME_SIZE)
            .on_upgrade(move |socket| handle_ws(socket, state, user_id)),
        Err(rejection) => rejection.into_response(),
    }
}

async fn handle_ws(socket: WebSocket, state: AppState, user_id: Uuid) {
    // 拆分 socket 为 reader/writer，用 mpsc 汇聚 outgoing（响应 + ping）由单一 writer 任务发送，
    // 这样 ping 任务与帧处理可并发往 socket 写入（SRV-003 心跳）。
    let (mut writer, mut reader) = socket.split();
    let (out_tx, mut out_rx) = mpsc::channel::<Message>(64);

    // Writer 任务：排空 out_rx，写入 socket。
    let writer_task = tokio::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            if writer.send(msg).await.is_err() {
                break;
            }
        }
    });

    // Ping 任务：每 30s 发 Ping（SRV-003）。
    let ping_tx = out_tx.clone();
    let ping_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(PING_INTERVAL);
        interval.tick().await; // 跳过首次立即触发
        loop {
            interval.tick().await;
            if ping_tx
                .send(Message::Ping(Bytes::from_static(b"ping")))
                .await
                .is_err()
            {
                break;
            }
        }
    });

    // 整体超时 600s（SRV-009）+ 每帧读超时 60s（SRV-003）。
    let _ = tokio::time::timeout(SESSION_TIMEOUT, async {
        loop {
            match tokio::time::timeout(READ_TIMEOUT, reader.next()).await {
                Ok(Some(Ok(msg))) => match msg {
                    Message::Text(text) => {
                        if handle_text_frame(&out_tx, &state, text.as_str(), user_id)
                            .await
                            .is_break()
                        {
                            break;
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                },
                Ok(Some(Err(e))) => {
                    tracing::warn!(error = %e, "ws read error");
                    break;
                }
                Ok(None) => break,
                Err(_) => {
                    tracing::info!("ws read timeout (60s idle), closing connection");
                    break;
                }
            }
        }
    })
    .await;

    drop(out_tx); // 关闭 outgoing 通道 → writer_task 结束
    ping_task.abort();
    let _ = writer_task.await;
}

/// 处理一帧文本请求。返回 `ControlFlow::Break` 表示应关闭连接。
async fn handle_text_frame(
    out_tx: &mpsc::Sender<Message>,
    state: &AppState,
    text: &str,
    user_id: Uuid,
) -> std::ops::ControlFlow<()> {
    let req: WsChatRequest = match serde_json::from_str(text) {
        Ok(r) => r,
        Err(_) => {
            let _ = send_event(
                out_tx,
                &OrchestratorEvent::Error {
                    message: "invalid request: expected {message, session_id?}".into(),
                },
            )
            .await;
            return std::ops::ControlFlow::Continue(());
        }
    };

    let session_id = match req.session_id.as_deref() {
        Some(s) => match Uuid::parse_str(s) {
            Ok(id) => id,
            Err(_) => {
                let _ = send_event(
                    out_tx,
                    &OrchestratorEvent::Error {
                        message: "invalid session_id".into(),
                    },
                )
                .await;
                return std::ops::ControlFlow::Continue(());
            }
        },
        None => Uuid::new_v4(),
    };

    // 取出或新建会话：只取共享 history Arc，不深拷 messages（R2-SRV-004）。
    // SRV-006：跨用户既存 session_id 发 Error 并 return，不创建不覆盖。
    // P1-D-004/E-005/C-008：用写锁 entry 原子"取或建"，避免并发请求创建多个 history_arc。
    let history_arc = {
        let mut sessions = state.sessions.write().await;
        match sessions.entry(session_id) {
            Entry::Occupied(e) => {
                let d = e.get();
                if d.user_id != user_id {
                    drop(sessions);
                    let _ = send_event(
                        out_tx,
                        &OrchestratorEvent::Error {
                            message: "session not found".into(),
                        },
                    )
                    .await;
                    return std::ops::ControlFlow::Continue(());
                }
                d.history.clone()
            }
            Entry::Vacant(e) => {
                let data = SessionData {
                    session: Session {
                        id: session_id,
                        created_at: Utc::now(),
                        messages: Vec::new(),
                    },
                    history: Arc::new(RwLock::new(History::new())),
                    user_id,
                };
                e.insert(data).history.clone()
            }
        }
    };

    let user_msg_text = req.message.clone();
    let (tx, mut rx) = mpsc::channel::<OrchestratorEvent>(64);
    let orch = state.orchestrator.clone();
    let user_msg = req.message;
    // SRV-007：history 用 Arc<RwLock<History>> 共享，spawn 任务内持写锁跑 run_streaming，
    // 防止并发请求丢失更新。guard 在任务结束自动释放。
    let history_for_task = history_arc.clone();
    let mut join = AbortOnDrop::new(tokio::spawn(async move {
        let mut guard = history_for_task.write().await;
        orch.run_streaming(&mut guard, user_msg, tx).await
    }));

    // 转发事件给 WS 客户端，并捕获 Complete 的 tool_calls（R2-SRV-008）。
    let mut final_text = String::new();
    let mut final_tool_calls: Vec<ToolCallRecord> = Vec::new();
    let mut got_complete = false;
    let mut task_errored = false;

    // P1-SRV-002：同时等待事件、任务完成、单轮超时；超时或断连时 abort 并回 Error 帧。
    let timeout = tokio::time::sleep(task_timeout());
    tokio::pin!(timeout);
    loop {
        // 优先处理 receiver：保证已入队的事件（含上游 Error）在 join 完成前被发出，
        // 避免事件与任务完成同时就绪时随机选择导致漏发（P1-C-011/E-008）。
        tokio::select! {
            biased;
            event = rx.recv() => {
                match event {
                    Some(event) => {
                        // P1-C-011/E-008：上游 Error 事件不直接透传，原始信息落入日志，
                        // 前端只收到通用文案。
                        if let OrchestratorEvent::Error { ref message } = event {
                            tracing::error!(error = %message, "orchestrator error");
                            let _ = send_event(
                                out_tx,
                                &OrchestratorEvent::Error {
                                    message: "internal error".into(),
                                },
                            )
                            .await;
                            task_errored = true;
                            break;
                        }
                        if let OrchestratorEvent::Complete {
                            ref text,
                            ref tool_calls,
                        } = event
                        {
                            final_text = text.clone();
                            final_tool_calls = tool_calls.clone();
                            got_complete = true;
                        }
                        if send_event(out_tx, &event).await.is_err() {
                            // 客户端已断开：停止生成并释放锁。
                            join.abort();
                            break;
                        }
                        if got_complete {
                            break;
                        }
                    }
                    None => {
                        // Orchestrator 已结束或通道关闭，等待 join 观察结果。
                        match join.await {
                            Ok(Ok(())) => {}
                            Ok(Err(e)) => {
                                task_errored = true;
                                tracing::error!(error = %e, "run_streaming failed");
                                let _ = send_event(
                                    out_tx,
                                    &OrchestratorEvent::Error {
                                        message: "orchestrator task failed".into(),
                                    },
                                )
                                .await;
                            }
                            Err(_) => {
                                task_errored = true;
                                tracing::error!("orchestrator task panicked");
                                let _ = send_event(
                                    out_tx,
                                    &OrchestratorEvent::Error {
                                        message: "orchestrator task panicked".into(),
                                    },
                                )
                                .await;
                            }
                        }
                        break;
                    }
                }
            }
            res = &mut join => {
                match res {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => {
                        task_errored = true;
                        tracing::error!(error = %e, "run_streaming failed");
                        let _ = send_event(
                            out_tx,
                            &OrchestratorEvent::Error {
                                message: "orchestrator task failed".into(),
                            },
                        )
                        .await;
                    }
                    Err(_) => {
                        task_errored = true;
                        tracing::error!("orchestrator task panicked");
                        let _ = send_event(
                            out_tx,
                            &OrchestratorEvent::Error {
                                message: "orchestrator task panicked".into(),
                            },
                        )
                        .await;
                    }
                }
                break;
            }
            _ = &mut timeout => {
                tracing::error!("orchestrator task timed out after {:?}", task_timeout());
                join.abort();
                // 等待任务真正释放 history 写锁后再继续，避免同 session 后续请求阻塞。
                let _ = join.await;
                task_errored = true;
                let _ = send_event(
                    out_tx,
                    &OrchestratorEvent::Error {
                        message: "orchestrator task timed out".into(),
                    },
                )
                .await;
                break;
            }
        }
    }
    drop(rx);

    // 回写会话。R2-SRV-004：append 新消息而非整体 insert，避免并发覆盖。
    // P1-D-015/E-006/C-009：写回前复核 user_id，跨用户时不污染既有会话。
    if !task_errored && got_complete {
        let assistant_tool_calls: Vec<ToolCall> = final_tool_calls
            .iter()
            .map(|r| ToolCall {
                id: r.id.clone(),
                tool: r.name.clone(),
                input: r.input.clone(),
            })
            .collect();
        let new_messages = vec![
            CoreMessage::User(user_msg_text),
            CoreMessage::Assistant(AssistantMsg {
                text: final_text,
                tool_calls: assistant_tool_calls,
            }),
        ];
        let mut sessions = state.sessions.write().await;
        match sessions.get_mut(&session_id) {
            Some(d) if d.user_id == user_id => {
                d.session.messages.extend(new_messages);
            }
            _ => {
                sessions.insert(
                    session_id,
                    SessionData {
                        session: Session {
                            id: session_id,
                            created_at: Utc::now(),
                            messages: new_messages,
                        },
                        history: history_arc,
                        user_id,
                    },
                );
            }
        }
    }

    std::ops::ControlFlow::Continue(())
}

async fn send_event(
    out_tx: &mpsc::Sender<Message>,
    event: &OrchestratorEvent,
) -> Result<(), mpsc::error::SendError<Message>> {
    let json = serde_json::to_string(event).unwrap_or_else(|_| "{}".into());
    out_tx.send(Message::Text(json.into())).await
}
