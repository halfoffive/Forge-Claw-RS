//! `Sandbox`：工具注册 + 分层安全调度（Critical 拦截 / Confirm 确认 / Allow 放行）。

use std::path::{Path, PathBuf};

use forgeclaw_core::model::{SafetyLevel, ToolResult};
use serde_json::Value;

use crate::{FileReadTool, FileWriteTool, GrepTool, SearchTool, ShellTool, Tool};

/// 确认回调：`(tool_name, input) -> 是否放行`。
pub type Confirmer = Box<dyn Fn(&str, &Value) -> bool + Send + Sync>;

/// 工具沙箱：持有工作目录、工具集合与确认回调。
pub struct Sandbox {
    working_dir: PathBuf,
    tools: Vec<Box<dyn Tool>>,
    confirmer: Confirmer,
}

impl Sandbox {
    pub fn new(working_dir: PathBuf, confirmer: Confirmer) -> Self {
        Self {
            working_dir,
            tools: Vec::new(),
            confirmer,
        }
    }

    /// 注册一个工具。
    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.push(tool);
    }

    /// 已注册工具名列表。
    pub fn list(&self) -> Vec<String> {
        self.tools.iter().map(|t| t.name().to_string()).collect()
    }

    /// 工作目录。
    pub fn working_dir(&self) -> &Path {
        &self.working_dir
    }

    /// 按 `tool_name` 路由执行：先 `check` 判定级别，再决定拦截/确认/放行。
    pub async fn execute(&self, tool_name: &str, input: Value) -> anyhow::Result<ToolResult> {
        let tool = self
            .tools
            .iter()
            .find(|t| t.name() == tool_name)
            .ok_or_else(|| anyhow::anyhow!("tool not found: {}", tool_name))?;
        match tool.check(&input).await {
            SafetyLevel::Critical => Ok(ToolResult {
                output: String::new(),
                error: Some("blocked: critical safety level".to_string()),
                duration_ms: 0,
            }),
            SafetyLevel::Confirm => {
                if (self.confirmer)(tool_name, &input) {
                    tool.execute(input).await
                } else {
                    Ok(ToolResult {
                        output: String::new(),
                        error: Some("blocked: user denied".to_string()),
                        duration_ms: 0,
                    })
                }
            }
            SafetyLevel::Allow => tool.execute(input).await,
        }
    }

    /// 预置 5 工具（shell/read/write/search/grep）+ `auto_confirm` 的默认沙箱。
    pub fn default_for(working_dir: PathBuf) -> Self {
        let mut sb = Sandbox::new(working_dir.clone(), auto_confirm());
        sb.register(Box::new(ShellTool::new(working_dir.clone())));
        sb.register(Box::new(FileReadTool::new(working_dir.clone())));
        sb.register(Box::new(FileWriteTool::new(working_dir.clone())));
        sb.register(Box::new(SearchTool::new(working_dir.clone())));
        sb.register(Box::new(GrepTool::new(working_dir)));
        sb
    }
}

/// 默认确认器：总是放行（供测试与默认场景）。
pub fn auto_confirm() -> Confirmer {
    Box::new(|_name: &str, _input: &Value| true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    #[tokio::test]
    async fn default_for_registers_five_tools() {
        let dir = tempdir().unwrap();
        let sb = Sandbox::default_for(dir.path().to_path_buf());
        assert_eq!(sb.list(), vec!["shell", "read", "write", "search", "grep"]);
    }

    #[tokio::test]
    async fn routes_to_correct_tool() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("x.txt"), "content").unwrap();
        let mut sb = Sandbox::new(dir.path().to_path_buf(), auto_confirm());
        sb.register(Box::new(FileReadTool::new(dir.path().to_path_buf())));
        sb.register(Box::new(ShellTool::new(dir.path().to_path_buf())));
        let r = sb.execute("read", json!({"path": "x.txt"})).await.unwrap();
        assert_eq!(r.output, "content");
        let r2 = sb
            .execute("shell", json!({"command": "echo hi"}))
            .await
            .unwrap();
        assert!(r2.output.contains("hi"));
    }

    #[tokio::test]
    async fn critical_blocks_without_confirmer() {
        let dir = tempdir().unwrap();
        let mut sb = Sandbox::new(dir.path().to_path_buf(), auto_confirm());
        sb.register(Box::new(ShellTool::new(dir.path().to_path_buf())));
        let r = sb
            .execute("shell", json!({"command": "rm -rf /"}))
            .await
            .unwrap();
        assert!(r.error.unwrap().contains("blocked"));
    }

    #[tokio::test]
    async fn confirm_false_blocks() {
        let dir = tempdir().unwrap();
        let confirmer: Confirmer = Box::new(|_, _| false);
        let mut sb = Sandbox::new(dir.path().to_path_buf(), confirmer);
        sb.register(Box::new(FileWriteTool::new(dir.path().to_path_buf())));
        let r = sb
            .execute("write", json!({"path": "y.txt", "content": "z"}))
            .await
            .unwrap();
        assert!(r.error.unwrap().contains("blocked"));
        assert!(!dir.path().join("y.txt").exists());
    }

    #[tokio::test]
    async fn confirm_true_proceeds() {
        let dir = tempdir().unwrap();
        let mut sb = Sandbox::new(dir.path().to_path_buf(), auto_confirm());
        sb.register(Box::new(FileWriteTool::new(dir.path().to_path_buf())));
        let r = sb
            .execute("write", json!({"path": "y.txt", "content": "z"}))
            .await
            .unwrap();
        assert!(r.error.is_none(), "{:?}", r.error);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("y.txt")).unwrap(),
            "z"
        );
    }

    #[tokio::test]
    async fn unknown_tool_errors() {
        let dir = tempdir().unwrap();
        let sb = Sandbox::default_for(dir.path().to_path_buf());
        let res = sb.execute("nope", json!({})).await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn allow_tool_skips_confirmer_even_if_false() {
        // ShellTool 对 `ls` 为 Allow，即便 confirmer 总返回 false 也应执行。
        let dir = tempdir().unwrap();
        let confirmer: Confirmer = Box::new(|_, _| false);
        let mut sb = Sandbox::new(dir.path().to_path_buf(), confirmer);
        sb.register(Box::new(ShellTool::new(dir.path().to_path_buf())));
        let r = sb
            .execute("shell", json!({"command": "echo ok"}))
            .await
            .unwrap();
        assert!(r.error.is_none(), "{:?}", r.error);
        assert!(r.output.contains("ok"));
    }
}
