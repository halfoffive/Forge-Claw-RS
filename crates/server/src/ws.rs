//! WebSocket `/ws/chat`：接收 `{message, session_id}` 文本帧，调 orchestrator.run_streaming，
//! 把 [`OrchestratorEvent`] 序列化为 JSON 文本帧发回，直到 Complete/Error。
//!
//! 流式桥接：run_streaming 不内部 spawn，故在此 spawn 一个任务驱动 LLM 循环 + 推送事件，
//! 主循环并发排空 receiver 并转发给 WS 客户端。Complete 后把更新后的 history 回写会话存储。
//!
//! 鉴权：浏览器 WS 无法设 Authorization header，故从 `?ticket=<ticket>` query 参数取一次性 ticket，
//! 经 `state.consume_ticket` 校验；无效或过期则返回 401 不升级。有效会话绑定 user_id。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{FromRequestParts, Query, Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use futures::StreamExt;
use serde::Deserialize;
use tokio::sync::{mpsc, RwLock};
use tokio::time::{interval_at, Instant};
use uuid::Uuid;

use forgeclaw_core::model::{AssistantMsg, Message as CoreMessage, Session};
use forgeclaw_llm::History;

use crate::api::{AppState, SessionData};
use crate::orchestrator::OrchestratorEvent;

/// 心跳间隔：每 30s 发送一次 Ping。
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
/// Pong 超时：60s 内未收到 Pong 则关闭连接。
const PONG_TIMEOUT: Duration = Duration::from_secs(60);
/// LLM 任务关闭等待超时：断开或 Complete 后最多等 120s。
const JOIN_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(120);

