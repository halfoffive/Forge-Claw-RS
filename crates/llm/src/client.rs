//! LlmClient trait + OpenAiClient + SSE 解析。
//!
//! SSE 解析抽成纯函数 [`parse_sse_events`] 与流包装 [`parse_sse_stream`]，
//! 前者可独立同步测试，后者用 `futures::stream::unfold` 包装字节流。

use std::time::Duration;

use async_trait::async_trait;
use futures::stream::{BoxStream, Stream, StreamExt};
use serde::Deserialize;

use crate::{ChatRequest, Event, Role};

// ============ LlmClient trait ============

/// LLM 客户端抽象。返回流式 [`Event`]。
#[async_trait]
pub trait LlmClient: Send + Sync {
    /// 发起流式聊天。`req.stream` 应为 true；实现可强制覆盖。
    async fn chat(&self, req: ChatRequest) -> anyhow::Result<BoxStream<'static, Event>>;
}

// ============ SSE 解析 ============

#[derive(Debug, Deserialize)]
struct StreamChunk {
    #[serde(default)]
    choices: Vec<StreamChoice>,
}

#[derive(Debug, Deserialize)]
struct StreamChoice {
    #[serde(default)]
    delta: StreamDelta,
}

#[derive(Debug, Default, Deserialize)]
struct StreamDelta {
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<StreamToolCall>>,
}

#[derive(Debug, Deserialize)]
struct StreamToolCall {
    index: u32,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<StreamToolFunction>,
}

