//! 文件读写工具 + 路径安全辅助函数。

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use forgeclaw_core::model::{SafetyLevel, ToolResult};
use serde_json::{json, Value};

use crate::Tool;

/// 判断 `path` 是否位于 `base` 目录内（含 `base` 自身）。
///
/// 通过 `std::fs::canonicalize` 解析符号链接与 `..`，因此对逃逸场景安全。
/// 若 `path` 尚不存在（如写入目标），则规范化其父目录后拼接文件名再判定。
pub fn is_within(path: &Path, base: &Path) -> bool {
    let canon_base = match base.canonicalize() {
        Ok(b) => b,
        Err(_) => return false,
    };
    let canon_path = match path.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            let parent = match path.parent() {
                Some(p) if !p.as_os_str().is_empty() => p,
                _ => return false,
            };
            let canon_parent = match parent.canonicalize() {
                Ok(p) => p,
                Err(_) => return false,
            };
            match path.file_name() {
                Some(name) => canon_parent.join(name),
                None => return false,
            }
        }
    };
    canon_path.starts_with(&canon_base)
}

/// 判断路径是否落在敏感目录（即使在 working_dir 内也拒绝写入）。
fn is_sensitive_path(path: &Path) -> bool {
    let s = path.to_string_lossy();
    let system_prefixes = [
        "/etc", "/usr", "/bin", "/sbin", "/lib", "/boot", "/dev", "/proc", "/sys", "/var/log",
    ];
    for prefix in system_prefixes {
        if s == prefix || s.starts_with(&format!("{}/", prefix)) {
            return true;
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            let home_sensitive = ["/.ssh", "/.aws", "/.config", "/.gnupg", "/.netrc"];
            for suffix in home_sensitive {
                let full = format!("{}{}", home, suffix);
                if s == full || s.starts_with(&format!("{}/", full)) {
                    return true;
                }
            }
        }
    }
    false
}

/// 把 `~/...` 展开为 `$HOME/...`。
///
/// - 非 `~` 开头路径：返回 `Some(原样)`。
/// - `~` / `~/...` 且 `HOME` 已设置：返回 `Some(展开后)`。
/// - `~` / `~/...` 但 `HOME` 未设置：返回 `None`（不再返回字面量 `~`，避免下游误用）。
fn expand_tilde(p: &str) -> Option<String> {
    if p == "~" {
        return std::env::var("HOME").ok();
    }
    if let Some(rest) = p.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return Some(format!("{}/{}", home, rest));
        }
        return None;
    }
    Some(p.to_string())
}

/// 把输入路径解析为绝对路径：先展开 `~`，相对路径则拼到 `base` 下。
///
/// 若 `~` 无法展开（HOME 未设置），退化为原始输入（作为相对路径处理），
/// 由后续 [`is_within`] canonicalize 检查保证不越界。
fn resolve_within_base(input_path: &str, base: &Path) -> PathBuf {
    let expanded = expand_tilde(input_path).unwrap_or_else(|| input_path.to_string());
    let p = Path::new(&expanded);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        base.join(p)
    }
}

fn blocked(reason: &str) -> ToolResult {
    ToolResult {
        output: String::new(),
        error: Some(format!("blocked: {}", reason)),
        duration_ms: 0,
    }
}

/// 读取工作目录内文件内容。
pub struct FileReadTool {
    working_dir: PathBuf,
}

impl FileReadTool {
    pub fn new(working_dir: PathBuf) -> Self {
        Self { working_dir }
    }

    fn resolve(&self, input_path: &str) -> PathBuf {
        resolve_within_base(input_path, &self.working_dir)
    }
}

#[async_trait]
impl Tool for FileReadTool {
    fn name(&self) -> &str {
        "read"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "要读取的文件路径（相对工作目录或绝对路径）" }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, input: Value) -> anyhow::Result<ToolResult> {
        let path_str = input
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'path' field"))?;
        let path = self.resolve(path_str);
        if !is_within(&path, &self.working_dir) {
            return Ok(blocked("path outside working directory"));
        }
        let start = std::time::Instant::now();
        match tokio::fs::read_to_string(&path).await {
            Ok(content) => Ok(ToolResult {
                output: content,
                error: None,
                duration_ms: start.elapsed().as_millis() as u64,
            }),
            Err(e) => Ok(ToolResult {
                output: String::new(),
                error: Some(format!("read failed: {}", e)),
                duration_ms: start.elapsed().as_millis() as u64,
            }),
        }
    }
}

/// 写入工作目录内文件。敏感路径与越界路径一律拒绝。
pub struct FileWriteTool {
    working_dir: PathBuf,
}

impl FileWriteTool {
    pub fn new(working_dir: PathBuf) -> Self {
        Self { working_dir }
    }

    fn resolve(&self, input_path: &str) -> PathBuf {
        resolve_within_base(input_path, &self.working_dir)
    }
}

