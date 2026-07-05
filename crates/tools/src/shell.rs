//! Shell 命令工具 + 危险命令黑名单。

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use async_trait::async_trait;
use forgeclaw_core::model::{SafetyLevel, ToolResult};
use regex::Regex;
use serde_json::{json, Value};

use crate::file::is_within;
use crate::Tool;

/// Shell 命令执行工具，工作目录被硬限制在 `working_dir` 内。
pub struct ShellTool {
    working_dir: PathBuf,
}

impl ShellTool {
    pub fn new(working_dir: PathBuf) -> Self {
        Self { working_dir }
    }
}

static DANGEROUS: OnceLock<Regex> = OnceLock::new();

/// 危险命令黑名单（单一正则，命令规范化为单空格后整体匹配）。
///
/// 覆盖：`rm -rf /`|`/*`|`~`|`$HOME`、fork bomb、`git push --force|-f`、
/// `mkfs`、`dd if=/dev/zero of=/dev/...`、`chmod -R 777 /`、`> /dev/sdX`。
fn dangerous_regex() -> &'static Regex {
    DANGEROUS.get_or_init(|| {
        Regex::new(concat!(
            // 命令起始或前导空格（避免匹配单词中段）。
            r"(?:^|\s)(?:",
            r"rm\s+-rf\s+/(?:\s|$|\*)",
            r"|rm\s+-rf\s+~(?:\s|$)",
            r"|rm\s+-rf\s+\$HOME(?:\s|$|/)",
            // fork bomb：允许任意空白穿插。
            r"|:\(\)\s*\{\s*:\|:\s*&\s*\}\s*;\s*:",
            r"|git\s+push\s+(?:--force|-f)\b",
            r"|mkfs\b",
            r"|dd\s+if=/dev/zero\s+of=/dev/",
            r"|chmod\s+-R\s+777\s+/(?:\s|$)",
            r"|>\s*/dev/sd[a-z]",
            r")",
        ))
        .expect("dangerous regex is valid")
    })
}

/// 判定命令是否命中黑名单：先压缩多空格、trim，再用正则匹配。
fn is_dangerous(command: &str) -> bool {
    let normalized: String = command.split_whitespace().collect::<Vec<_>>().join(" ");
    dangerous_regex().is_match(&normalized)
}