#[derive(Debug, Default, Deserialize)]
struct StreamToolFunction {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

/// 将一段完整 SSE 字节串解析为 [`Event`] 列表（纯函数，可独立测试）。
///
/// 按 `\n\n` 分块，每块取 `data: ` 前缀行拼接为 payload：
/// - `[DONE]` → [`Event::Done`]
/// - 合法 JSON chunk → 解析 `choices[*].delta`，产出 [`Event::Delta`] / [`Event::ToolCallDelta`]
/// - 非法 JSON → 跳过该块（不致命）
pub fn parse_sse_events(data: &str) -> Vec<Event> {
    // 兼容 \r\n / \r 行结束符，统一归一化为 \n 后再切分。
    let data = data.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = Vec::new();
    for block in data.split("\n\n") {
        let mut payload_parts: Vec<&str> = Vec::new();
        for line in block.lines() {
            // 先剥离 `data:` 前缀，再选择性剥离单个前导空格（兼容 `data:` 与 `data: ` 两种写法）。
            if let Some(rest) = line.strip_prefix("data:") {
                let rest = rest.strip_prefix(' ').unwrap_or(rest);
                payload_parts.push(rest);
            }
        }
        if payload_parts.is_empty() {
            continue;
        }
        let payload = payload_parts.join("\n");
        if payload == "[DONE]" {
            out.push(Event::Done);
            continue;
        }
        match serde_json::from_str::<StreamChunk>(&payload) {
            Ok(chunk) => out.extend(chunk_to_events(chunk)),
            Err(e) => tracing::warn!("invalid sse json: {e}"),
        }
    }
    out
}

fn chunk_to_events(chunk: StreamChunk) -> Vec<Event> {
    let mut out = Vec::new();
    for choice in chunk.choices {
        if let Some(role) = choice.delta.role {
            if let Err(e) = Role::try_from(role.as_str()) {
                tracing::warn!("unknown sse role: {e}");
                continue;
            }
        }
        if let Some(c) = choice.delta.content {
            if !c.is_empty() {
                out.push(Event::Delta(c));
            }
        }
        if let Some(tcs) = choice.delta.tool_calls {
            for tc in tcs {
                let (name, arguments) = match tc.function {
                    Some(f) => (f.name, f.arguments),
                    None => (None, None),
                };
                out.push(Event::ToolCallDelta {
                    index: tc.index,
                    id: tc.id,
                    name,
                    arguments,
                });
            }
        }
    }
    out
}

/// 将字节流包装为 [`Event`] 流：逐块累积、按 `\n\n` 切分、调用 [`parse_sse_events`]。
///
/// 泛型 `B: AsRef<[u8]>` 使其既能吃 reqwest 的 `Bytes` 流，也能在测试里吃 `Vec<u8>`，
/// 无需引入 `bytes` crate 依赖。
pub fn parse_sse_stream<S, B>(s: S) -> impl Stream<Item = Event> + Send + 'static
where
    S: Stream<Item = Result<B, reqwest::Error>> + Send + Unpin + 'static,
    B: AsRef<[u8]> + Send + 'static,
{
    struct State<S> {
        inner: S,
        buf: Vec<u8>,
        pending: std::collections::VecDeque<Event>,
        done: bool,
        received_done: bool,
    }

    let init = State {
        inner: s,
        buf: Vec::new(),
        pending: std::collections::VecDeque::new(),
        done: false,
        received_done: false,
    };

    futures::stream::unfold(init, |mut st| async move {
        loop {
            if let Some(ev) = st.pending.pop_front() {
                if matches!(ev, Event::Done) {
                    st.received_done = true;
                }
                return Some((ev, st));
            }
            if st.done {
                if !st.received_done {
                    st.received_done = true;
                    return Some((Event::Error("stream ended without [DONE]".into()), st));
                }
                return None;
            }
            match st.inner.next().await {
                None => {
                    st.done = true;
                    if !st.buf.is_empty() {
                        let evs = parse_sse_events(&String::from_utf8_lossy(&st.buf));
                        st.buf.clear();
                        for ev in evs {
                            if matches!(ev, Event::Done) {
                                st.received_done = true;
                            }
                            st.pending.push_back(ev);
                        }
                    }
                    continue;
                }
                Some(Err(e)) => {
                    // 流持续出错时终止整流，避免无限错误循环。
                    st.done = true;
                    return Some((Event::Error(e.to_string()), st));
                }
                Some(Ok(bytes)) => {
                    // 累积原始字节，避免分块边界上的 lossy 转换丢字节。
                    st.buf.extend_from_slice(bytes.as_ref());
                    while let Some((idx, sep_len)) = next_sse_boundary(&st.buf) {
                        let block: Vec<u8> = st.buf.drain(..idx).collect();
                        st.buf.drain(..sep_len);
                        for ev in parse_sse_events(&String::from_utf8_lossy(&block)) {
                            if matches!(ev, Event::Done) {
                                st.received_done = true;
                            }
                            st.pending.push_back(ev);
                        }
                    }
                    continue;
                }
            }
        }
    })
}

/// 在字节缓冲中查找最早的 SSE 事件边界：`\n\n` 或 `\r\n\r\n`（兼容 CRLF）。
/// 返回 `(边界起始位置, 分隔符字节长度)`。
fn next_sse_boundary(buf: &[u8]) -> Option<(usize, usize)> {
    let n2 = buf.windows(2).position(|w| w == b"\n\n");
    let r4 = buf.windows(4).position(|w| w == b"\r\n\r\n");
    match (n2, r4) {
        (Some(a), Some(b)) => Some(if a <= b { (a, 2) } else { (b, 4) }),
        (Some(a), None) => Some((a, 2)),
        (None, Some(b)) => Some((b, 4)),
        (None, None) => None,
    }
}

// ============ OpenAiClient（兼容 DeepSeek/GLM） ============

/// OpenAI 兼容协议客户端。POST `{base_url}/chat/completions`，SSE 流式。
pub struct OpenAiClient {
    base_url: String,
    api_key: String,
    http: reqwest::Client,
}

impl OpenAiClient {
    /// 构造客户端：rustls + 连接超时10s + 整体请求超时600s。`base_url` 末尾 `/` 会被裁掉。
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>) -> anyhow::Result<Self> {
        let connect_timeout = Duration::from_secs(10);
        let request_timeout = Duration::from_secs(600);
        let http = reqwest::Client::builder()
            .use_rustls_tls()
            .connect_timeout(connect_timeout)
            .pool_idle_timeout(Duration::from_secs(90))
            .timeout(request_timeout)
            .build()?;
        Ok(Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
            http,
        })
    }

    /// 带重试的 POST。仅对**连接错误**与 **5xx** 重试（最多 3 次重试，共 4 次尝试，指数退避：1s,2s,4s）；
    /// 4xx 立即返回（由调用方处理状态码）。
    async fn send_with_retry(
        &self,
        url: &str,
        body: &serde_json::Value,
    ) -> anyhow::Result<reqwest::Response> {
        let mut last_err: Option<anyhow::Error> = None;
        for attempt in 0..4u32 {
            if attempt > 0 {
                let backoff = Duration::from_millis(500u64 * 2u64.pow(attempt));
                tokio::time::sleep(backoff).await;
            }
            match self
                .http
                .post(url)
                .bearer_auth(&self.api_key)
                .json(body)
                .send()
                .await
            {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() || status.is_client_error() {
                        return Ok(resp);
                    }
                    // 5xx → 重试
                    last_err = Some(anyhow::anyhow!("HTTP {}", status));
                }
                Err(e) => {
                    // 连接错误 → 重试
                    last_err = Some(anyhow::Error::from(e));
                }
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("retry exhausted")))
    }
}