#[async_trait]
impl Tool for FileWriteTool {
    fn name(&self) -> &str {
        "write"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "要写入的文件路径（相对工作目录或绝对路径）" },
                "content": { "type": "string", "description": "写入内容" }
            },
            "required": ["path", "content"]
        })
    }

    async fn check(&self, input: &Value) -> SafetyLevel {
        let path_str = input.get("path").and_then(|v| v.as_str()).unwrap_or("");
        let path = self.resolve(path_str);
        if is_sensitive_path(&path) {
            return SafetyLevel::Critical;
        }
        SafetyLevel::Confirm
    }

    async fn execute(&self, input: Value) -> anyhow::Result<ToolResult> {
        let path_str = input
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'path' field"))?;
        let content = input
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'content' field"))?;
        let path = self.resolve(path_str);
        if is_sensitive_path(&path) {
            return Ok(blocked("sensitive path"));
        }
        if !is_within(&path, &self.working_dir) {
            return Ok(blocked("path outside working directory"));
        }
        let start = std::time::Instant::now();
        match tokio::fs::write(&path, content).await {
            Ok(()) => Ok(ToolResult {
                output: format!("wrote {} bytes to {}", content.len(), path.display()),
                error: None,
                duration_ms: start.elapsed().as_millis() as u64,
            }),
            Err(e) => Ok(ToolResult {
                output: String::new(),
                error: Some(format!("write failed: {}", e)),
                duration_ms: start.elapsed().as_millis() as u64,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn is_within_existing_inside() {
        let base = tempdir().unwrap();
        let inside = base.path().join("child.txt");
        std::fs::write(&inside, "x").unwrap();
        assert!(is_within(&inside, base.path()));
    }

    #[test]
    fn is_within_nonexistent_uses_parent() {
        let base = tempdir().unwrap();
        let new_file = base.path().join("brand_new.txt");
        assert!(is_within(&new_file, base.path()));
    }

    #[test]
    fn is_within_blocks_dotdot_escape() {
        let base = tempdir().unwrap();
        let base_canon = base.path().canonicalize().unwrap();
        let parent_file = base_canon.parent().unwrap().join("fc_outside_secret.txt");
        std::fs::write(&parent_file, "x").unwrap();
        let escape = base.path().join("../fc_outside_secret.txt");
        assert!(!is_within(&escape, base.path()));
        let _ = std::fs::remove_file(&parent_file);
    }

    #[cfg(unix)]
    #[test]
    fn is_within_blocks_symlink_escape() {
        use std::os::unix::fs::symlink;
        let base = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let outside_file = outside.path().join("real.txt");
        std::fs::write(&outside_file, "x").unwrap();
        let link = base.path().join("link.txt");
        symlink(&outside_file, &link).unwrap();
        assert!(!is_within(&link, base.path()));
    }

    #[tokio::test]
    async fn read_file_in_working_dir() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello").unwrap();
        let tool = FileReadTool::new(dir.path().to_path_buf());
        let r = tool.execute(json!({"path": "a.txt"})).await.unwrap();
        assert_eq!(r.output, "hello");
        assert!(r.error.is_none());
    }

    #[tokio::test]
    async fn read_file_outside_blocked() {
        let dir = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let f = outside.path().join("secret.txt");
        std::fs::write(&f, "x").unwrap();
        let tool = FileReadTool::new(dir.path().to_path_buf());
        let r = tool
            .execute(json!({"path": f.to_string_lossy().to_string()}))
            .await
            .unwrap();
        assert!(r.error.unwrap().contains("blocked"));
    }

    #[tokio::test]
    async fn read_missing_file_returns_error() {
        let dir = tempdir().unwrap();
        let tool = FileReadTool::new(dir.path().to_path_buf());
        let r = tool.execute(json!({"path": "nope.txt"})).await.unwrap();
        assert!(r.error.is_some());
    }

    #[tokio::test]
    async fn write_file_in_working_dir_succeeds() {
        let dir = tempdir().unwrap();
        let tool = FileWriteTool::new(dir.path().to_path_buf());
        let r = tool
            .execute(json!({"path": "out.txt", "content": "hi"}))
            .await
            .unwrap();
        assert!(r.error.is_none(), "{:?}", r.error);
        let written = std::fs::read_to_string(dir.path().join("out.txt")).unwrap();
        assert_eq!(written, "hi");
    }

    #[tokio::test]
    async fn write_to_etc_blocked() {
        let dir = tempdir().unwrap();
        let tool = FileWriteTool::new(dir.path().to_path_buf());
        let r = tool
            .execute(json!({"path": "/etc/forgeclaw_test_xyz", "content": "x"}))
            .await
            .unwrap();
        assert!(r.error.unwrap().contains("blocked"));
    }

    #[tokio::test]
    async fn write_outside_working_dir_blocked() {
        let dir = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let tool = FileWriteTool::new(dir.path().to_path_buf());
        let target = outside.path().join("stolen.txt");
        let r = tool
            .execute(json!({"path": target.to_string_lossy().to_string(), "content": "x"}))
            .await
            .unwrap();
        assert!(r.error.unwrap().contains("blocked"));
        assert!(!target.exists());
    }

    #[tokio::test]
    async fn write_check_sensitive_is_critical() {
        let dir = tempdir().unwrap();
        let tool = FileWriteTool::new(dir.path().to_path_buf());
        let lvl = tool.check(&json!({"path": "/etc/x", "content": "y"})).await;
        assert_eq!(lvl, SafetyLevel::Critical);
    }

    #[tokio::test]
    async fn write_check_normal_is_confirm() {
        let dir = tempdir().unwrap();
        let tool = FileWriteTool::new(dir.path().to_path_buf());
        let lvl = tool.check(&json!({"path": "ok.txt", "content": "y"})).await;
        assert_eq!(lvl, SafetyLevel::Confirm);
    }
}
