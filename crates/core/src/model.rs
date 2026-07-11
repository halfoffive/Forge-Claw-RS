//! 领域模型：会话、消息、工具调用、提示词章节。
//!
//! 所有结构派生 `Serialize/Deserialize/Clone/Debug`，可被上层 crate（llm/tools/server）直接复用。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 一次会话：固定 ID + 创建时间 + append-only 消息历史。
///
/// 注意：保持 `messages` 字节稳定是 DeepSeek prefix-cache 命中的前提，
/// 上层只允许追加新消息，不得修改既有消息。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: Uuid,
    pub created_at: DateTime<Utc>,
    pub messages: Vec<Message>,
}

/// 消息种类。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Message {
    User(String),
    Assistant(AssistantMsg),
    Tool(ToolCall, ToolResult),
}

/// 助手消息：可选文本回复 + 一组工具调用。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssistantMsg {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
}

/// 一次工具调用请求。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub tool: String,
    pub input: serde_json::Value,
}

/// 工具调用结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub output: String,
    pub error: Option<String>,
    pub duration_ms: u64,
}

/// 提示词章节：来自带 frontmatter 的 Markdown 文件。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Section {
    pub id: String,
    pub title: String,
    pub level: SafetyLevel,
    pub enabled: bool,
    pub order: i32,
    pub body: String,
}

/// 安全层级。
///
/// 序列化为小写字符串：`critical` / `confirm` / `allow`。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SafetyLevel {
    Critical,
    Confirm,
    Allow,
}

impl std::fmt::Display for SafetyLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SafetyLevel::Critical => write!(f, "critical"),
            SafetyLevel::Confirm => write!(f, "confirm"),
            SafetyLevel::Allow => write!(f, "allow"),
        }
    }
}
