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
use forgeclaw_core::model::{AssistantMsg, Message as CoreMessage, Session};
use forgeclaw_llm::History;

use crate::api::{AppState, SessionData};
use crate::orchestrator::OrchestratorEvent;

/// WS 读帧超时（SRV-003）：60s 无帧即关闭。
const READ_TIMEOUT: Duration = Duration::from_secs(60);
/// WS 整体超时（SRV-009）：单连接最长 600s。
const SESSION_TIMEOUT: Duration = Duration::from_secs(600);
/// 心跳间隔（SRV-003）：每 30s 发 Ping。
const PING_INTERVAL: Duration = Duration::from_secs(30);
/// WS 单帧/单消息大小上限（SRV-014）。
const MAX_WS_FRAME_SIZE: usize = 256 * 1024;

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

    let session_id = req
        .session_id
        .as_deref()
        .and_then(|s| Uuid::parse_str(s).ok())
        .unwrap_or_else(Uuid::new_v4);

    // 取出或新建会话（克隆副本，避免在长 LLM 调用期间持 sessions 锁）。
    // SRV-006：跨用户既存 session_id 发 Error 并 return，不创建不覆盖。
    let mut data = {
        let sessions = state.sessions.read().await;
        match sessions.get(&session_id) {
            Some(d) if d.user_id == user_id => d.clone(),
            Some(_) => {
                let _ = send_event(
                    out_tx,
                    &OrchestratorEvent::Error {
                        message: "session not found".into(),
                    },
                )
                .await;
                return std::ops::ControlFlow::Continue(());
            }
            None => SessionData {
                session: Session {
                    id: session_id,
                    created_at: Utc::now(),
                    messages: Vec::new(),
                },
                history: Arc::new(RwLock::new(History::new())),
                user_id,
            },
        }
    };
    data.session
        .messages
        .push(CoreMessage::User(req.message.clone()));

    let (tx, mut rx) = mpsc::channel::<OrchestratorEvent>(64);
    let orch = state.orchestrator.clone();
    let user_msg = req.message;
    // SRV-007：history 用 Arc<RwLock<History>> 共享，spawn 任务内持写锁跑 run_streaming，
    // 防止并发请求丢失更新。guard 在任务结束自动释放。
    let history_arc = data.history.clone();
    let join = tokio::spawn(async move {
        let mut guard = history_arc.write().await;
        let res = orch.run_streaming(&mut guard, user_msg, tx).await;
        (data, res)
    });

    // 转发事件给 WS 客户端
    let mut final_text = String::new();
    let mut got_complete = false;
    while let Some(event) = rx.recv().await {
        if let OrchestratorEvent::Complete { ref text, .. } = event {
            final_text = text.clone();
            got_complete = true;
        }
        if send_event(out_tx, &event).await.is_err() {
            break;
        }
        if matches!(
            event,
            OrchestratorEvent::Complete { .. } | OrchestratorEvent::Error { .. }
        ) {
            break;
        }
    }
    drop(rx);

    // 回写会话。SRV-013：join.await 的 Err 分支记日志。
    match join.await {
        Ok((mut final_data, res)) => {
            if let Err(e) = res {
                tracing::error!(error = %e, "run_streaming failed");
            }
            if got_complete {
                final_data
                    .session
                    .messages
                    .push(CoreMessage::Assistant(AssistantMsg {
                        text: final_text,
                        tool_calls: Vec::new(),
                    }));
            }
            let mut sessions = state.sessions.write().await;
            sessions.insert(session_id, final_data);
        }
        Err(e) => {
            tracing::error!(error = %e, "orchestrator task join failed");
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
