//! forgeclaw-llm：LLM 适配器（LlmClient trait + OpenAiClient + cache-first History）。
//!
//! 详见 `client` 模块的 [`crate::OpenAiClient`] 与 [`crate::LlmClient`]；
//! cache-first 设计见 [`crate::History`]。

pub mod client;

use serde::{Deserialize, Serialize};

pub use client::{parse_sse_events, parse_sse_stream, LlmClient, OpenAiClient};

/// 默认上下文窗口限制（字节数）：100,000 字节 ≈ 25,000 tokens（中英混合平均 4 字节/token）。
/// 安全余量，防止请求过大触发 API 400 错误。
pub const DEFAULT_CONTEXT_LIMIT: usize = 100_000;

// ============ DTO ============

/// OpenAI 风格 `tool_calls` 数组中的单条工具调用。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolCallDto {
    pub id: String,
    pub function: FunctionCallDto,
}

/// 工具调用的函数描述。
///
/// `arguments` 为 JSON **字符串**（非对象），遵循 OpenAI 协议；由调用方负责序列化。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FunctionCallDto {
    pub name: String,
    pub arguments: String,
}

/// 聊天角色。序列化为小写字符串：`system` / `user` / `assistant` / `tool`。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

impl Role {
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        }
    }
}

/// 未知 role 字符串错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownRoleError(String);

impl std::fmt::Display for UnknownRoleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unknown role: {}", self.0)
    }
}

impl TryFrom<&str> for Role {
    type Error = UnknownRoleError;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s {
            "system" => Ok(Role::System),
            "user" => Ok(Role::User),
            "assistant" => Ok(Role::Assistant),
            "tool" => Ok(Role::Tool),
            _ => Err(UnknownRoleError(s.to_string())),
        }
    }
}

/// 一条聊天消息。`role` ∈ `system` / `user` / `assistant` / `tool`。
///
/// 对 `role=tool` 的消息，`tool_call_id` 必填（对应被回复的 tool_call id）；
/// 其余 role 下为 `None`，序列化时省略以保持字节稳定。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCallDto>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    /// 工具结果消息：`role=tool`，`tool_call_id` 指向被回复的工具调用。
    pub fn tool(content: impl Into<String>, tool_call_id: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
        }
    }
}

/// 工具规格（OpenAI `tools` 字段元素）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolSpec {
    #[serde(rename = "type")]
    pub typ: String,
    pub function: FunctionSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FunctionSpec {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// 一次聊天请求。序列化为 OpenAI `/chat/completions` 请求体。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolSpec>>,
}

impl ChatRequest {
    /// 从 [`History`] 构造请求：按 messages 原序序列化，不重排、不修改字段，
    /// 保证字节稳定前缀以命中 DeepSeek prefix-cache。
    pub fn from_history(
        history: &History,
        model: impl Into<String>,
        temperature: Option<f32>,
        max_tokens: Option<u32>,
        tools: Option<Vec<ToolSpec>>,
    ) -> Self {
        Self {
            model: model.into(),
            messages: history.messages().to_vec(),
            temperature,
            max_tokens,
            stream: true,
            tools,
        }
    }
}

/// LLM 流式事件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// 文本增量。
    Delta(String),
    /// 工具调用增量。首个 chunk 含 `id`+`name`，后续 chunk 仅含 `arguments` 增量。
    ToolCallDelta {
        index: u32,
        id: Option<String>,
        name: Option<String>,
        arguments: Option<String>,
    },
    /// 流结束（收到 `data: [DONE]`）。
    Done,
    /// 流内错误（不终止整流，由调用方决定是否中止）。
    Error(String),
}

// ============ cache-first append-only History ============

/// append-only 消息历史，保证字节稳定前缀以命中 DeepSeek prefix-cache。
///
/// 借鉴 Reasonix 缓存优先设计：
/// - 仅允许通过 [`History::append`] / [`History::extend`] 追加新消息；
/// - 既有消息（含 system 前缀 `messages[0]`）不可修改、不可删除、不可重排；
/// - DeepSeek prefix-cache 对命中前缀的输入 token 约按 **1/5** 计费，
///   保持前缀字节稳定可最大化命中率、显著降低长会话成本。
///
/// `messages` 字段为私有，类型层面阻止外部 mutate；调用方应 append-only。
#[derive(Debug, Clone, Default)]
pub struct History {
    messages: Vec<ChatMessage>,
}

