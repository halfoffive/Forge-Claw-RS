//! WebSocket `/ws/chat`：接收 `{message, session_id}` 文本帧，调 orchestrator.run_streaming，
//! 把 [`OrchestratorEvent`] 序列化为 JSON 文本帧发回，直到 Complete/Error。
//!
//! 流式桥接：run_streaming 不内部 spawn，故在此 spawn 一个任务驱动 LLM 循环 + 推送事件，
//! 主循环并发排空 receiver 并转发给 WS 客户端。Complete 后把更新后的 history 回写会话存储。
//!
//! 鉴权：浏览器 WS 无法设 Authorization header，故从 `?token=<token>` query 参数取 token，
//! 经 `state.user_store.find_by_token` 校验；无效则返回 401 不升级。有效会话绑定 user_id。

use std::collections::HashMap;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{FromRequestParts, Query, Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use futures::StreamExt;
use serde::Deserialize;
use tokio::sync::mpsc;
use uuid::Uuid;

use chrono::Utc;
use forgeclaw_core::model::{AssistantMsg, Message as CoreMessage, Session};
use forgeclaw_llm::History;

use crate::api::{AppState, SessionData};
use crate::orchestrator::OrchestratorEvent;

#[derive(Debug, Deserialize)]
struct WsChatRequest {
    message: String,
    session_id: Option<String>,
}

/// `/ws/chat` 升级处理器。从 `?token=` query 参数鉴权。
///
/// 浏览器 WS 无法设 Authorization header，故 token 走 query 参数。
/// token 校验在 WS 升级前完成：无效 → 401 不升级。校验通过后手动调
/// `WebSocketUpgrade::from_request_parts` 完成升级握手。
pub async fn ws_chat_handler(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
    req: Request,
) -> Response {
    let user = match params
        .get("token")
        .and_then(|t| state.user_store.find_by_token(t))
    {
        Some(u) => u,
        None => return (StatusCode::UNAUTHORIZED, "unauthorized").into_response(),
    };
    let (mut parts, _body) = req.into_parts();
    match WebSocketUpgrade::from_request_parts(&mut parts, &state).await {
        Ok(ws) => ws.on_upgrade(move |socket| handle_ws(socket, state, user.id)),
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
    // 跨用户访问既存 session_id 视为新建（不泄漏、不混用）。
    let mut data = {
        let sessions = state.sessions.read().await;
        match sessions.get(&session_id) {
            Some(d) if d.user_id == user_id => d.clone(),
            _ => SessionData {
                session: Session {
                    id: session_id,
                    created_at: Utc::now(),
                    messages: Vec::new(),
                },
                history: History::new(),
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
    let join = tokio::spawn(async move {
        let res = orch.run_streaming(&mut data.history, user_msg, tx).await;
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
        if send_event(socket, &event).await.is_err() {
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

    // 回写会话
    if let Ok((mut final_data, _res)) = join.await {
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

    std::ops::ControlFlow::Continue(())
}

async fn send_event(socket: &mut WebSocket, event: &OrchestratorEvent) -> Result<(), axum::Error> {
    let json = serde_json::to_string(event).unwrap_or_else(|_| "{}".into());
    socket.send(Message::Text(json.into())).await
}