/// 允许测试通过环境变量覆盖心跳间隔，避免真实等待 30s。
fn heartbeat_interval() -> Duration {
    std::env::var("FORGECLAW_WS_HEARTBEAT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(HEARTBEAT_INTERVAL)
}

/// 允许测试通过环境变量覆盖 Pong 超时。
fn pong_timeout() -> Duration {
    std::env::var("FORGECLAW_WS_PONG_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(PONG_TIMEOUT)
}

#[derive(Debug, Deserialize)]
struct WsChatRequest {
    message: String,
    session_id: Option<String>,
}

/// `/ws/chat` 升级处理器。从 `?ticket=` query 参数鉴权。
///
/// ticket 由 `/api/auth/login` 返回，60s 内一次性有效。
/// ticket 校验在 WS 升级前完成：无效/过期 → 401 不升级。校验通过后手动调
/// `WebSocketUpgrade::from_request_parts` 完成升级握手。
pub async fn ws_chat_handler(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
    req: Request,
) -> Response {
    let ticket = match params.get("ticket").and_then(|t| Uuid::parse_str(t).ok()) {
        Some(t) => t,
        None => return (StatusCode::UNAUTHORIZED, "unauthorized").into_response(),
    };
    let user = match state.consume_ticket(ticket).await {
        Some(u) => u,
        None => return (StatusCode::UNAUTHORIZED, "unauthorized").into_response(),
    };
    let (mut parts, _body) = req.into_parts();
    match WebSocketUpgrade::from_request_parts(&mut parts, &state).await {
        Ok(ws) => ws
            .max_message_size(256 * 1024)
            .on_upgrade(move |socket| handle_ws(socket, state, user.id)),
        Err(rejection) => rejection.into_response(),
    }
}

async fn handle_ws(mut socket: WebSocket, state: AppState, user_id: Uuid) {
    while let Some(msg_result) = socket.next().await {
        let msg = match msg_result {
            Ok(m) => m,
            Err(_) => break,
        };
        match msg {
            Message::Text(text) => {
                if handle_text_frame(&mut socket, &state, text.as_str(), user_id)
                    .await
                    .is_break()
                {
                    break;
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
}

/// 处理一帧文本请求。返回 `ControlFlow::Break` 表示应关闭连接。
async fn handle_text_frame(
    socket: &mut WebSocket,
    state: &AppState,
    text: &str,
    user_id: Uuid,
) -> std::ops::ControlFlow<()> {
    let req: WsChatRequest = match serde_json::from_str(text) {
        Ok(r) => r,
        Err(_) => {
            let _ = send_event(
                socket,
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

    // 取出或新建会话（克隆副本，避免在长 LLM 调用期间持锁）。
    // 跨用户访问既存 session_id 返回错误并关闭连接（与 REST 404 行为对齐）。
    let mut data = {
        let sessions = state.sessions.read().await;
        match sessions.get(&session_id) {
            Some(d) if d.user_id == user_id => d.clone(),
            Some(_) => {
                let _ = send_event(
                    socket,
                    &OrchestratorEvent::Error {
                        message: "session not found".into(),
                    },
                )
                .await;
                return std::ops::ControlFlow::Break(());
            }
            None => SessionData {
                session: Session::new(session_id),
                history: Arc::new(RwLock::new(History::new())),
                user_id,
            },
        }
    };
    data.session.append(CoreMessage::User(req.message.clone()));

    let (tx, mut rx) = mpsc::channel::<OrchestratorEvent>(64);
    let orch = state.orchestrator.clone();
    let user_msg = req.message;
    let join = tokio::spawn(async move {
        let res = {
            let mut history_guard = data.history.write().await;
            orch.run_streaming(&mut history_guard, user_msg, tx).await
        };
        (data, res)
    });

    // 并发转发事件、发心跳、监听 Pong/Close。
    let mut final_text = String::new();
    let mut got_complete = false;
    let heartbeat_interval = heartbeat_interval();
    let pong_timeout = pong_timeout();
    let mut heartbeat = interval_at(Instant::now() + heartbeat_interval, heartbeat_interval);
    let mut pong_deadline: Option<Instant> = None;
    let mut disconnected = false;

    loop {
        tokio::select! {
            // 心跳：定时发 Ping，若超时未收到 Pong 则断开。
            _ = heartbeat.tick() => {
                if let Some(deadline) = pong_deadline {
                    if Instant::now() >= deadline {
                        disconnected = true;
                        break;
                    }
                }
                if socket.send(Message::Ping(Vec::new().into())).await.is_err() {
                    disconnected = true;
                    break;
                }
                pong_deadline = Some(Instant::now() + pong_timeout);
            }

            // 监听客户端 Pong / Close / 其他帧。
            msg = socket.next() => {
                match msg {
                    Some(Ok(Message::Pong(_))) => {
                        pong_deadline = None;
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        disconnected = true;
                        break;
                    }
                    _ => {}
                }
            }

            // 转发 LLM 事件。
            event = rx.recv() => {
                match event {
                    Some(event) => {
                        if let OrchestratorEvent::Complete { ref text, .. } = event {
                            final_text = text.clone();
                            got_complete = true;
                        }
                        if send_event(socket, &event).await.is_err() {
                            disconnected = true;
                            break;
                        }
                        if matches!(
                            event,
                            OrchestratorEvent::Complete { .. } | OrchestratorEvent::Error { .. }
                        ) {
                            break;
                        }
                    }
                    None => break,
                }
            }
        }
    }

    // 客户端已断开时：先 drop receiver 让 tx.send 失败，再 abort 任务，避免继续烧 token。
    if disconnected {
        drop(rx);
        join.abort();
    }

    // 等待任务结束，显式处理正常完成 / panic / 超时三态。
    match tokio::time::timeout(JOIN_SHUTDOWN_TIMEOUT, join).await {
        Ok(Ok((mut final_data, Ok(_)))) => {
            if got_complete {
                final_data
                    .session
                    .append(CoreMessage::Assistant(AssistantMsg {
                        text: final_text,
                        tool_calls: Vec::new(),
                    }));
            }
            let mut sessions = state.sessions.write().await;
            sessions.insert(session_id, final_data);
        }
        Ok(Ok((_, Err(e)))) => {
            tracing::error!(?e, "run_streaming failed");
            if !disconnected {
                let _ = send_event(
                    socket,
                    &OrchestratorEvent::Error {
                        message: "internal server error".into(),
                    },
                )
                .await;
            }
        }
        Ok(Err(join_err)) => {
            if join_err.is_panic() {
                tracing::error!("llm task panicked");
                if !disconnected {
                    let _ = send_event(
                        socket,
                        &OrchestratorEvent::Error {
                            message: "internal server error".into(),
                        },
                    )
                    .await;
                }
            }
            // cancellation 属于 disconnect abort 的预期路径，无需写回。
        }
        Err(_) => {
            tracing::error!("llm task did not shut down within 120s");
            if !disconnected {
                let _ = send_event(
                    socket,
                    &OrchestratorEvent::Error {
                        message: "internal server error".into(),
                    },
                )
                .await;
            }
        }
    }

    if disconnected {
        std::ops::ControlFlow::Break(())
    } else {
        std::ops::ControlFlow::Continue(())
    }
}

async fn send_event(socket: &mut WebSocket, event: &OrchestratorEvent) -> Result<(), axum::Error> {
    let json = serde_json::to_string(event).unwrap_or_else(|_| "{}".into());
    socket.send(Message::Text(json.into())).await
}
