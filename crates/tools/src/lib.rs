//! forgeclaw-tools：工具沙箱（Task 3）：`Tool` trait + 5 个内置工具 + `Sandbox`。
//!
//! 安全模型分层：
//! - [`Tool::check`] 返回 [`SafetyLevel`]；[`Sandbox`] 据此决定拦截/确认/放行。
//! - 工作目录硬限制由 [`file::is_within`]（canonicalize + starts_with）保证。
//! - 危险命令黑名单与敏感路径写入在工具自身层面再兜底一次（即使绕过 Sandbox 也安全）。

pub mod file;
pub mod sandbox;
pub mod search;
pub mod shell;

pub use file::{is_within, FileReadTool, FileWriteTool};
pub use sandbox::{auto_confirm, Confirmer, Sandbox};
pub use search::{GrepTool, SearchTool};
pub use shell::ShellTool;

use async_trait::async_trait;
use forgeclaw_core::model::{SafetyLevel, ToolResult};
use serde_json::Value;

/// 工具统一接口。
#[async_trait]
pub trait Tool: Send + Sync {
    /// 工具名（`Sandbox` 路由用）。
    fn name(&self) -> &str;

    /// 输入 JSON Schema。
    fn schema(&self) -> Value;

    /// 执行工具。
    async fn execute(&self, input: Value) -> anyhow::Result<ToolResult>;

    /// 安全级别判定（默认 [`SafetyLevel::Allow`]）。
    /// - [`SafetyLevel::Critical`]：`Sandbox` 直接拦截，不调用 confirmer。
    /// - [`SafetyLevel::Confirm`]：`Sandbox` 调用 confirmer 询问。
    /// - [`SafetyLevel::Allow`]：直接执行。
    async fn check(&self, _input: &Value) -> SafetyLevel {
        SafetyLevel::Allow
    }
}