impl History {
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
        }
    }

    /// 以 system 前缀构造初始 History。system 一旦设置不可变。
    pub fn with_system(system: impl Into<String>) -> Self {
        Self {
            messages: vec![ChatMessage::system(system)],
        }
    }

    /// 追加一条消息——修改 History 的唯一方式。
    pub fn append(&mut self, msg: ChatMessage) {
        self.messages.push(msg);
    }

    /// 按传入顺序批量追加。
    pub fn extend<I: IntoIterator<Item = ChatMessage>>(&mut self, msgs: I) {
        self.messages.extend(msgs);
    }

    pub fn len(&self) -> usize {
        self.messages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// 按原序返回消息切片，不重排、不修改。
    pub fn messages(&self) -> &[ChatMessage] {
        &self.messages
    }

    /// 估算消息总 token 数：按字节数/4 估算（中英混合平均）。
    /// content 字节数 + tool_calls 中 name 和 arguments 字节数都计入。
    pub fn estimate_tokens(&self) -> usize {
        self.estimate_bytes() / 4
    }

    /// 估算消息总字节数。
    pub fn estimate_bytes(&self) -> usize {
        self.messages.iter().map(estimate_message_bytes).sum()
    }

    /// 截断历史到 DEFAULT_CONTEXT_LIMIT 以内：
    /// - 保留 system prompt（如果 messages[0] 是 system）
    /// - 从前面移除最早的完整对话轮次（user -> assistant -> tool*）
    /// - 保持 tool 消息与对应 assistant tool_calls 配对完整
    /// - 保持剩余消息顺序不变
    /// - 总是保留最后一轮对话（当前轮次）
    pub fn truncate_to_limit(&mut self) {
        while self.estimate_bytes() > DEFAULT_CONTEXT_LIMIT {
            let has_system = !self.messages.is_empty() && self.messages[0].role == Role::System;
            let start = if has_system { 1 } else { 0 };

            if self.messages.len() <= start + 1 {
                return;
            }

            let mut end = start + 1;
            while end < self.messages.len() && self.messages[end].role != Role::User {
                end += 1;
            }

            if end == self.messages.len() {
                return;
            }

            self.messages.drain(start..end);
        }
    }
}

