//! 文件名 glob 搜索（`SearchTool`）+ 内容正则搜索（`GrepTool`）。

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use forgeclaw_core::model::ToolResult;
use regex::Regex;
use serde_json::{json, Value};
use walkdir::WalkDir;

use crate::file::is_within;
use crate::Tool;

/// 把简单 glob（`*` / `?`）翻译为锚定正则：转义其它正则元字符。
fn glob_to_regex(glob: &str) -> String {
    let mut s = String::with_capacity(glob.len() + 2);
    s.push('^');
    for c in glob.chars() {
        match c {
            '*' => s.push_str(".*"),
            '?' => s.push('.'),
            c if "\\.+()[]{}|^$".contains(c) => {
                s.push('\\');
                s.push(c);
            }
            c => s.push(c),
        }
    }
    s.push('$');
    s
}

/// 按文件名 glob 在工作目录内搜索文件。
pub struct SearchTool {
    working_dir: PathBuf,
}

impl SearchTool {
    pub fn new(working_dir: PathBuf) -> Self {
        Self { working_dir }
    }
}

#[async_trait]
impl Tool for SearchTool {
    fn name(&self) -> &str {
        "search"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "文件名 glob 模式（* 与 ?）" },
                "max": { "type": "integer", "minimum": 0, "description": "最大返回数，默认 100" }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, input: Value) -> anyhow::Result<ToolResult> {
        let pattern = input
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'pattern' field"))?;
        let max = input.get("max").and_then(|v| v.as_u64()).unwrap_or(100) as usize;
        let start = std::time::Instant::now();
        let re = match Regex::new(&glob_to_regex(pattern)) {
            Ok(re) => re,
            Err(e) => {
                return Ok(ToolResult {
                    output: String::new(),
                    error: Some(format!("invalid pattern: {}", e)),
                    duration_ms: start.elapsed().as_millis() as u64,
                });
            }
        };
        let base = match self.working_dir.canonicalize() {
            Ok(b) => b,
            Err(e) => {
                return Ok(ToolResult {
                    output: String::new(),
                    error: Some(format!("working dir error: {}", e)),
                    duration_ms: start.elapsed().as_millis() as u64,
                });
            }
        };
        let mut hits: Vec<String> = Vec::new();
        for entry in WalkDir::new(&base).into_iter().filter_map(|e| e.ok()) {
            if !entry.file_type().is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy();
            if re.is_match(&name) {
                if let Ok(rel) = entry.path().strip_prefix(&base) {
                    hits.push(rel.to_string_lossy().to_string());
                }
                if hits.len() >= max {
                    break;
                }
            }
        }
        Ok(ToolResult {
            output: hits.join("\n"),
            error: None,
            duration_ms: start.elapsed().as_millis() as u64,
        })
    }
}

/// 在工作目录内对文件内容做正则搜索，返回 `文件:行号:内容`。
pub struct GrepTool {
    working_dir: PathBuf,
}

impl GrepTool {
    pub fn new(working_dir: PathBuf) -> Self {
        Self { working_dir }
    }
}

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "正则表达式" },
                "path": { "type": "string", "description": "可选子目录（须在工作目录内），默认整个工作目录" },
                "max": { "type": "integer", "minimum": 0, "description": "最大返回匹配行数，默认 100" }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, input: Value) -> anyhow::Result<ToolResult> {
        let pattern = input
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'pattern' field"))?;
        let max = input.get("max").and_then(|v| v.as_u64()).unwrap_or(100) as usize;
        let start = std::time::Instant::now();
        let re = match Regex::new(pattern) {
            Ok(re) => re,
            Err(e) => {
                return Ok(ToolResult {
                    output: String::new(),
                    error: Some(format!("invalid regex: {}", e)),
                    duration_ms: start.elapsed().as_millis() as u64,
                });
            }
        };
        let base = if let Some(p) = input.get("path").and_then(|v| v.as_str()) {
            let p_path = if Path::new(p).is_absolute() {
                PathBuf::from(p)
            } else {
                self.working_dir.join(p)
            };
            if !is_within(&p_path, &self.working_dir) {
                return Ok(ToolResult {
                    output: String::new(),
                    error: Some("blocked: path outside working directory".to_string()),
                    duration_ms: start.elapsed().as_millis() as u64,
                });
            }
            p_path
                .canonicalize()
                .unwrap_or_else(|_| self.working_dir.clone())
        } else {
            self.working_dir
                .canonicalize()
                .unwrap_or_else(|_| self.working_dir.clone())
        };
        let mut hits: Vec<String> = Vec::new();
        for entry in WalkDir::new(&base).into_iter().filter_map(|e| e.ok()) {
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            let rel = path.strip_prefix(&base).unwrap_or(path);
            let content = match std::fs::read_to_string(path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            for (i, line) in content.lines().enumerate() {
                if re.is_match(line) {
                    hits.push(format!("{}:{}:{}", rel.display(), i + 1, line));
                    if hits.len() >= max {
                        break;
                    }
                }
            }
            if hits.len() >= max {
                break;
            }
        }
        Ok(ToolResult {
            output: hits.join("\n"),
            error: None,
            duration_ms: start.elapsed().as_millis() as u64,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    #[tokio::test]
    async fn search_finds_files_by_glob() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "x").unwrap();
        std::fs::write(dir.path().join("b.md"), "x").unwrap();
        std::fs::write(dir.path().join("nested.txt"), "x").unwrap();
        let tool = SearchTool::new(dir.path().to_path_buf());
        let r = tool.execute(json!({"pattern": "*.txt"})).await.unwrap();
        let lines: Vec<&str> = r.output.lines().collect();
        assert!(lines.contains(&"a.txt"));
        assert!(lines.contains(&"nested.txt"));
        assert!(!lines.contains(&"b.md"));
    }

    #[tokio::test]
    async fn search_respects_max() {
        let dir = tempdir().unwrap();
        for i in 0..5 {
            std::fs::write(dir.path().join(format!("f{i}.txt")), "x").unwrap();
        }
        let tool = SearchTool::new(dir.path().to_path_buf());
        let r = tool
            .execute(json!({"pattern": "*.txt", "max": 2}))
            .await
            .unwrap();
        assert_eq!(r.output.lines().count(), 2);
    }

    #[tokio::test]
    async fn grep_finds_matching_lines() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "foo\nbar\nbaz foo\n").unwrap();
        let tool = GrepTool::new(dir.path().to_path_buf());
        let r = tool.execute(json!({"pattern": "foo"})).await.unwrap();
        assert!(r.output.contains("a.txt:1:foo"));
        assert!(r.output.contains("a.txt:3:baz foo"));
    }

    #[tokio::test]
    async fn grep_path_outside_blocked() {
        let dir = tempdir().unwrap();
        let outside = tempdir().unwrap();
        std::fs::write(outside.path().join("o.txt"), "secret\n").unwrap();
        let tool = GrepTool::new(dir.path().to_path_buf());
        let r = tool
            .execute(
                json!({"pattern": "secret", "path": outside.path().to_string_lossy().to_string()}),
            )
            .await
            .unwrap();
        assert!(r.error.unwrap().contains("blocked"));
    }
}