#[async_trait]
impl LlmClient for OpenAiClient {
    async fn chat(&self, req: ChatRequest) -> anyhow::Result<BoxStream<'static, Event>> {
        let url = format!("{}/chat/completions", self.base_url);
        // 序列化为 JSON value，强制 stream=true（DeepSeek/GLM OpenAI 兼容端点）
        let mut body = serde_json::to_value(&req)?;
        if let Some(obj) = body.as_object_mut() {
            obj.insert("stream".into(), serde_json::Value::Bool(true));
        }
        let response = self.send_with_retry(&url, &body).await?;
        let status = response.status();
        if !status.is_success() {
            let text = response
                .text()
                .await
                .unwrap_or_else(|_| "<failed to read error body>".to_string());
            // 截断到前 512 字节，避免超大错误响应撑爆日志/上下文。
            let truncated: &str = if text.len() > 512 {
                let mut end = 512;
                while !text.is_char_boundary(end) {
                    end -= 1;
                }
                &text[..end]
            } else {
                &text
            };
            anyhow::bail!("HTTP {}: {}", status, truncated);
        }
        let byte_stream = response.bytes_stream();
        let event_stream = parse_sse_stream(byte_stream);
        Ok(Box::pin(event_stream))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sse_events_text_deltas_and_done() {
        let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\n\
                   data: {\"choices\":[{\"delta\":{\"content\":\" world\"}}]}\n\n\
                   data: [DONE]\n\n";
        let ev = parse_sse_events(sse);
        assert_eq!(ev.len(), 3);
        assert_eq!(ev[0], Event::Delta("Hello".into()));
        assert_eq!(ev[1], Event::Delta(" world".into()));
        assert_eq!(ev[2], Event::Done);
    }

    #[test]
    fn parse_sse_events_tool_call_deltas() {
        let sse = "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"function\":{\"name\":\"get_weather\",\"arguments\":\"{\"}}]}}]}\n\n\
                   data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"loc\"}}]}}]}\n\n\
                   data: [DONE]\n\n";
        let ev = parse_sse_events(sse);
        assert_eq!(ev.len(), 3);
        match &ev[0] {
            Event::ToolCallDelta {
                index,
                id,
                name,
                arguments,
            } => {
                assert_eq!(*index, 0);
                assert_eq!(id.as_deref(), Some("call_1"));
                assert_eq!(name.as_deref(), Some("get_weather"));
                assert_eq!(arguments.as_deref(), Some("{"));
            }
            other => panic!("unexpected: {:?}", other),
        }
        match &ev[1] {
            Event::ToolCallDelta {
                index,
                id,
                name,
                arguments,
            } => {
                assert_eq!(*index, 0);
                assert!(id.is_none());
                assert!(name.is_none());
                assert_eq!(arguments.as_deref(), Some("loc"));
            }
            other => panic!("unexpected: {:?}", other),
        }
        assert_eq!(ev[2], Event::Done);
    }

    #[test]
    fn parse_sse_events_skips_invalid_json() {
        let sse = "data: not-json\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n";
        let ev = parse_sse_events(sse);
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0], Event::Delta("ok".into()));
    }

    #[test]
    fn parse_sse_events_unknown_role_does_not_become_user() {
        // 未知 role 不应被静默转换为 user，应跳过对应 choice。
        let sse = "data: {\"choices\":[{\"delta\":{\"role\":\"unknown\",\"content\":\"hi\"}}]}\n\n\
                   data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n";
        let ev = parse_sse_events(sse);
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0], Event::Delta("ok".into()));
    }

    #[test]
    fn parse_sse_events_ignores_non_data_lines() {
        // 注释行（`:`）与 event/id 行应被忽略
        let sse = ": comment\n\nevent: ping\nid: 1\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"x\"}}]}\n\n";
        let ev = parse_sse_events(sse);
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0], Event::Delta("x".into()));
    }

    #[test]
    fn parse_sse_events_handles_trailing_done_without_blank_line() {
        // [DONE] 末尾无 `\n\n`，应仍被解析
        let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\ndata: [DONE]";
        let ev = parse_sse_events(sse);
        assert_eq!(ev.len(), 2);
        assert_eq!(ev[0], Event::Delta("hi".into()));
        assert_eq!(ev[1], Event::Done);
    }

    #[test]
    fn parse_sse_events_empty_content_skipped() {
        // content 为空字符串应跳过（不产生空 Delta 噪声）
        let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"\"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"a\"}}]}\n\n";
        let ev = parse_sse_events(sse);
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0], Event::Delta("a".into()));
    }

    #[test]
    fn parse_sse_events_handles_crlf_line_endings() {
        // \r\n 行结束符与 \r\n\r\n 事件分隔符都应被正确解析。
        let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"He\"}}]}\r\n\r\ndata: {\"choices\":[{\"delta\":{\"content\":\"llo\"}}]}\r\n\r\ndata: [DONE]\r\n\r\n";
        let ev = parse_sse_events(sse);
        assert_eq!(ev.len(), 3);
        assert_eq!(ev[0], Event::Delta("He".into()));
        assert_eq!(ev[1], Event::Delta("llo".into()));
        assert_eq!(ev[2], Event::Done);
    }

    #[test]
    fn parse_sse_events_data_without_space() {
        // `data:` 后无空格也应解析。
        let sse = "data:{\"choices\":[{\"delta\":{\"content\":\"x\"}}]}\n\ndata:[DONE]\n\n";
        let ev = parse_sse_events(sse);
        assert_eq!(ev.len(), 2);
        assert_eq!(ev[0], Event::Delta("x".into()));
        assert_eq!(ev[1], Event::Done);
    }

    #[tokio::test]
    async fn parse_sse_stream_handles_crlf_boundaries() {
        let bytes: Vec<Result<Vec<u8>, reqwest::Error>> = vec![Ok(
            b"data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\r\n\r\ndata: [DONE]\r\n\r\n"
                .to_vec(),
        )];
        let src = futures::stream::iter(bytes);
        let mut s = Box::pin(parse_sse_stream(src));
        let mut got = Vec::new();
        while let Some(ev) = s.next().await {
            got.push(ev);
        }
        assert_eq!(got, vec![Event::Delta("hi".into()), Event::Done]);
    }

    #[tokio::test]
    async fn parse_sse_stream_assembles_events_across_chunks() {
        // 把 SSE 流切成任意字节块，验证 stream 仍能正确拼装
        let bytes: Vec<Result<Vec<u8>, reqwest::Error>> = vec![
            // 第一块切断在事件中间
            Ok(b"data: {\"choices\":[{\"delta\":{\"content\":\"He\"}}]}".to_vec()),
            Ok(
                b"\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"llo\"}}]}\n\ndata: [DONE]\n\n"
                    .to_vec(),
            ),
        ];
        let src = futures::stream::iter(bytes);
        // Unfold 流非 Unpin，需 pin 后才能 .next()
        let mut s = Box::pin(parse_sse_stream(src));
        let mut got = Vec::new();
        while let Some(ev) = s.next().await {
            got.push(ev);
        }
        assert_eq!(got.len(), 3);
        assert_eq!(got[0], Event::Delta("He".into()));
        assert_eq!(got[1], Event::Delta("llo".into()));
        assert_eq!(got[2], Event::Done);
    }

    #[tokio::test]
    async fn parse_sse_stream_single_done() {
        let bytes: Vec<Result<Vec<u8>, reqwest::Error>> = vec![Ok(b"data: [DONE]\n\n".to_vec())];
        let src = futures::stream::iter(bytes);
        let mut s = Box::pin(parse_sse_stream(src));
        let mut got = Vec::new();
        while let Some(ev) = s.next().await {
            got.push(ev);
        }
        assert_eq!(got, vec![Event::Done]);
    }

    #[tokio::test]
    async fn parse_sse_stream_sends_error_on_unexpected_end_without_done() {
        let bytes: Vec<Result<Vec<u8>, reqwest::Error>> = vec![Ok(
            b"data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\n".to_vec(),
        )];
        let src = futures::stream::iter(bytes);
        let mut s = Box::pin(parse_sse_stream(src));
        let mut got = Vec::new();
        while let Some(ev) = s.next().await {
            got.push(ev);
        }
        assert_eq!(got.len(), 2);
        assert_eq!(got[0], Event::Delta("Hello".into()));
        match &got[1] {
            Event::Error(msg) => assert_eq!(msg, "stream ended without [DONE]"),
            other => panic!("expected Error, got {:?}", other),
        }
    }
}
