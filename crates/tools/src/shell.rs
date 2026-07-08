//! Shell 命令工具 + 危险命令黑名单。

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;
use forgeclaw_core::model::{SafetyLevel, ToolResult};
use regex::Regex;
use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncReadExt};

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
/// 覆盖：`rm -rf <绝对路径>`（`/`|`/*`|`~`|`$HOME`|任意 `/...`）、fork bomb、
/// `git push --force|-f`（任意位置）、`bash -c`/`sh -c`/`eval`/`exec`（可绕过黑名单）、
/// `mkfs`、`dd if=/dev/zero of=/dev/...`、`chmod -R 777 /`、`> /dev/sdX`。
///
/// 不使用 `(?:^|\s)` 前瞻，改用 `\b` 词边界，既防 `$(rm -rf /)` 等命令替换绕过，
/// 又避免匹配单词中段。
fn dangerous_regex() -> &'static Regex {
    DANGEROUS.get_or_init(|| {
        Regex::new(concat!(
            r"(?:",
            // rm -rf <绝对路径>：/、/*、任意 /... 绝对路径。
            r"\brm\s+-rf\s+/\S*",
            r"|\brm\s+-rf\s+~(?:\s|$)",
            r"|\brm\s+-rf\s+\$HOME(?:\s|$|/)",
            // fork bomb：允许任意空白穿插。
            r"|:\(\)\s*\{\s*:\|:\s*&\s*\}\s*;\s*:",
            // git push --force/-f 任意位置（不只紧跟 push 之后）。
            r"|\bgit\s+push\b.*(?:--force\b|\s-f\b)",
            // 可执行任意命令的构造，会绕过黑名单，一律拦截。
            r"|\bbash\s+-c\b",
            r"|\bsh\s+-c\b",
            r"|\beval\b",
            r"|\bexec\b",
            // 其它危险命令。
            r"|\bmkfs\b",
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
            tracing::warn!(command = %command, "dangerous command detected");
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
            tracing::warn!(command = %command, "blocked dangerous command");
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
        let cap = 1024 * 1024; // 1MB 截断阈值

        // spawn 子进程，手动并发读 stdout/stderr（各限 1MB），整体 60s 超时。
        let run = async {
            let mut child = tokio::process::Command::new("sh")
                .arg("-c")
                .arg(command)
                .current_dir(&working_dir)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .spawn()?;
            let stdout = child.stdout.take().expect("piped stdout");
            let stderr = child.stderr.take().expect("piped stderr");
            let (out_bytes, err_bytes) =
                tokio::join!(read_capped(stdout, cap), read_capped(stderr, cap));
            let status = child.wait().await?;
            Ok::<(Vec<u8>, Vec<u8>, std::process::ExitStatus), std::io::Error>((
                out_bytes, err_bytes, status,
            ))
        };

        let timeout_result = tokio::time::timeout(Duration::from_secs(60), run).await;
        let duration_ms = start.elapsed().as_millis() as u64;

        let (out_bytes, err_bytes, status) = match timeout_result {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                return Ok(ToolResult {
                    output: String::new(),
                    error: Some(format!("spawn/io failed: {}", e)),
                    duration_ms,
                });
            }
            Err(_) => {
                return Ok(ToolResult {
                    output: String::new(),
                    error: Some("command timed out after 60s".to_string()),
                    duration_ms,
                });
            }
        };

        let stdout = String::from_utf8_lossy(&out_bytes).to_string();
        let stderr = String::from_utf8_lossy(&err_bytes).to_string();

        // 以 exit code 判断 error；stderr 仅作为附加上下文。
        if status.success() {
            Ok(ToolResult {
                output: stdout,
                error: None,
                duration_ms,
            })
        } else {
            let code = status.code().unwrap_or(-1);
            let err = if stderr.is_empty() {
                format!("exit code: {}", code)
            } else {
                format!("exit code: {}\n{}", code, stderr)
            };
            Ok(ToolResult {
                output: stdout,
                error: Some(err),
                duration_ms,
            })
        }
    }
}

/// 读取流至 EOF，最多保留前 `cap` 字节；超出部分继续读空以避免管道写端阻塞，
/// 但不追加到结果，防止超大输出 OOM。
async fn read_capped<R: AsyncRead + Unpin>(mut r: R, cap: usize) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    loop {
        match r.read(&mut tmp).await {
            Ok(0) => break,
            Ok(n) => {
                if buf.len() < cap {
                    let remaining = cap - buf.len();
                    let take = n.min(remaining);
                    buf.extend_from_slice(&tmp[..take]);
                }
            }
            Err(_) => break,
        }
    }
    buf
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
        // 黑名单绕过构造：bash -c / sh -c / eval / exec 可执行任意命令，一律拦截。
        assert!(is_dangerous("bash -c 'rm -rf /'"));
        assert!(is_dangerous("sh -c evil"));
        assert!(is_dangerous("eval $(cat evil)"));
        assert!(is_dangerous("exec rm -rf /"));
        // 命令替换内的危险命令也应被 \b 词边界捕获。
        assert!(is_dangerous("$(rm -rf /)"));
        // --force 在任意位置（含末尾）均拦截。
        assert!(is_dangerous("git push origin main --force"));
        assert!(!is_dangerous("ls -la"));
        assert!(!is_dangerous("cargo build"));
        // rm -rf <绝对路径>：/tmp/... 是绝对路径，应拦截。
        assert!(is_dangerous("rm -rf /tmp/build-artifacts"));
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