fn estimate_message_bytes(msg: &ChatMessage) -> usize {
    let mut bytes = msg.content.len();
    if let Some(tcs) = &msg.tool_calls {
        for tc in tcs {
            bytes += tc.id.len();
            bytes += tc.function.name.len();
            bytes += tc.function.arguments.len();
        }
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_append_only_keeps_system_stable() {
        let mut h = History::with_system("You are helpful.");
        let sys_before = h.messages()[0].clone();
        assert_eq!(h.len(), 1);

        h.append(ChatMessage::user("hi"));
        h.append(ChatMessage::assistant("hello"));

        assert_eq!(h.len(), 3);
        // system 消息不变
        assert_eq!(h.messages()[0], sys_before);
        // 顺序保留
        assert_eq!(h.messages()[1].role, Role::User);
        assert_eq!(h.messages()[2].role, Role::Assistant);
    }

    #[test]
    fn history_extend_preserves_order() {
        let mut h = History::new();
        h.extend([
            ChatMessage::system("s"),
            ChatMessage::user("u1"),
            ChatMessage::user("u2"),
        ]);
        assert_eq!(h.len(), 3);
        assert_eq!(h.messages()[0].content, "s");
        assert_eq!(h.messages()[1].content, "u1");
        assert_eq!(h.messages()[2].content, "u2");
    }

    #[test]
    fn chatrequest_serialization_is_deterministic() {
        let mut h = History::with_system("sys");
        h.append(ChatMessage::user("m1"));
        let r1 = ChatRequest::from_history(&h, "deepseek-chat", Some(0.7), None, None);
        let r2 = ChatRequest::from_history(&h, "deepseek-chat", Some(0.7), None, None);
        let j1 = serde_json::to_string(&r1).unwrap();
        let j2 = serde_json::to_string(&r2).unwrap();
        assert_eq!(j1, j2, "同一 History 两次序列化必须字节相同");
    }

    #[test]
    fn chatrequest_prefix_byte_stable_across_append() {
        // append 后，旧消息数组的序列化结果应作为新结果的字节前缀
        let mut h1 = History::with_system("sys");
        h1.append(ChatMessage::user("m1"));
        let m1_json = serde_json::to_string(h1.messages()).unwrap();

        let mut h2 = History::with_system("sys");
        h2.append(ChatMessage::user("m1"));
        h2.append(ChatMessage::assistant("r1"));
        let m2_json = serde_json::to_string(h2.messages()).unwrap();

        // m1_json = "[{...},{...}]"  m2_json = "[{...},{...},{...}]"
        // 去掉 m1_json 末尾 "]" 后，应为 m2_json 的前缀
        let prefix = &m1_json[..m1_json.len() - 1];
        assert!(
            m2_json.starts_with(prefix),
            "append 后旧前缀应字节稳定；m1={:?} m2={:?}",
            m1_json,
            m2_json
        );
    }

    #[test]
    fn chatrequest_omits_none_fields() {
        let r = ChatRequest::from_history(&History::new(), "m", None, None, None);
        let j = serde_json::to_string(&r).unwrap();
        assert!(!j.contains("temperature"));
        assert!(!j.contains("max_tokens"));
        assert!(!j.contains("tools"));
        assert!(j.contains("\"stream\":true"));
    }

    #[test]
    fn chatmessage_tool_role_serializes_tool_call_id() {
        let m = ChatMessage {
            role: Role::Tool,
            content: "42".into(),
            tool_calls: None,
            tool_call_id: Some("call_1".into()),
        };
        let j = serde_json::to_string(&m).unwrap();
        assert!(j.contains("\"tool_call_id\":\"call_1\""));
        assert!(j.contains("\"role\":\"tool\""));
        assert!(!j.contains("tool_calls"));
    }

    #[test]
    fn role_try_from_known_roles() {
        assert_eq!(Role::try_from("system"), Ok(Role::System));
        assert_eq!(Role::try_from("user"), Ok(Role::User));
        assert_eq!(Role::try_from("assistant"), Ok(Role::Assistant));
        assert_eq!(Role::try_from("tool"), Ok(Role::Tool));
    }

    #[test]
    fn role_try_from_unknown_role_does_not_fall_back_to_user() {
        let result = Role::try_from("developer");
        assert!(result.is_err());
        assert_ne!(result.ok(), Some(Role::User));
    }

    #[test]
    fn estimate_tokens_counts_content_and_tool_calls() {
        let mut h = History::new();
        h.append(ChatMessage::user("hello"));
        assert_eq!(h.estimate_tokens(), 5 / 4);

        let mut h2 = History::new();
        let mut assistant_msg = ChatMessage::assistant("");
        assistant_msg.tool_calls = Some(vec![ToolCallDto {
            id: "call_1".into(),
            function: FunctionCallDto {
                name: "test_tool".into(),
                arguments: "{\"a\":1}".into(),
            },
        }]);
        h2.append(assistant_msg);
        let expected_bytes = "call_1".len() + "test_tool".len() + "{\"a\":1}".len();
        assert_eq!(h2.estimate_tokens(), expected_bytes / 4);
    }

    #[test]
    fn truncate_no_op_when_under_limit() {
        let mut h = History::with_system("sys");
        h.append(ChatMessage::user("hello"));
        h.append(ChatMessage::assistant("hi"));
        let before = h.messages().to_vec();
        h.truncate_to_limit();
        assert_eq!(h.messages(), &before[..]);
    }

    #[test]
    fn truncate_removes_oldest_turns_keeps_system_and_latest() {
        let mut h = History::with_system("sys");
        let long_content = "x".repeat(DEFAULT_CONTEXT_LIMIT / 3);
        h.append(ChatMessage::user(long_content.clone()));
        h.append(ChatMessage::assistant(long_content.clone()));
        h.append(ChatMessage::user(long_content.clone()));
        h.append(ChatMessage::assistant("final"));

        assert!(h.estimate_bytes() > DEFAULT_CONTEXT_LIMIT);
        h.truncate_to_limit();
        assert!(h.estimate_bytes() <= DEFAULT_CONTEXT_LIMIT);

        assert_eq!(h.messages()[0].role, Role::System);
        assert_eq!(h.messages().last().unwrap().content, "final");
        assert!(h.messages().iter().any(|m| m.role == Role::User && m.content == long_content));
    }

    #[test]
    fn truncate_keeps_tool_calls_paired() {
        let mut h = History::with_system("sys");
        let long_content = "x".repeat(DEFAULT_CONTEXT_LIMIT / 2);

        h.append(ChatMessage::user(long_content.clone()));
        let mut a1 = ChatMessage::assistant("");
        a1.tool_calls = Some(vec![ToolCallDto {
            id: "call_old".into(),
            function: FunctionCallDto {
                name: "tool".into(),
                arguments: "{}".into(),
            },
        }]);
        h.append(a1);
        h.append(ChatMessage::tool("old result", "call_old"));

        h.append(ChatMessage::user(long_content.clone()));
        let mut a2 = ChatMessage::assistant("");
        a2.tool_calls = Some(vec![ToolCallDto {
            id: "call_new".into(),
            function: FunctionCallDto {
                name: "tool".into(),
                arguments: "{}".into(),
            },
        }]);
        h.append(a2);
        h.append(ChatMessage::tool("new result", "call_new"));

        assert!(h.estimate_bytes() > DEFAULT_CONTEXT_LIMIT);
        h.truncate_to_limit();

        assert_eq!(h.messages()[0].role, Role::System);
        let tool_msgs: Vec<_> = h.messages().iter().filter(|m| m.role == Role::Tool).collect();
        for tm in tool_msgs {
            let tc_id = tm.tool_call_id.as_ref().unwrap();
            assert!(
                h.messages().iter().any(|m| {
                    m.role == Role::Assistant
                        && m.tool_calls.as_ref().is_some_and(|tcs| {
                            tcs.iter().any(|tc| tc.id == *tc_id)
                        })
                }),
                "tool message {} has no matching assistant tool_call",
                tc_id
            );
        }
    }

    #[test]
    fn truncate_preserves_message_order() {
        let mut h = History::with_system("sys");
        let long_content = "x".repeat(DEFAULT_CONTEXT_LIMIT / 4);
        for i in 0..5 {
            h.append(ChatMessage::user(format!("u{}", i)));
            h.append(ChatMessage::assistant(format!("a{}", i)));
        }
        h.append(ChatMessage::user(long_content));
        h.append(ChatMessage::assistant("final"));

        h.truncate_to_limit();

        let msgs = h.messages();
        let mut i = if !msgs.is_empty() && msgs[0].role == Role::System { 1 } else { 0 };
        while i < msgs.len() {
            assert_eq!(msgs[i].role, Role::User, "expected user at position {}", i);
            i += 1;
            while i < msgs.len() && msgs[i].role != Role::User {
                assert!(
                    msgs[i].role == Role::Assistant || msgs[i].role == Role::Tool,
                    "expected assistant/tool after user at position {}, got {:?}",
                    i,
                    msgs[i].role
                );
                i += 1;
            }
        }
    }

    #[test]
    fn truncate_works_without_system_prompt() {
        let mut h = History::new();
        let long_content = "x".repeat(DEFAULT_CONTEXT_LIMIT / 3);
        h.append(ChatMessage::user(long_content.clone()));
        h.append(ChatMessage::assistant(long_content.clone()));
        h.append(ChatMessage::user("final"));
        h.append(ChatMessage::assistant("ok"));

        h.truncate_to_limit();
        assert!(h.estimate_bytes() <= DEFAULT_CONTEXT_LIMIT);
        assert!(h.messages().iter().any(|m| m.role == Role::User));
        assert_eq!(h.messages().last().unwrap().content, "ok");
    }
}
