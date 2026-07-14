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

/// 规范化用于写入的目标路径，并确认其仍位于 `base` 目录内。
///
/// 返回的是绝对、已解析符号链接的路径，可直接用于后续写入，避免在检查
/// 与写入之间重新解析原始路径，从而关闭 TOCTOU 符号链接替换窗口。
fn canonicalize_write_path(path: &Path, base: &Path) -> Option<PathBuf> {
    let canon_base = base.canonicalize().ok()?;
    let canon_path = path.canonicalize().ok().or_else(|| {
        let parent = path.parent().filter(|p| !p.as_os_str().is_empty())?;
        let canon_parent = parent.canonicalize().ok()?;
        path.file_name().map(|name| canon_parent.join(name))
    })?;
    if canon_path.starts_with(&canon_base) {
        Some(canon_path)
    } else {
        None
    }
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
        const MAX_READ_BYTES: usize = 1024 * 1024;

        let start = std::time::Instant::now();

        #[cfg(unix)]
        let read_result: std::io::Result<Vec<u8>> = {
            use std::os::unix::io::AsRawFd;
            use tokio::fs::OpenOptions;

            let canon_working_dir = match self.working_dir.canonicalize() {
                Ok(d) => d,
                Err(e) => {
                    return Ok(ToolResult {
                        output: String::new(),
                        error: Some(format!("cannot canonicalize working dir: {}", e)),
                        duration_ms: start.elapsed().as_millis() as u64,
                    })
                }
            };

            let file = match OpenOptions::new()
                .read(true)
                .open(&path)
                .await
            {
                Ok(f) => f,
                Err(e) => {
                    return Ok(ToolResult {
                        output: String::new(),
                        error: Some(format!("read failed: {}", e)),
                        duration_ms: start.elapsed().as_millis() as u64,
                    })
                }
            };

            let std_file = file.into_std().await;
            let fd = std_file.as_raw_fd();
            let fd_path = format!("/proc/self/fd/{}", fd);
            let real_path = match std::fs::canonicalize(&fd_path) {
                Ok(p) => p,
                Err(_) => {
                    return Ok(blocked("path outside working directory (cannot resolve fd)"));
                }
            };

            if !real_path.starts_with(&canon_working_dir) {
                return Ok(blocked("path outside working directory"));
            }

            use tokio::io::AsyncReadExt;
            let tokio_file = tokio::fs::File::from_std(std_file);
            let mut bytes = Vec::new();
            match tokio_file.take(MAX_READ_BYTES as u64 + 1).read_to_end(&mut bytes).await {
                Ok(_) => Ok(bytes),
                Err(e) => Err(e),
            }
        };

        #[cfg(not(unix))]
        let read_result: std::io::Result<Vec<u8>> = {
            if !is_within(&path, &self.working_dir) {
                return Ok(blocked("path outside working directory"));
            }
            tokio::fs::read(&path).await
        };

        match read_result {
            Ok(bytes) => {
                let truncated = bytes.len() > MAX_READ_BYTES;
                let text =
                    String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_READ_BYTES)]).into_owned();
                Ok(ToolResult {
                    output: text,
                    error: if truncated {
                        Some(format!("output truncated at {} bytes", MAX_READ_BYTES))
                    } else {
                        None
                    },
                    duration_ms: start.elapsed().as_millis() as u64,
                })
            }
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
        if !is_within(&path, &self.working_dir) {
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
        let safe_path = match canonicalize_write_path(&path, &self.working_dir) {
            Some(p) => p,
            None => return Ok(blocked("path outside working directory")),
        };
        let start = std::time::Instant::now();
        match tokio::fs::write(&safe_path, content).await {
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

    #[tokio::test]
    async fn write_check_outside_is_critical() {
        let dir = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let tool = FileWriteTool::new(dir.path().to_path_buf());
        let target = outside.path().join("x.txt");
        let lvl = tool
            .check(&json!({"path": target.to_string_lossy().to_string(), "content": "y"}))
            .await;
        assert_eq!(lvl, SafetyLevel::Critical);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn write_symlink_inside_succeeds() {
        use std::os::unix::fs::symlink;
        let dir = tempdir().unwrap();
        let real = dir.path().join("real.txt");
        std::fs::write(&real, "original").unwrap();
        let link = dir.path().join("link.txt");
        symlink(&real, &link).unwrap();
        let tool = FileWriteTool::new(dir.path().to_path_buf());
        let r = tool
            .execute(json!({"path": "link.txt", "content": "updated"}))
            .await
            .unwrap();
        assert!(r.error.is_none(), "{:?}", r.error);
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "updated");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn write_symlink_replaced_to_outside_blocked() {
        use std::os::unix::fs::symlink;
        let dir = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let inside_file = dir.path().join("inside.txt");
        std::fs::write(&inside_file, "inside").unwrap();
        let outside_file = outside.path().join("secret.txt");
        std::fs::write(&outside_file, "outside").unwrap();
        let link = dir.path().join("link.txt");
        symlink(&inside_file, &link).unwrap();
        // 模拟检查通过后、写入前符号链接被替换为指向外部路径。
        std::fs::remove_file(&link).unwrap();
        symlink(&outside_file, &link).unwrap();
        let tool = FileWriteTool::new(dir.path().to_path_buf());
        let r = tool
            .execute(json!({"path": "link.txt", "content": "attacker"}))
            .await
            .unwrap();
        assert!(r.error.unwrap().contains("blocked"));
        assert_eq!(std::fs::read_to_string(&outside_file).unwrap(), "outside");
    }
}