#[async_trait]
impl Tool for ShellTool {
    fn name(&self) -> &str {
        "shell"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "shell 命令" },
                "cwd": { "type": "string", "description": "可选工作目录（须在工作目录内）" }
            },
            "required": ["command"]
        })
    }

    async fn check(&self, input: &Value) -> SafetyLevel {
        let command = input.get("command").and_then(|v| v.as_str()).unwrap_or("");
        if is_dangerous(command) {
            return SafetyLevel::Critical;
        }
        SafetyLevel::Allow
    }

    async fn execute(&self, input: Value) -> anyhow::Result<ToolResult> {
        let command = input
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'command' field"))?;

        if is_dangerous(command) {
            return Ok(ToolResult {
                output: String::new(),
                error: Some("blocked: dangerous command".to_string()),
                duration_ms: 0,
            });
        }

        let working_dir = if let Some(cwd) = input.get("cwd").and_then(|v| v.as_str()) {
            let cwd_path = if Path::new(cwd).is_absolute() {
                PathBuf::from(cwd)
            } else {
                self.working_dir.join(cwd)
            };
            if !is_within(&cwd_path, &self.working_dir) {
                return Ok(ToolResult {
                    output: String::new(),
                    error: Some("blocked: cwd outside working directory".to_string()),
                    duration_ms: 0,
                });
            }
            cwd_path
                .canonicalize()
                .unwrap_or_else(|_| self.working_dir.clone())
        } else {
            self.working_dir
                .canonicalize()
                .unwrap_or_else(|_| self.working_dir.clone())
        };

        let start = std::time::Instant::now();
        let output = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(&working_dir)
            .output()
            .await?;
        let duration_ms = start.elapsed().as_millis() as u64;
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        Ok(ToolResult {
            output: stdout,
            error: if stderr.is_empty() {
                None
            } else {
                Some(stderr)
            },
            duration_ms,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn dangerous_detection() {
        assert!(is_dangerous("rm -rf /"));
        assert!(is_dangerous("  rm   -rf   /  "));
        assert!(is_dangerous("rm -rf /*"));
        assert!(is_dangerous("rm -rf ~"));
        assert!(is_dangerous("rm -rf $HOME"));
        assert!(is_dangerous("sudo rm -rf /"));
        assert!(is_dangerous(":(){:|:&};:"));
        assert!(is_dangerous(":(){ :|: & };:"));
        assert!(is_dangerous("git push --force origin main"));
        assert!(is_dangerous("git push -f"));
        assert!(is_dangerous("mkfs.ext4 /dev/sda1"));
        assert!(is_dangerous("dd if=/dev/zero of=/dev/sda bs=1M"));
        assert!(is_dangerous("chmod -R 777 /"));
        assert!(is_dangerous("echo x > /dev/sda"));
        assert!(!is_dangerous("ls -la"));
        assert!(!is_dangerous("cargo build"));
        assert!(!is_dangerous("rm -rf /tmp/build-artifacts"));
        assert!(!is_dangerous("rm -rf ./node_modules"));
        assert!(!is_dangerous("git push -u origin main"));
        assert!(!is_dangerous("echo hello > /dev/null"));
    }

    #[tokio::test]
    async fn blocks_dangerous_rm_rf_root() {
        let dir = tempdir().unwrap();
        let tool = ShellTool::new(dir.path().to_path_buf());
        let r = tool.execute(json!({"command": "rm -rf /"})).await.unwrap();
        assert!(r.error.unwrap().contains("blocked"));
        assert!(r.output.is_empty());
    }

    #[tokio::test]
    async fn blocks_dangerous_fork_bomb() {
        let dir = tempdir().unwrap();
        let tool = ShellTool::new(dir.path().to_path_buf());
        for cmd in [":(){:|:&};:", ":(){ :|: & };:"] {
            let r = tool.execute(json!({"command": cmd})).await.unwrap();
            assert!(
                r.error.as_deref().unwrap().contains("blocked"),
                "cmd={}",
                cmd
            );
        }
    }

    #[tokio::test]
    async fn blocks_git_push_force() {
        let dir = tempdir().unwrap();
        let tool = ShellTool::new(dir.path().to_path_buf());
        let r = tool
            .execute(json!({"command": "git push --force origin main"}))
            .await
            .unwrap();
        assert!(r.error.unwrap().contains("blocked"));
    }

    #[tokio::test]
    async fn runs_ls_in_working_dir() {
        let dir = tempdir().unwrap();
        let tool = ShellTool::new(dir.path().to_path_buf());
        let r = tool.execute(json!({"command": "ls -la"})).await.unwrap();
        assert!(r.error.is_none(), "stderr: {:?}", r.error);
        assert!(r.output.contains("total") || r.output.contains('.'));
    }

    #[tokio::test]
    async fn blocks_cwd_outside_working_dir() {
        let dir = tempdir().unwrap();
        let tool = ShellTool::new(dir.path().to_path_buf());
        let r = tool
            .execute(json!({"command": "pwd", "cwd": "/tmp"}))
            .await
            .unwrap();
        assert!(r.error.unwrap().contains("blocked"));
    }

    #[tokio::test]
    async fn check_dangerous_is_critical() {
        let dir = tempdir().unwrap();
        let tool = ShellTool::new(dir.path().to_path_buf());
        let lvl = tool.check(&json!({"command": "rm -rf /"})).await;
        assert_eq!(lvl, SafetyLevel::Critical);
    }

    #[tokio::test]
    async fn check_safe_is_allow() {
        let dir = tempdir().unwrap();
        let tool = ShellTool::new(dir.path().to_path_buf());
        let lvl = tool.check(&json!({"command": "ls -la"})).await;
        assert_eq!(lvl, SafetyLevel::Allow);
    }
}
